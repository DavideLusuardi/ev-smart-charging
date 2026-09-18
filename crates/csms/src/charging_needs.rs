//! `NotifyEVChargingNeeds`: an ISO 15118 EV reports when it leaves and how much energy it
//! wants.
//!
//! Anything the EV leaves out is filled in from the site defaults, the same ones used for EVs
//! that report nothing. Needs can arrive before the transaction starts; they wait on the EVSE
//! until it does, or until the EVSE is free again.

use ocpp::messages::notify_ev_charging_needs::{
    ChargingNeeds, EnergyTransferMode, NotifyEvChargingNeedsRequest, NotifyEvChargingNeedsResponse,
    NotifyEvChargingNeedsStatus,
};
use optimizer::{EvseRef, KnownNeeds, NeedsDefaults};
use tracing::info;

use crate::SiteConfig;
use crate::state::SiteState;
use crate::station_id::StationId;

/// What an EV reported, before the defaults fill the gaps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReportedNeeds {
    /// Unix time the EV leaves.
    pub departure_secs: Option<i64>,
    /// Energy wanted for the whole session.
    pub energy_wh: Option<f64>,
    /// The EV's max power.
    pub max_power_w: Option<f64>,
}

impl ReportedNeeds {
    /// Reads the needs, in W and Wh.
    ///
    /// AC power is `evMaxCurrent × evMaxVoltage × phases`, with the phase count of the
    /// requested transfer mode, at most the site's `ac_phases`. DC power is `evMaxPower`, or
    /// `evMaxCurrent × evMaxVoltage` without it.
    pub fn from_ocpp(needs: &ChargingNeeds, site: &SiteConfig) -> Self {
        let mut reported = Self {
            departure_secs: needs.departure_time.map(|t| t.timestamp()),
            energy_wh: None,
            max_power_w: None,
        };
        if let Some(ac) = &needs.ac_charging_parameters {
            let phases = match needs.requested_energy_transfer {
                EnergyTransferMode::AcSinglePhase => 1,
                EnergyTransferMode::AcTwoPhase => 2,
                EnergyTransferMode::AcThreePhase | EnergyTransferMode::Dc => 3,
            }
            .min(site.ac_phases);
            reported.energy_wh = Some(f64::from(ac.energy_amount));
            reported.max_power_w = Some(
                f64::from(ac.ev_max_current) * f64::from(ac.ev_max_voltage) * f64::from(phases),
            );
        } else if let Some(dc) = &needs.dc_charging_parameters {
            reported.energy_wh = dc.energy_amount.map(f64::from);
            reported.max_power_w = Some(dc.ev_max_power.map_or_else(
                || f64::from(dc.ev_max_current) * f64::from(dc.ev_max_voltage),
                f64::from,
            ));
        }
        // Nonsense values say nothing.
        reported.energy_wh = reported.energy_wh.filter(|&e| e >= 0.0);
        reported.max_power_w = reported.max_power_w.filter(|&p| p > 0.0);
        reported
    }

    /// The needs to plan with, for a session that started at `started_secs`.
    pub fn resolve(&self, started_secs: i64, defaults: &NeedsDefaults) -> KnownNeeds {
        KnownNeeds {
            departure_secs: self
                .departure_secs
                .unwrap_or(started_secs + defaults.dwell_secs),
            energy_wh: self.energy_wh.unwrap_or(defaults.energy_wh),
            max_power_w: self.max_power_w,
        }
    }
}

/// Applies one `NotifyEVChargingNeeds` from `station`. Returns the response, and whether
/// the plan must change.
pub(crate) fn notify_ev_charging_needs(
    site_config: &SiteConfig,
    site: &mut SiteState,
    station: &StationId,
    req: &NotifyEvChargingNeedsRequest,
) -> (NotifyEvChargingNeedsResponse, bool) {
    let evse = EvseRef {
        station_id: station.as_str().to_owned(),
        evse_id: req.evse_id,
    };
    let reported = ReportedNeeds::from_ocpp(&req.charging_needs, site_config);
    info!(%evse, ?reported, "EV charging needs");
    let tx = site.session_on(&evse).map(|s| s.transaction_id.clone());
    let replan = match tx {
        Some(tx) => {
            let started = site.session(&tx).expect("just found").started.timestamp();
            site.set_needs(&tx, reported.resolve(started, &site_config.needs_defaults));
            true
        }
        None => {
            site.set_pending_needs(evse, reported);
            false
        }
    };
    let response = NotifyEvChargingNeedsResponse {
        custom_data: None,
        status: NotifyEvChargingNeedsStatus::Accepted,
        status_info: None,
    };
    (response, replan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use crate::handlers::tests::{call, new_site, run};
    use serde_json::{Value, json};

    fn needs(value: Value) -> ReportedNeeds {
        let needs: ChargingNeeds = ocpp::decode(value).unwrap();
        ReportedNeeds::from_ocpp(&needs, &SiteConfig::default())
    }

    #[test]
    fn ac_power_uses_the_phases_of_the_transfer_mode() {
        let ac = |mode: &str| {
            needs(json!({
                "requestedEnergyTransfer": mode,
                "departureTime": "2026-09-30T18:00:00Z",
                "acChargingParameters": {
                    "energyAmount": 20000, "evMinCurrent": 6, "evMaxCurrent": 16, "evMaxVoltage": 230
                }
            }))
        };
        let three = ac("AC_three_phase");
        assert_eq!(three.max_power_w, Some(16.0 * 230.0 * 3.0));
        assert_eq!(three.energy_wh, Some(20_000.0));
        assert_eq!(
            three.departure_secs,
            Some(
                "2026-09-30T18:00:00Z"
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .unwrap()
                    .timestamp()
            )
        );
        assert_eq!(ac("AC_single_phase").max_power_w, Some(16.0 * 230.0));

        // A single-phase site caps a three-phase EV.
        let site = SiteConfig {
            ac_phases: 1,
            ..SiteConfig::default()
        };
        let needs: ChargingNeeds = ocpp::decode(json!({
            "requestedEnergyTransfer": "AC_three_phase",
            "acChargingParameters": {
                "energyAmount": 20000, "evMinCurrent": 6, "evMaxCurrent": 16, "evMaxVoltage": 230
            }
        }))
        .unwrap();
        assert_eq!(
            ReportedNeeds::from_ocpp(&needs, &site).max_power_w,
            Some(16.0 * 230.0)
        );
    }

    #[test]
    fn dc_power_prefers_the_reported_max_power() {
        let with_power = needs(json!({
            "requestedEnergyTransfer": "DC",
            "dcChargingParameters": {
                "evMaxCurrent": 400, "evMaxVoltage": 800, "evMaxPower": 150000, "energyAmount": 45000
            }
        }));
        assert_eq!(with_power.max_power_w, Some(150_000.0));
        assert_eq!(with_power.energy_wh, Some(45_000.0));
        assert_eq!(with_power.departure_secs, None);

        let without = needs(json!({
            "requestedEnergyTransfer": "DC",
            "dcChargingParameters": { "evMaxCurrent": 200, "evMaxVoltage": 500 }
        }));
        assert_eq!(without.max_power_w, Some(100_000.0));
        assert_eq!(without.energy_wh, None);
    }

    #[test]
    fn missing_needs_come_from_the_defaults() {
        let defaults = NeedsDefaults {
            dwell_secs: 3_600,
            energy_wh: 10_000.0,
        };
        let reported = ReportedNeeds {
            departure_secs: None,
            energy_wh: None,
            max_power_w: Some(50_000.0),
        };
        assert_eq!(
            reported.resolve(1_000, &defaults),
            KnownNeeds {
                departure_secs: 4_600,
                energy_wh: 10_000.0,
                max_power_w: Some(50_000.0),
            }
        );
    }

    fn notify(departure: &str) -> Value {
        json!({
            "evseId": 1,
            "chargingNeeds": {
                "requestedEnergyTransfer": "DC",
                "departureTime": departure,
                "dcChargingParameters": { "evMaxCurrent": 200, "evMaxVoltage": 500, "energyAmount": 30000 }
            }
        })
    }

    fn started() -> Value {
        json!({
            "eventType": "Started",
            "timestamp": "2026-09-30T10:00:00Z",
            "triggerReason": "CablePluggedIn",
            "seqNo": 0,
            "transactionInfo": { "transactionId": "tx" },
            "evse": { "id": 1 }
        })
    }

    #[test]
    fn needs_for_a_running_session_apply_at_once() {
        let config = Config::default();
        let mut site = new_site();
        call(&config, &mut site, "TransactionEvent", started());
        let handled = run(
            &config,
            &mut site,
            "NotifyEVChargingNeeds",
            notify("2026-09-30T11:00:00Z"),
        );
        assert!(handled.replan);
        let needs = site.session("tx").unwrap().needs.unwrap();
        assert_eq!(needs.energy_wh, 30_000.0);
        assert_eq!(needs.max_power_w, Some(100_000.0));
    }

    #[test]
    fn needs_before_the_transaction_wait_for_it() {
        let config = Config::default();
        let mut site = new_site();
        let response = call(
            &config,
            &mut site,
            "NotifyEVChargingNeeds",
            notify("2026-09-30T11:00:00Z"),
        );
        assert_eq!(response, json!({ "status": "Accepted" }));
        call(&config, &mut site, "TransactionEvent", started());
        assert_eq!(
            site.session("tx").unwrap().needs.unwrap().energy_wh,
            30_000.0
        );
    }

    #[test]
    fn waiting_needs_are_dropped_when_the_evse_is_free() {
        let config = Config::default();
        let mut site = new_site();
        call(
            &config,
            &mut site,
            "NotifyEVChargingNeeds",
            notify("2026-09-30T11:00:00Z"),
        );
        let available = json!({
            "timestamp": "2026-09-30T10:00:00Z",
            "connectorStatus": "Available",
            "evseId": 1,
            "connectorId": 1
        });
        call(&config, &mut site, "StatusNotification", available);
        call(&config, &mut site, "TransactionEvent", started());
        assert_eq!(site.session("tx").unwrap().needs, None);
    }
}
