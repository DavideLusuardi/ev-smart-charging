//! Receding-horizon planning: turns the current site state into per-EVSE power profiles.
//!
//! The CSMS and the offline simulator both call [`Planner::plan`] on every control tick, so
//! the benchmark exercises the same logic that runs live.

use crate::profile::StepProfile;
use crate::types::{EvseRef, Schedule, Scheduler, SessionId, SessionRequest, SiteInput, Slot};

/// The site state at planning time.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRequest {
    pub now_secs: i64,
    pub slot_secs: u32,
    /// The site's grid limit.
    pub capacity_w: f64,
    /// EVSEs with no session. Each may start drawing `fallback_limit_w` (its
    /// TxDefaultProfile) as soon as an EV plugs in, so that much is kept free for it.
    pub idle_evses: usize,
    pub fallback_limit_w: f64,
    pub sessions: Vec<SessionRequest>,
}

impl PlanRequest {
    /// Capacity left for the sessions after reserving the idle EVSEs' fallback limits.
    pub fn planning_capacity_w(&self) -> f64 {
        (self.capacity_w - self.idle_evses as f64 * self.fallback_limit_w).max(0.0)
    }

    /// The slot containing `now_secs`.
    pub fn now_slot(&self) -> Slot {
        self.now_secs.div_euclid(i64::from(self.slot_secs))
    }
}

/// The power profile planned for one EVSE.
#[derive(Debug, Clone, PartialEq)]
pub struct EvseLimit {
    pub evse: EvseRef,
    pub session: SessionId,
    /// Starts at the planning time; periods change at slot boundaries.
    pub profile: StepProfile,
}

impl EvseLimit {
    /// The limit for the rest of the current slot.
    pub fn first_limit_w(&self) -> f64 {
        self.profile.first_limit_w()
    }
}

/// The result of one planning run.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The scheduler's input, kept for reporting and validation.
    pub site: SiteInput,
    pub schedule: Schedule,
    /// One entry per session, in request order.
    pub limits: Vec<EvseLimit>,
}

/// Runs a scheduler over a receding horizon.
#[derive(Debug, Clone)]
pub struct Planner<S> {
    scheduler: S,
    max_horizon_slots: usize,
}

impl<S: Scheduler> Planner<S> {
    /// A planner that looks at most `max_horizon_slots` ahead.
    pub fn new(scheduler: S, max_horizon_slots: usize) -> Self {
        Self {
            scheduler,
            max_horizon_slots,
        }
    }

    pub fn scheduler(&self) -> &S {
        &self.scheduler
    }

    /// Plans from `now` up to the latest departure (capped at the maximum horizon).
    ///
    /// The current slot is treated as a full slot even when `now` falls inside it. Each
    /// profile ends with a 0 W period at the session's departure when that lies within the
    /// horizon; otherwise the last planned value holds.
    pub fn plan(&self, request: &PlanRequest) -> Plan {
        let now_slot = request.now_slot();
        let latest = request.sessions.iter().map(|s| s.departure).max();
        let horizon = latest.map_or(0, |d| {
            (d - now_slot).clamp(0, self.max_horizon_slots as i64)
        }) as usize;

        let site = SiteInput {
            slot_secs: request.slot_secs,
            now: now_slot,
            capacity_w: vec![request.planning_capacity_w(); horizon],
        };
        let schedule = self.scheduler.schedule(&site, &request.sessions);
        debug_assert_eq!(schedule.validate(&site, &request.sessions), Ok(()));

        let slot_secs = i64::from(request.slot_secs);
        let offset = |k: usize| (now_slot + k as i64) * slot_secs - request.now_secs;
        let limits = request
            .sessions
            .iter()
            .map(|s| {
                let power = &schedule.power_w[&s.id];
                let first = std::iter::once((0, power.first().copied().unwrap_or(0.0)));
                let later = power
                    .iter()
                    .enumerate()
                    .skip(1)
                    .map(|(k, &p)| (offset(k), p));
                let departs_in_horizon = horizon > 0 && s.departure <= now_slot + horizon as i64;
                let end = departs_in_horizon.then(|| (offset(horizon), 0.0));
                EvseLimit {
                    evse: s.evse.clone(),
                    session: s.id.clone(),
                    profile: StepProfile::from_steps(
                        request.now_secs,
                        first.chain(later).chain(end),
                    ),
                }
            })
            .collect();

        Plan {
            site,
            schedule,
            limits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EqualShare, Period};

    fn session(
        id: &str,
        evse_id: i32,
        departure: Slot,
        max_power_w: f64,
        remaining_wh: f64,
    ) -> SessionRequest {
        SessionRequest {
            id: id.into(),
            evse: EvseRef {
                station_id: "st".into(),
                evse_id,
            },
            departure,
            remaining_wh,
            max_power_w,
        }
    }

    fn request(now_secs: i64, sessions: Vec<SessionRequest>) -> PlanRequest {
        PlanRequest {
            now_secs,
            slot_secs: 300,
            capacity_w: 20_000.0,
            idle_evses: 0,
            fallback_limit_w: 1_400.0,
            sessions,
        }
    }

    fn periods(limit: &EvseLimit) -> Vec<(i64, f64)> {
        limit
            .profile
            .periods
            .iter()
            .map(|p| (p.start_offset_secs, p.limit_w))
            .collect()
    }

    #[test]
    fn periods_start_at_slot_boundaries_after_a_mid_slot_now() {
        // now is 120 s into slot 1000; slots then start at +180, +480, ...
        let now = 1000 * 300 + 120;
        // 11 kW for one full slot delivers 916.67 Wh; ask for a bit more so slot 2 is partial.
        let r = request(now, vec![session("a", 1, 1003, 11_000.0, 1_000.0)]);
        let plan = Planner::new(EqualShare, 96).plan(&r);
        let p = periods(&plan.limits[0]);
        assert_eq!(p[0], (0, 11_000.0));
        assert_eq!(p[1].0, 180);
        assert!((p[1].1 - 1_000.0).abs() < 1e-6, "{p:?}");
        assert_eq!(p[2], (480, 0.0));
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn ends_with_zero_at_departure() {
        let r = request(1000 * 300, vec![session("a", 1, 1002, 7_000.0, 100_000.0)]);
        let plan = Planner::new(EqualShare, 96).plan(&r);
        assert_eq!(periods(&plan.limits[0]), vec![(0, 7_000.0), (600, 0.0)]);
        assert_eq!(plan.limits[0].first_limit_w(), 7_000.0);
    }

    #[test]
    fn caps_the_horizon_and_holds_the_last_value() {
        let r = request(1000 * 300, vec![session("a", 1, 5000, 7_000.0, 1e9)]);
        let plan = Planner::new(EqualShare, 4).plan(&r);
        assert_eq!(plan.site.horizon(), 4);
        assert_eq!(
            plan.limits[0].profile.periods,
            vec![Period {
                start_offset_secs: 0,
                limit_w: 7_000.0
            }]
        );
        assert_eq!(plan.limits[0].profile.value_at(10_000_000), 7_000.0);
    }

    #[test]
    fn reserves_the_fallback_limit_for_idle_evses() {
        let mut r = request(1000 * 300, vec![session("a", 1, 1002, 50_000.0, 1e9)]);
        r.idle_evses = 3;
        let plan = Planner::new(EqualShare, 96).plan(&r);
        assert_eq!(r.planning_capacity_w(), 20_000.0 - 3.0 * 1_400.0);
        assert_eq!(plan.limits[0].first_limit_w(), 15_800.0);

        r.idle_evses = 100;
        assert_eq!(r.planning_capacity_w(), 0.0);
    }

    #[test]
    fn departed_or_missing_sessions() {
        let plan = Planner::new(EqualShare, 96).plan(&request(1000 * 300, vec![]));
        assert!(plan.limits.is_empty());
        assert_eq!(plan.site.horizon(), 0);

        let r = request(
            1000 * 300 + 10,
            vec![session("gone", 1, 1000, 7_000.0, 5_000.0)],
        );
        let plan = Planner::new(EqualShare, 96).plan(&r);
        assert_eq!(periods(&plan.limits[0]), vec![(0, 0.0)]);
    }

    #[test]
    fn splits_capacity_between_sessions() {
        let r = request(
            1000 * 300,
            vec![
                session("a", 1, 1001, 22_000.0, 1e9),
                session("b", 2, 1002, 22_000.0, 1e9),
            ],
        );
        let plan = Planner::new(EqualShare, 96).plan(&r);
        assert_eq!(periods(&plan.limits[0]), vec![(0, 10_000.0), (300, 0.0)]);
        assert_eq!(
            periods(&plan.limits[1]),
            vec![(0, 10_000.0), (300, 20_000.0), (600, 0.0)]
        );
    }
}
