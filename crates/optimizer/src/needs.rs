//! What each session still needs, as far as the controller can tell.
//!
//! ISO 15118 EVs report their departure time and energy. For the others the controller
//! assumes site defaults. When an EV outlives an estimate (still plugged in after the
//! assumed departure, or still charging after the assumed energy), the estimate is rolled
//! forward by one slot at a time, so the EV is treated as about to be done rather than
//! dropped.

use crate::types::{EvseRef, SessionId, SessionRequest};

/// Assumptions for EVs that don't report their needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NeedsDefaults {
    /// Assumed time between plug-in and departure.
    pub dwell_secs: i64,
    /// Assumed energy for the whole session.
    pub energy_wh: f64,
}

/// Needs reported by the EV (NotifyEVChargingNeeds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnownNeeds {
    /// Unix time the EV leaves.
    pub departure_secs: i64,
    /// Energy requested for the whole session.
    pub energy_wh: f64,
    /// The EV's max power, when it reports one.
    pub max_power_w: Option<f64>,
}

/// What the controller knows about one ongoing session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionState {
    pub id: SessionId,
    pub evse: EvseRef,
    /// Unix time the session started.
    pub started_secs: i64,
    pub evse_max_w: f64,
    /// Energy delivered so far.
    pub delivered_wh: f64,
    pub needs: Option<KnownNeeds>,
    /// The EV has stopped taking power (it is full), so it needs nothing more.
    pub ev_suspended: bool,
}

impl SessionState {
    /// The scheduler input for this session at `now_secs`.
    ///
    /// The departure slot is `floor(departure / slot_secs)`, an exclusive end that leaves
    /// out a final partial slot. A departure at or before `now` becomes `now + slot_secs`.
    /// Remaining energy of zero or less, on an EV still charging, becomes one slot at max
    /// power.
    pub fn request(
        &self,
        now_secs: i64,
        slot_secs: u32,
        defaults: &NeedsDefaults,
    ) -> SessionRequest {
        let slot = i64::from(slot_secs);
        let max_power_w = self
            .needs
            .and_then(|n| n.max_power_w)
            .map_or(self.evse_max_w, |ev| ev.min(self.evse_max_w))
            .max(0.0);

        let departure_secs = self
            .needs
            .map_or(self.started_secs + defaults.dwell_secs, |n| {
                n.departure_secs
            });
        let departure_secs = if departure_secs <= now_secs {
            now_secs + slot
        } else {
            departure_secs
        };

        let energy_wh = self.needs.map_or(defaults.energy_wh, |n| n.energy_wh);
        let remaining_wh = if self.ev_suspended {
            0.0
        } else if energy_wh - self.delivered_wh <= 0.0 {
            max_power_w * f64::from(slot_secs) / 3600.0
        } else {
            energy_wh - self.delivered_wh
        };

        SessionRequest {
            id: self.id.clone(),
            evse: self.evse.clone(),
            departure: departure_secs.div_euclid(slot),
            remaining_wh,
            max_power_w,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOT: u32 = 300;
    const DEFAULTS: NeedsDefaults = NeedsDefaults {
        dwell_secs: 3_600,
        energy_wh: 10_000.0,
    };

    fn state(needs: Option<KnownNeeds>) -> SessionState {
        SessionState {
            id: "tx".into(),
            evse: EvseRef {
                station_id: "st".into(),
                evse_id: 1,
            },
            started_secs: 30_000,
            evse_max_w: 22_000.0,
            delivered_wh: 0.0,
            needs,
            ev_suspended: false,
        }
    }

    #[test]
    fn unknown_needs_use_the_defaults() {
        let r = state(None).request(30_120, SLOT, &DEFAULTS);
        assert_eq!(r.departure, (30_000 + 3_600) / 300);
        assert_eq!(r.remaining_wh, 10_000.0);
        assert_eq!(r.max_power_w, 22_000.0);
    }

    #[test]
    fn known_needs_override_the_defaults() {
        let s = SessionState {
            delivered_wh: 2_500.0,
            ..state(Some(KnownNeeds {
                departure_secs: 40_250,
                energy_wh: 30_000.0,
                max_power_w: Some(11_000.0),
            }))
        };
        let r = s.request(31_000, SLOT, &DEFAULTS);
        // A partial final slot is left out: 40_250 lies inside slot 134.
        assert_eq!(r.departure, 134);
        assert_eq!(r.remaining_wh, 27_500.0);
        assert_eq!(r.max_power_w, 11_000.0);
    }

    #[test]
    fn ev_max_is_capped_by_the_evse() {
        let s = state(Some(KnownNeeds {
            departure_secs: 40_000,
            energy_wh: 30_000.0,
            max_power_w: Some(50_000.0),
        }));
        assert_eq!(s.request(30_000, SLOT, &DEFAULTS).max_power_w, 22_000.0);
    }

    #[test]
    fn departure_rolls_forward_one_slot_at_a_time() {
        // The default departure is 33_600; at 33_700 the EV is still there.
        let r = state(None).request(33_700, SLOT, &DEFAULTS);
        assert_eq!(r.departure, 33_700 / 300 + 1);
        assert_eq!(r.departure, (33_700 + 300) / 300);
    }

    #[test]
    fn energy_rolls_forward_while_the_ev_keeps_charging() {
        let s = SessionState {
            delivered_wh: 10_000.0,
            ..state(None)
        };
        let r = s.request(31_000, SLOT, &DEFAULTS);
        assert!((r.remaining_wh - 22_000.0 * 300.0 / 3600.0).abs() < 1e-9);
    }

    #[test]
    fn a_suspended_ev_needs_nothing() {
        let s = SessionState {
            delivered_wh: 4_000.0,
            ev_suspended: true,
            ..state(None)
        };
        assert_eq!(s.request(31_000, SLOT, &DEFAULTS).remaining_wh, 0.0);
    }
}
