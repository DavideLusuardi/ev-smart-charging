//! What the CSMS knows about its stations and sessions at runtime.
//!
//! Whether a station is connected right now is the [`Registry`](crate::Registry)'s job; this
//! keeps what the stations reported, which outlives a connection.

use std::collections::{BTreeMap, BTreeSet};

use ocpp::DateTime;
use ocpp::messages::boot_notification::ChargingStation;
use ocpp::messages::status_notification::ConnectorStatus;
use ocpp::messages::transaction_event::ChargingState;
use optimizer::{EvseRef, KnownNeeds, Plan, SessionState, StepProfile};

use crate::charging_needs::ReportedNeeds;
use crate::station_id::StationId;

/// Every station that has connected since the CSMS started, and the sessions in progress.
#[derive(Debug)]
pub struct SiteState {
    /// The site's grid limit.
    pub grid_limit_w: f64,
    /// The control loop's latest plan.
    pub latest_plan: Option<Plan>,
    stations: BTreeMap<StationId, StationState>,
    /// Sessions in progress, by transaction id.
    sessions: BTreeMap<String, Session>,
    /// The TxProfile each EVSE last accepted. An EVSE without one follows its
    /// TxDefaultProfile.
    effective: BTreeMap<EvseRef, StepProfile>,
    /// EVSEs whose effective profile is only an upper bound, because an update went
    /// unanswered: the EVSE follows either its old profile or the new one.
    uncertain: BTreeSet<EvseRef>,
    /// Needs reported on an EVSE before its transaction started.
    pending_needs: BTreeMap<EvseRef, ReportedNeeds>,
}

/// One station.
#[derive(Debug, Clone, PartialEq)]
pub struct StationState {
    /// What the station reported in its last `BootNotification`, if it sent one.
    pub info: Option<StationInfo>,
    /// When the CSMS last received a frame from the station.
    pub last_seen: DateTime,
    /// EVSEs by id, as reported in `StatusNotification`s.
    pub evses: BTreeMap<i32, EvseState>,
}

/// A station's identification from `BootNotification`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationInfo {
    pub vendor: String,
    pub model: String,
    pub serial_number: Option<String>,
    pub firmware_version: Option<String>,
}

impl From<&ChargingStation> for StationInfo {
    fn from(station: &ChargingStation) -> Self {
        Self {
            vendor: station.vendor_name.to_string(),
            model: station.model.to_string(),
            serial_number: station.serial_number.as_ref().map(ToString::to_string),
            firmware_version: station.firmware_version.as_ref().map(ToString::to_string),
        }
    }
}

/// One EVSE of a station.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EvseState {
    /// The last reported status of each connector, by connector id.
    pub connectors: BTreeMap<i32, ConnectorStatus>,
}

/// A transaction in progress, as reported in `TransactionEvent`s.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub transaction_id: String,
    pub evse: EvseRef,
    /// When the transaction started, or when the CSMS first heard of it.
    pub started: DateTime,
    /// The energy register at the first reading; delivered energy counts from there.
    pub meter_start_wh: Option<f64>,
    /// Energy delivered so far.
    pub delivered_wh: f64,
    /// The last reported power.
    pub power_w: Option<f64>,
    /// The last reported charging state.
    pub charging_state: Option<ChargingState>,
    /// What the EV reported it needs, if it did.
    pub needs: Option<KnownNeeds>,
    /// The EV's max power as learned from meter values.
    pub observed_max_w: Option<f64>,
    /// The EV stopped taking power (`SuspendedEV`): it is full.
    pub ev_suspended: bool,
}

impl Session {
    /// A session with nothing measured yet.
    pub fn new(transaction_id: impl Into<String>, evse: EvseRef, started: DateTime) -> Self {
        Self {
            transaction_id: transaction_id.into(),
            evse,
            started,
            meter_start_wh: None,
            delivered_wh: 0.0,
            power_w: None,
            charging_state: None,
            needs: None,
            observed_max_w: None,
            ev_suspended: false,
        }
    }

    /// What the controller knows about the session, for planning.
    pub fn to_state(&self, evse_max_w: f64) -> SessionState {
        SessionState {
            id: self.transaction_id.as_str().into(),
            evse: self.evse.clone(),
            started_secs: self.started.timestamp(),
            evse_max_w,
            delivered_wh: self.delivered_wh,
            needs: self.needs,
            observed_max_w: self.observed_max_w,
            ev_suspended: self.ev_suspended,
        }
    }
}

impl SiteState {
    /// A site with no stations yet.
    pub fn new(grid_limit_w: f64) -> Self {
        Self {
            grid_limit_w,
            latest_plan: None,
            stations: BTreeMap::new(),
            sessions: BTreeMap::new(),
            effective: BTreeMap::new(),
            uncertain: BTreeSet::new(),
            pending_needs: BTreeMap::new(),
        }
    }

    /// Records that a frame arrived from `id` at `now`, adding the station if it is new.
    pub fn touch(&mut self, id: &StationId, now: DateTime) -> &mut StationState {
        self.stations
            .entry(id.clone())
            .and_modify(|s| s.last_seen = now)
            .or_insert_with(|| StationState {
                info: None,
                last_seen: now,
                evses: BTreeMap::new(),
            })
    }

    /// One station, if it has ever connected.
    pub fn station(&self, id: &StationId) -> Option<&StationState> {
        self.stations.get(id)
    }

    /// All stations, sorted by identity.
    pub fn stations(&self) -> impl Iterator<Item = (&StationId, &StationState)> {
        self.stations.iter()
    }

    /// Every EVSE the CSMS knows of: reported in a `StatusNotification` or running a session.
    pub fn evses(&self) -> BTreeSet<EvseRef> {
        let reported = self.stations.iter().flat_map(|(id, station)| {
            station.evses.keys().map(|&evse_id| EvseRef {
                station_id: id.as_str().to_owned(),
                evse_id,
            })
        });
        let in_use = self.sessions.values().map(|s| s.evse.clone());
        reported.chain(in_use).collect()
    }

    /// Sessions in progress, by transaction id.
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    pub fn session(&self, transaction_id: &str) -> Option<&Session> {
        self.sessions.get(transaction_id)
    }

    pub fn session_mut(&mut self, transaction_id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(transaction_id)
    }

    /// The session on an EVSE, if there is one.
    pub fn session_on(&self, evse: &EvseRef) -> Option<&Session> {
        self.sessions.values().find(|s| &s.evse == evse)
    }

    /// Adds a session. A session still recorded on the same EVSE is dropped: the station
    /// can only run one at a time there, so the CSMS missed its end.
    pub fn start_session(&mut self, session: Session) {
        let stale: Vec<String> = self
            .sessions
            .values()
            .filter(|s| s.evse == session.evse)
            .map(|s| s.transaction_id.clone())
            .collect();
        for id in stale {
            self.end_session(&id);
        }
        self.sessions
            .insert(session.transaction_id.clone(), session);
    }

    /// Removes a session and its EVSE's TxProfile, which the station drops with the
    /// transaction.
    pub fn end_session(&mut self, transaction_id: &str) -> Option<Session> {
        let session = self.sessions.remove(transaction_id)?;
        self.effective.remove(&session.evse);
        self.uncertain.remove(&session.evse);
        Some(session)
    }

    /// Sets what a session needs, replacing any earlier report or estimate. Returns whether
    /// the session exists.
    pub fn set_needs(&mut self, transaction_id: &str, needs: KnownNeeds) -> bool {
        self.session_mut(transaction_id)
            .map(|s| s.needs = Some(needs))
            .is_some()
    }

    /// Keeps needs reported on an EVSE with no session, for the next session there.
    pub fn set_pending_needs(&mut self, evse: EvseRef, needs: ReportedNeeds) {
        self.pending_needs.insert(evse, needs);
    }

    /// Takes the needs waiting on an EVSE, or drops them when the EV left without charging.
    pub fn take_pending_needs(&mut self, evse: &EvseRef) -> Option<ReportedNeeds> {
        self.pending_needs.remove(evse)
    }

    /// The TxProfile each EVSE last accepted.
    pub fn effective_profiles(&self) -> &BTreeMap<EvseRef, StepProfile> {
        &self.effective
    }

    /// Records that an EVSE accepted a TxProfile.
    pub fn set_effective(&mut self, evse: EvseRef, profile: StepProfile) {
        self.uncertain.remove(&evse);
        self.effective.insert(evse, profile);
    }

    /// Records that an EVSE follows at most `bound`, without knowing its exact limit.
    pub fn set_effective_bound(&mut self, evse: EvseRef, bound: StepProfile) {
        self.uncertain.insert(evse.clone());
        self.effective.insert(evse, bound);
    }

    /// The limit an EVSE followed at unix time `t_secs`: its TxProfile, or `fallback_w` (its
    /// TxDefaultProfile) without one. `None` when the CSMS can't tell: the last update went
    /// unanswered, or `t_secs` is before the TxProfile started, under an earlier limit.
    pub fn known_limit_at(&self, evse: &EvseRef, t_secs: i64, fallback_w: f64) -> Option<f64> {
        if self.uncertain.contains(evse) {
            return None;
        }
        match self.effective.get(evse) {
            Some(p) if t_secs < p.start_secs => None,
            Some(p) => Some(p.value_at(t_secs)),
            None => Some(fallback_w),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evse(station: &str, evse_id: i32) -> EvseRef {
        EvseRef {
            station_id: station.into(),
            evse_id,
        }
    }

    #[test]
    fn a_new_session_on_a_busy_evse_replaces_the_old_one() {
        let mut site = SiteState::new(100_000.0);
        let now = DateTime::now();
        site.start_session(Session::new("a", evse("CS1", 1), now));
        site.set_effective(evse("CS1", 1), StepProfile::constant(0, 7_000.0));
        site.start_session(Session::new("b", evse("CS1", 2), now));
        site.start_session(Session::new("c", evse("CS1", 1), now));

        let ids: Vec<_> = site.sessions().map(|s| s.transaction_id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
        assert!(site.effective_profiles().is_empty());
        assert_eq!(
            site.known_limit_at(&evse("CS1", 1), 0, 1_400.0),
            Some(1_400.0)
        );
    }

    #[test]
    fn known_evses_include_those_with_sessions() {
        let mut site = SiteState::new(100_000.0);
        let now = DateTime::now();
        let cs1 = StationId::try_from("CS1").unwrap();
        site.touch(&cs1, now).evses.insert(2, EvseState::default());
        site.start_session(Session::new("a", evse("CS2", 1), now));
        assert_eq!(
            site.evses().into_iter().collect::<Vec<_>>(),
            [evse("CS1", 2), evse("CS2", 1)]
        );
    }
}
