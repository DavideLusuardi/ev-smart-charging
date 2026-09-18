//! The control loop: plans the site on a timer and after events, and sends each EVSE its
//! new charging profile when it changed enough.
//!
//! Planning and choosing what to send are the same pure code the offline simulator runs
//! ([`Planner`] and [`dispatch`]); this module only gathers the input and talks to the
//! stations. Profiles go out in two phases so the grid limit holds while stations accept
//! them in any order: first the pure decreases, then, once every one is accepted, the rest.
//!
//! The CSMS keeps, per EVSE, the profile it believes the station follows (its *effective*
//! profile), and stays on the safe side when it can't be sure:
//!
//! - a rejected profile leaves the old one in force;
//! - after a timeout or a lost connection the station may or may not follow the new
//!   profile, so the higher of the two is assumed until a later profile is accepted;
//! - sessions on a disconnected station can't be steered, so their highest future limit is
//!   kept free for them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::join_all;
use ocpp::messages::SetChargingProfile;
use ocpp::messages::set_charging_profile::ChargingProfileStatus;
use optimizer::{
    EvseRef, PlanRequest, Planner, ProfileUpdate, Scheduler, StepProfile, dispatch,
    scheduler_by_name,
};
use tokio::task::JoinHandle;
use tracing::{Instrument, debug, error, info, info_span, warn};

use crate::handle::CallError;
use crate::profiles::{self, MAX_PERIODS};
use crate::station_id::StationId;
use crate::{AppState, SiteConfig};

/// The longest planning horizon. A profile has at most one period per slot plus one, and
/// merging it with the previous profile in the two-phase push adds a couple more, which must
/// stay within [`MAX_PERIODS`].
pub const MAX_HORIZON_SLOTS: usize = MAX_PERIODS - 24;

/// Settings the control loop can't run with.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ControlError {
    #[error("unknown scheduler `{0}`; known: {known}", known = optimizer::SCHEDULERS.join(", "))]
    UnknownScheduler(String),
    #[error("max_horizon_slots is {0}, above the limit of {MAX_HORIZON_SLOTS}")]
    HorizonTooLong(usize),
    #[error("slot_secs must be positive")]
    ZeroSlot,
}

/// Starts the control loop on its own task.
pub fn spawn_control_loop(state: Arc<AppState>) -> Result<JoinHandle<()>, ControlError> {
    let controller = Controller::new(state)?;
    Ok(tokio::spawn(
        controller.run().instrument(info_span!("control")),
    ))
}

struct Controller {
    state: Arc<AppState>,
    planner: Planner<Box<dyn Scheduler + Send + Sync>>,
}

/// What became of one SetChargingProfile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Accepted,
    /// Not applied: rejected, or never sent.
    Refused,
    /// The station may or may not follow it.
    Unknown,
}

impl Controller {
    fn new(state: Arc<AppState>) -> Result<Self, ControlError> {
        let settings = &state.config.control;
        let scheduler = scheduler_by_name(&settings.scheduler)
            .ok_or_else(|| ControlError::UnknownScheduler(settings.scheduler.clone()))?;
        if settings.slot_secs == 0 {
            return Err(ControlError::ZeroSlot);
        }
        if settings.max_horizon_slots > MAX_HORIZON_SLOTS {
            return Err(ControlError::HorizonTooLong(settings.max_horizon_slots));
        }
        let planner = Planner::new(scheduler, settings.max_horizon_slots);
        Ok(Self { state, planner })
    }

    fn site_config(&self) -> &SiteConfig {
        &self.state.config.site
    }

    /// Plans every `tick`, and `debounce` after an event.
    async fn run(self) {
        let settings = &self.state.config.control;
        info!(
            scheduler = settings.scheduler,
            tick = ?settings.tick,
            "control loop started"
        );
        loop {
            tokio::select! {
                () = tokio::time::sleep(settings.tick) => {}
                () = self.state.replan.notified() => {
                    tokio::time::sleep(settings.debounce).await;
                    // This plan covers the events that came in while waiting.
                    let _ = self.state.replan.notified().now_or_never();
                }
            }
            self.tick().await;
        }
    }

    /// One control step: plan, choose the updates, send them.
    async fn tick(&self) {
        let now = self.state.clock.now_secs();
        let (request, effective) = self.request(now);
        // The solve runs on this task. It takes microseconds for the heuristics and
        // milliseconds for the LP at the sizes a site has; a much bigger site would move it
        // to a blocking thread.
        let plan = self.planner.plan(&request);
        if let Err(violation) = plan.schedule.validate(&plan.site, &request.sessions) {
            error!(%violation, "the scheduler produced an invalid schedule; nothing sent");
            return;
        }
        let updates = dispatch(
            now,
            request.planning_capacity_w(),
            &plan.limits,
            &effective,
            self.site_config().fallback_limit_w,
            &self.state.config.control.dispatch,
        );
        self.state.site().latest_plan = Some(plan);
        if updates.is_empty() {
            debug!(sessions = request.sessions.len(), "plan unchanged");
            return;
        }
        debug!(
            decreases = updates.phase1.len(),
            increases = updates.phase2.len(),
            "sending profiles"
        );
        if !self.push(now, &updates.phase1).await {
            warn!("not every decrease was accepted; increases wait for the next plan");
            return;
        }
        self.push(now, &updates.phase2).await;
    }

    /// The planner input at `now`, and the effective profiles of the sessions it plans.
    fn request(&self, now: i64) -> (PlanRequest, BTreeMap<EvseRef, StepProfile>) {
        let site_config = self.site_config();
        let settings = &self.state.config.control;
        let site = self.state.site();
        let mut sessions = Vec::new();
        let mut effective = BTreeMap::new();
        let mut reserved_w = 0.0;
        for session in site.sessions() {
            let profile = site.effective_profiles().get(&session.evse);
            if self.is_connected(&session.evse) {
                sessions.push(session.to_state(site_config.evse_max_w).request(
                    now,
                    settings.slot_secs,
                    &site_config.needs_defaults,
                ));
                if let Some(p) = profile {
                    effective.insert(session.evse.clone(), p.clone());
                }
            } else {
                reserved_w += profile.map_or(site_config.fallback_limit_w, |p| p.peak_from(now));
            }
        }
        let busy: BTreeSet<&EvseRef> = site.sessions().map(|s| &s.evse).collect();
        let idle_evses = site.evses().iter().filter(|e| !busy.contains(e)).count();
        let request = PlanRequest {
            now_secs: now,
            slot_secs: settings.slot_secs,
            capacity_w: (site.grid_limit_w - reserved_w).max(0.0),
            idle_evses,
            fallback_limit_w: site_config.fallback_limit_w,
            sessions,
        };
        (request, effective)
    }

    fn is_connected(&self, evse: &EvseRef) -> bool {
        StationId::try_from(evse.station_id.as_str())
            .is_ok_and(|id| self.state.registry.is_connected(&id))
    }

    /// Sends the updates concurrently (each station still gets one call at a time). Returns
    /// whether every EVSE now follows its update.
    async fn push(&self, now: i64, updates: &[ProfileUpdate]) -> bool {
        let applied = join_all(updates.iter().map(|u| self.send(now, u))).await;
        applied.into_iter().all(|ok| ok)
    }

    /// Sends one update and records what the EVSE follows now. Returns whether it follows
    /// the update, or no longer matters because its session ended.
    async fn send(&self, now: i64, update: &ProfileUpdate) -> bool {
        let outcome = self.call(update).await;
        let mut site = self.state.site();
        let tx = update.session.0.as_str();
        if site.session(tx).is_none_or(|s| s.evse != update.evse) {
            // The transaction ended meanwhile, and its TxProfile with it.
            return true;
        }
        match outcome {
            Outcome::Accepted => {
                site.set_effective(update.evse.clone(), update.profile.clone());
                true
            }
            Outcome::Refused => false,
            Outcome::Unknown => {
                let fallback = self.site_config().fallback_limit_w;
                let old = site
                    .effective_profiles()
                    .get(&update.evse)
                    .cloned()
                    .unwrap_or_else(|| StepProfile::constant(now, fallback));
                site.set_effective(update.evse.clone(), old.pointwise_max(&update.profile, now));
                false
            }
        }
    }

    async fn call(&self, update: &ProfileUpdate) -> Outcome {
        let evse = &update.evse;
        let request = match profiles::tx_profile(&update.session.0, evse.evse_id, &update.profile) {
            Ok(request) => request,
            Err(e) => {
                error!(%evse, error = %e, "cannot encode the profile");
                return Outcome::Refused;
            }
        };
        let handle = StationId::try_from(evse.station_id.as_str())
            .ok()
            .and_then(|id| self.state.registry.get(&id));
        let Some(handle) = handle else {
            debug!(%evse, "station not connected, profile not sent");
            return Outcome::Refused;
        };
        match handle.call::<SetChargingProfile>(&request).await {
            Ok(response) if response.status == ChargingProfileStatus::Accepted => {
                debug!(%evse, limit_w = update.profile.first_limit_w(), "profile accepted");
                Outcome::Accepted
            }
            Ok(response) => {
                warn!(%evse, info = ?response.status_info, "profile rejected");
                Outcome::Refused
            }
            Err(e @ CallError::Rejected { .. }) => {
                warn!(%evse, error = %e, "profile refused");
                Outcome::Refused
            }
            Err(e) => {
                warn!(%evse, error = %e, "profile outcome unknown");
                Outcome::Unknown
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::StationHandle;
    use crate::state::{EvseState, Session};
    use crate::{Config, ControlSettings};
    use ocpp::DateTime;

    fn evse(station: &str, evse_id: i32) -> EvseRef {
        EvseRef {
            station_id: station.into(),
            evse_id,
        }
    }

    fn controller() -> Controller {
        Controller::new(AppState::new(Config::default())).unwrap()
    }

    /// CS1 (connected) runs a session on EVSE 1 and has EVSE 2 idle; CS2 (gone) runs one
    /// session under a 7 kW TxProfile and one under its fallback, and has EVSE 3 idle.
    #[tokio::test]
    async fn sessions_on_disconnected_stations_are_reserved() {
        let c = controller();
        let cs1 = StationId::try_from("CS1").unwrap();
        let _registration = c.state.registry.register(
            cs1.clone(),
            StationHandle::new(tokio::sync::mpsc::channel(1).0),
        );
        let now = c.state.clock.now_secs();
        {
            let mut site = c.state.site();
            let t = DateTime::now();
            site.touch(&cs1, t).evses.insert(2, EvseState::default());
            site.touch(&StationId::try_from("CS2").unwrap(), t)
                .evses
                .insert(3, EvseState::default());
            site.start_session(Session::new("a", evse("CS1", 1), t));
            site.start_session(Session::new("b", evse("CS2", 1), t));
            site.start_session(Session::new("c", evse("CS2", 2), t));
            site.set_effective(evse("CS1", 1), StepProfile::constant(now, 11_000.0));
            site.set_effective(
                evse("CS2", 1),
                StepProfile::from_steps(now - 60, [(0, 22_000.0), (60, 7_000.0)]),
            );
        }

        let (request, effective) = c.request(now);
        let ids: Vec<_> = request.sessions.iter().map(|s| s.id.0.as_str()).collect();
        assert_eq!(ids, ["a"]);
        assert_eq!(effective.keys().collect::<Vec<_>>(), [&evse("CS1", 1)]);
        assert_eq!(request.idle_evses, 2);
        // The 22 kW step is over; 7 kW for b, the 1.4 kW fallback for c.
        assert_eq!(request.capacity_w, 100_000.0 - 7_000.0 - 1_400.0);
    }

    #[test]
    fn bad_settings_are_refused() {
        let with = |control: ControlSettings| {
            Controller::new(AppState::new(Config {
                control,
                ..Config::default()
            }))
            .err()
        };
        assert_eq!(
            with(ControlSettings {
                scheduler: "magic".into(),
                ..ControlSettings::default()
            }),
            Some(ControlError::UnknownScheduler("magic".into()))
        );
        assert_eq!(
            with(ControlSettings {
                max_horizon_slots: 5_000,
                ..ControlSettings::default()
            }),
            Some(ControlError::HorizonTooLong(5_000))
        );
        assert_eq!(
            with(ControlSettings {
                slot_secs: 0,
                ..ControlSettings::default()
            }),
            Some(ControlError::ZeroSlot)
        );
    }
}
