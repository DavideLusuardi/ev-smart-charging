//! Domain types shared by every scheduling strategy.
//!
//! Time is split into slots of `slot_secs` seconds, numbered absolutely as
//! `floor(unix_secs / slot_secs)`. Power is in W and energy in Wh throughout.

use std::collections::BTreeMap;
use std::fmt;

/// An absolute slot number: `floor(unix_secs / slot_secs)`.
pub type Slot = i64;

/// Identifies a charging session (the OCPP transaction id in the CSMS).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub String);

impl From<&str> for SessionId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifies an EVSE: a station and the EVSE number within it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EvseRef {
    pub station_id: String,
    pub evse_id: i32,
}

impl fmt::Display for EvseRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.station_id, self.evse_id)
    }
}

/// The site over the planning horizon: slot `k` of the horizon is absolute slot `now + k`.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteInput {
    pub slot_secs: u32,
    pub now: Slot,
    /// Grid capacity available to the sessions, per horizon slot.
    pub capacity_w: Vec<f64>,
}

impl SiteInput {
    /// Number of slots in the horizon.
    pub fn horizon(&self) -> usize {
        self.capacity_w.len()
    }

    /// Length of one slot in hours, to convert W into Wh per slot.
    pub fn slot_hours(&self) -> f64 {
        f64::from(self.slot_secs) / 3600.0
    }
}

/// What one session needs from the scheduler.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRequest {
    pub id: SessionId,
    pub evse: EvseRef,
    /// First slot in which the EV is gone (exclusive end).
    pub departure: Slot,
    /// Requested energy minus the energy already delivered.
    pub remaining_wh: f64,
    /// min(EVSE max, EV max).
    pub max_power_w: f64,
}

impl SessionRequest {
    /// Number of horizon slots in which the session may charge.
    pub fn active_slots(&self, site: &SiteInput) -> usize {
        let slots = (self.departure - site.now).clamp(0, site.horizon() as i64);
        slots as usize
    }
}

/// Planned power per session and horizon slot.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    /// The absolute slot of each horizon position.
    pub slots: Vec<Slot>,
    /// Power per session, one entry per horizon slot.
    pub power_w: BTreeMap<SessionId, Vec<f64>>,
}

impl Schedule {
    /// A schedule with zero power for every session.
    pub fn zeros(site: &SiteInput, sessions: &[SessionRequest]) -> Self {
        let horizon = site.horizon();
        Self {
            slots: (0..horizon as i64).map(|k| site.now + k).collect(),
            power_w: sessions
                .iter()
                .map(|s| (s.id.clone(), vec![0.0; horizon]))
                .collect(),
        }
    }

    /// Energy scheduled for one session, in Wh.
    pub fn energy_wh(&self, id: &SessionId, site: &SiteInput) -> f64 {
        self.power_w
            .get(id)
            .map_or(0.0, |p| p.iter().sum::<f64>() * site.slot_hours())
    }

    /// Energy scheduled for all sessions, in Wh.
    pub fn total_energy_wh(&self, site: &SiteInput) -> f64 {
        self.power_w.values().flatten().sum::<f64>() * site.slot_hours()
    }

    /// Total power in horizon slot `k`.
    pub fn slot_total_w(&self, k: usize) -> f64 {
        self.power_w.values().map(|p| p[k]).sum()
    }

    /// Checks the schedule against the site and the sessions it was built for.
    pub fn validate(&self, site: &SiteInput, sessions: &[SessionRequest]) -> Result<(), Violation> {
        let horizon = site.horizon();
        if self.slots.len() != horizon {
            return Err(Violation::WrongHorizon {
                expected: horizon,
                got: self.slots.len(),
            });
        }
        if self.power_w.len() != sessions.len() {
            return Err(Violation::SessionCount {
                expected: sessions.len(),
                got: self.power_w.len(),
            });
        }
        for s in sessions {
            let power = self
                .power_w
                .get(&s.id)
                .ok_or_else(|| Violation::MissingSession(s.id.clone()))?;
            if power.len() != horizon {
                return Err(Violation::WrongHorizon {
                    expected: horizon,
                    got: power.len(),
                });
            }
            let active = s.active_slots(site);
            for (k, &p) in power.iter().enumerate() {
                if p < -tolerance(s.max_power_w) {
                    return Err(Violation::NegativePower {
                        session: s.id.clone(),
                        slot: k,
                        power_w: p,
                    });
                }
                if p > s.max_power_w.max(0.0) + tolerance(s.max_power_w) {
                    return Err(Violation::AboveMaxPower {
                        session: s.id.clone(),
                        slot: k,
                        power_w: p,
                        max_w: s.max_power_w,
                    });
                }
                if k >= active && p > tolerance(s.max_power_w) {
                    return Err(Violation::AfterDeparture {
                        session: s.id.clone(),
                        slot: k,
                        power_w: p,
                    });
                }
            }
            let energy = self.energy_wh(&s.id, site);
            if energy > s.remaining_wh.max(0.0) + tolerance(s.remaining_wh) {
                return Err(Violation::AboveRemainingEnergy {
                    session: s.id.clone(),
                    energy_wh: energy,
                    remaining_wh: s.remaining_wh,
                });
            }
        }
        for (k, &capacity) in site.capacity_w.iter().enumerate() {
            let total = self.slot_total_w(k);
            if total > capacity.max(0.0) + tolerance(capacity) {
                return Err(Violation::OverCapacity {
                    slot: k,
                    total_w: total,
                    capacity_w: capacity,
                });
            }
        }
        Ok(())
    }
}

/// Slack allowed for floating-point and solver error: 1e-6 relative plus 1e-3 absolute.
pub fn tolerance(scale: f64) -> f64 {
    1e-6 * scale.abs() + 1e-3
}

/// Why a schedule is invalid.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Violation {
    #[error("expected {expected} horizon slots, got {got}")]
    WrongHorizon { expected: usize, got: usize },
    #[error("expected {expected} sessions, got {got}")]
    SessionCount { expected: usize, got: usize },
    #[error("session {0} is missing from the schedule")]
    MissingSession(SessionId),
    #[error("session {session} has negative power {power_w} W in slot {slot}")]
    NegativePower {
        session: SessionId,
        slot: usize,
        power_w: f64,
    },
    #[error("session {session} has {power_w} W in slot {slot}, above its max {max_w} W")]
    AboveMaxPower {
        session: SessionId,
        slot: usize,
        power_w: f64,
        max_w: f64,
    },
    #[error("session {session} has {power_w} W in slot {slot}, after its departure")]
    AfterDeparture {
        session: SessionId,
        slot: usize,
        power_w: f64,
    },
    #[error("session {session} gets {energy_wh} Wh, above its remaining {remaining_wh} Wh")]
    AboveRemainingEnergy {
        session: SessionId,
        energy_wh: f64,
        remaining_wh: f64,
    },
    #[error("slot {slot} uses {total_w} W, above the capacity {capacity_w} W")]
    OverCapacity {
        slot: usize,
        total_w: f64,
        capacity_w: f64,
    },
}

/// A scheduling strategy.
pub trait Scheduler {
    /// A short name for reports (`equal`, `edf`, `lp`).
    fn name(&self) -> &'static str;

    /// Plans power for every session over the site's horizon.
    fn schedule(&self, site: &SiteInput, sessions: &[SessionRequest]) -> Schedule;
}

/// A session competing for one slot's capacity.
pub(crate) struct Candidate<'a> {
    pub session: &'a SessionRequest,
    /// Energy the session still needs before this slot.
    pub needed_wh: f64,
    /// The most it can take in this slot: its max power, limited by `needed_wh`.
    pub limit_w: f64,
}

/// Builds a schedule slot by slot. For each slot, `allocate` gets the sessions that are still
/// present and the slot's capacity, and returns their power in the same order. Energy
/// delivered in earlier slots is subtracted before the next one.
pub(crate) fn schedule_per_slot(
    site: &SiteInput,
    sessions: &[SessionRequest],
    allocate: impl Fn(&[Candidate<'_>], f64) -> Vec<f64>,
) -> Schedule {
    let hours = site.slot_hours();
    let mut schedule = Schedule::zeros(site, sessions);
    let mut needed: Vec<f64> = sessions.iter().map(|s| s.remaining_wh.max(0.0)).collect();
    let active: Vec<usize> = sessions.iter().map(|s| s.active_slots(site)).collect();

    for (k, &capacity) in site.capacity_w.iter().enumerate() {
        let present: Vec<usize> = (0..sessions.len()).filter(|&i| k < active[i]).collect();
        let candidates: Vec<Candidate<'_>> = present
            .iter()
            .map(|&i| Candidate {
                session: &sessions[i],
                needed_wh: needed[i],
                limit_w: sessions[i].max_power_w.min(needed[i] / hours).max(0.0),
            })
            .collect();
        let power = allocate(&candidates, capacity.max(0.0));
        for (&i, p) in present.iter().zip(power) {
            schedule
                .power_w
                .get_mut(&sessions[i].id)
                .expect("zeros has every session")[k] = p;
            needed[i] = (needed[i] - p * hours).max(0.0);
        }
    }
    schedule
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A one-hour-slot site with constant capacity.
    pub(crate) fn site(capacity_w: f64, horizon: usize) -> SiteInput {
        SiteInput {
            slot_secs: 3600,
            now: 100,
            capacity_w: vec![capacity_w; horizon],
        }
    }

    pub(crate) fn session(
        id: &str,
        departs_after: i64,
        max_power_w: f64,
        remaining_wh: f64,
    ) -> SessionRequest {
        SessionRequest {
            id: id.into(),
            evse: EvseRef {
                station_id: "st".into(),
                evse_id: 1,
            },
            departure: 100 + departs_after,
            remaining_wh,
            max_power_w,
        }
    }

    fn with_power(
        site: &SiteInput,
        sessions: &[SessionRequest],
        id: &str,
        power: Vec<f64>,
    ) -> Schedule {
        let mut schedule = Schedule::zeros(site, sessions);
        schedule.power_w.insert(id.into(), power);
        schedule
    }

    #[test]
    fn zero_schedule_is_valid() {
        let site = site(10.0, 3);
        let sessions = [session("a", 2, 10.0, 10.0)];
        let schedule = Schedule::zeros(&site, &sessions);
        assert_eq!(schedule.slots, vec![100, 101, 102]);
        assert_eq!(schedule.validate(&site, &sessions), Ok(()));
    }

    #[test]
    fn detects_over_capacity() {
        let site = site(10.0, 2);
        let sessions = [session("a", 2, 10.0, 100.0), session("b", 2, 10.0, 100.0)];
        let mut schedule = Schedule::zeros(&site, &sessions);
        schedule.power_w.insert("a".into(), vec![6.0, 0.0]);
        schedule.power_w.insert("b".into(), vec![6.0, 0.0]);
        assert!(matches!(
            schedule.validate(&site, &sessions),
            Err(Violation::OverCapacity { slot: 0, .. })
        ));
    }

    #[test]
    fn detects_per_session_violations() {
        let site = site(100.0, 3);
        let sessions = [session("a", 2, 10.0, 15.0)];
        let check =
            |power: Vec<f64>| with_power(&site, &sessions, "a", power).validate(&site, &sessions);

        assert!(matches!(
            check(vec![-1.0, 0.0, 0.0]),
            Err(Violation::NegativePower { .. })
        ));
        assert!(matches!(
            check(vec![11.0, 0.0, 0.0]),
            Err(Violation::AboveMaxPower { .. })
        ));
        assert!(matches!(
            check(vec![0.0, 0.0, 5.0]),
            Err(Violation::AfterDeparture { slot: 2, .. })
        ));
        assert!(matches!(
            check(vec![10.0, 10.0, 0.0]),
            Err(Violation::AboveRemainingEnergy { .. })
        ));
        assert_eq!(check(vec![10.0, 5.0, 0.0]), Ok(()));
    }

    #[test]
    fn detects_shape_errors() {
        let site = site(100.0, 3);
        let sessions = [session("a", 2, 10.0, 15.0)];
        let short = with_power(&site, &sessions, "a", vec![0.0]);
        assert!(matches!(
            short.validate(&site, &sessions),
            Err(Violation::WrongHorizon { .. })
        ));
        let other = [session("b", 2, 10.0, 15.0)];
        let wrong = Schedule::zeros(&site, &other);
        assert_eq!(
            wrong.validate(&site, &sessions),
            Err(Violation::MissingSession("a".into()))
        );
    }

    #[test]
    fn tolerates_solver_noise() {
        let site = site(10.0, 1);
        let sessions = [session("a", 1, 10.0, 10.0)];
        let schedule = with_power(&site, &sessions, "a", vec![10.0 + 1e-6]);
        assert_eq!(schedule.validate(&site, &sessions), Ok(()));
    }

    #[test]
    fn active_slots_clamp_to_horizon() {
        let site = site(10.0, 4);
        assert_eq!(session("a", 2, 1.0, 1.0).active_slots(&site), 2);
        assert_eq!(session("a", 9, 1.0, 1.0).active_slots(&site), 4);
        assert_eq!(session("a", -1, 1.0, 1.0).active_slots(&site), 0);
    }
}
