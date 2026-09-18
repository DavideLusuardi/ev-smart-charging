//! `TransactionEvent`: sessions starting, progressing and ending.
//!
//! A transaction becomes a planned session once the CSMS knows its EVSE. Meter values give
//! the energy delivered, counted from the first register reading, and the power drawn, from
//! which the EV's own max power is learned.

use ocpp::messages::transaction_event::{
    ChargingState, TransactionEventKind, TransactionEventRequest, TransactionEventResponse,
};
use optimizer::{EvseRef, observe_draw};
use tracing::{debug, info, warn};

use crate::Config;
use crate::handlers::id_token_info;
use crate::meter;
use crate::state::{Session, SiteState};
use crate::station_id::StationId;

/// Applies one `TransactionEvent` from `station`. Returns the response, and whether the
/// change calls for a new plan.
pub(crate) fn transaction_event(
    config: &Config,
    site: &mut SiteState,
    station: &StationId,
    req: &TransactionEventRequest,
) -> (TransactionEventResponse, bool) {
    let tx = req.transaction_info.transaction_id.as_str();
    let replan = match req.event_type {
        TransactionEventKind::Ended => {
            let ended = site.end_session(tx);
            match &ended {
                Some(s) => {
                    info!(tx, evse = %s.evse, delivered_wh = s.delivered_wh, "session ended")
                }
                None => debug!(tx, "end of a transaction the CSMS didn't track"),
            }
            ended.is_some()
        }
        TransactionEventKind::Started | TransactionEventKind::Updated => {
            let started = site.session(tx).is_none() && start(config, site, station, req);
            let changed = site.session(tx).is_some() && update(config, site, req);
            started || changed
        }
    };
    let response = TransactionEventResponse {
        // TODO(spec): OCPP 2.0.1 requires idTokenInfo when the request has an idToken, as we
        // read the TransactionEventResponse description; confirm it.
        id_token_info: req.id_token.as_ref().map(|t| id_token_info(config, t)),
        ..TransactionEventResponse::default()
    };
    (response, replan)
}

/// Starts tracking a transaction. One the CSMS first hears of in an `Updated` event (it
/// restarted, or missed the `Started`) is adopted, from now. Returns whether a session was
/// added; without an EVSE there is nothing to plan.
fn start(
    config: &Config,
    site: &mut SiteState,
    station: &StationId,
    req: &TransactionEventRequest,
) -> bool {
    let tx = req.transaction_info.transaction_id.as_str();
    let Some(evse) = &req.evse else {
        warn!(
            tx,
            "transaction without an EVSE, not planned until one is reported"
        );
        return false;
    };
    let evse = EvseRef {
        station_id: station.as_str().to_owned(),
        evse_id: evse.id,
    };
    if req.event_type == TransactionEventKind::Updated {
        info!(tx, %evse, "adopting a transaction already in progress");
    } else {
        info!(tx, %evse, "session started");
    }
    let mut session = Session::new(tx, evse, req.timestamp);
    if let Some(needs) = site.take_pending_needs(&session.evse) {
        session.needs =
            Some(needs.resolve(session.started.timestamp(), &config.site.needs_defaults));
    }
    site.start_session(session);
    true
}

/// Applies the charging state and meter values. Returns whether the EV started or stopped
/// being full, which changes what it needs.
fn update(config: &Config, site: &mut SiteState, req: &TransactionEventRequest) -> bool {
    let tx = req.transaction_info.transaction_id.as_str();
    let reading = req.meter_value.as_deref().and_then(meter::latest);
    // The limit the EV was under when the reading was taken, before borrowing the session.
    let limit_w = reading.and_then(|r| {
        let session = site.session(tx)?;
        let limit = site.limit_at(
            &session.evse,
            r.timestamp.timestamp(),
            config.site.fallback_limit_w,
        );
        Some(limit.min(config.site.evse_max_w))
    });
    let session = site.session_mut(tx).expect("the session exists");

    let mut replan = false;
    if let Some(state) = req.transaction_info.charging_state {
        session.charging_state = Some(state);
        // SuspendedEVSE is the station obeying a 0 W limit; it says nothing about the EV.
        let suspended = match state {
            ChargingState::SuspendedEv => Some(true),
            ChargingState::Charging => Some(false),
            _ => None,
        };
        if let Some(suspended) = suspended
            && suspended != session.ev_suspended
        {
            session.ev_suspended = suspended;
            replan = true;
        }
    }

    if let Some(reading) = reading {
        if let Some(energy_wh) = reading.energy_wh {
            let start_wh = *session.meter_start_wh.get_or_insert(energy_wh);
            session.delivered_wh = (energy_wh - start_wh).max(0.0);
        }
        if let Some(power_w) = reading.power_w {
            session.power_w = Some(power_w);
            // Only a charging EV shows what it can take: one that is merely plugged in
            // draws nothing, and would be learned as taking 0 W.
            let charging = match session.charging_state {
                Some(state) => state == ChargingState::Charging,
                None => power_w > 0.0,
            };
            if charging && let Some(limit_w) = limit_w {
                session.observed_max_w = observe_draw(session.observed_max_w, limit_w, power_w);
            }
        }
    }
    replan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::tests::{call, new_site, run};
    use optimizer::StepProfile;
    use serde_json::{Value, json};

    const TX: &str = "tx-1";

    fn evse1() -> EvseRef {
        EvseRef {
            station_id: "CS1".into(),
            evse_id: 1,
        }
    }

    fn event(kind: &str, extra: Value) -> Value {
        let mut e = json!({
            "eventType": kind,
            "timestamp": "2026-09-30T10:00:00Z",
            "triggerReason": "CablePluggedIn",
            "seqNo": 0,
            "transactionInfo": { "transactionId": TX }
        });
        e.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        e
    }

    fn meter(time: &str, energy_wh: f64, power_w: f64) -> Value {
        json!([{
            "timestamp": time,
            "sampledValue": [
                { "value": energy_wh, "measurand": "Energy.Active.Import.Register" },
                { "value": power_w, "measurand": "Power.Active.Import" }
            ]
        }])
    }

    fn charging(time: &str, energy_wh: f64, power_w: f64) -> Value {
        event(
            "Updated",
            json!({
                "timestamp": time,
                "transactionInfo": { "transactionId": TX, "chargingState": "Charging" },
                "meterValue": meter(time, energy_wh, power_w)
            }),
        )
    }

    /// Sends the event and returns whether it asked for a new plan.
    fn send(site: &mut SiteState, payload: Value) -> bool {
        let handled = run(&Config::default(), site, "TransactionEvent", payload);
        assert!(
            matches!(handled.reply, ocpp::Frame::CallResult { .. }),
            "{:?}",
            handled.reply
        );
        handled.replan
    }

    fn started(site: &mut SiteState) {
        let start = event(
            "Started",
            json!({
                "evse": { "id": 1, "connectorId": 1 },
                "meterValue": meter("2026-09-30T10:00:00Z", 1_000.0, 0.0)
            }),
        );
        assert!(send(site, start));
    }

    #[test]
    fn a_session_runs_from_started_to_ended() {
        let mut site = new_site();
        started(&mut site);
        let s = site.session(TX).unwrap();
        assert_eq!(s.evse, evse1());
        assert_eq!(s.started.to_string(), "2026-09-30T10:00:00Z");
        assert_eq!(s.meter_start_wh, Some(1_000.0));

        // Meter updates alone don't call for a new plan: the timer takes care of them.
        assert!(!send(
            &mut site,
            charging("2026-09-30T10:30:00Z", 6_500.0, 11_000.0)
        ));
        let s = site.session(TX).unwrap();
        assert_eq!(s.delivered_wh, 5_500.0);
        assert_eq!(s.power_w, Some(11_000.0));

        site.set_effective(evse1(), StepProfile::constant(0, 11_000.0));
        assert!(send(
            &mut site,
            event("Ended", json!({ "triggerReason": "EVDeparted" }))
        ));
        assert!(site.session(TX).is_none());
        assert!(site.effective_profiles().is_empty());
        // An end the CSMS didn't track changes nothing.
        assert!(!send(&mut site, event("Ended", json!({}))));
    }

    #[test]
    fn transactions_without_an_evse_are_not_planned() {
        let mut site = new_site();
        assert!(!send(&mut site, event("Started", json!({}))));
        assert!(site.session(TX).is_none());
        // The EVSE may come in a later event.
        let update = event("Updated", json!({ "evse": { "id": 1 } }));
        assert!(send(&mut site, update));
        assert_eq!(site.session(TX).unwrap().evse, evse1());
    }

    #[test]
    fn an_unknown_transaction_in_progress_is_adopted() {
        let mut site = new_site();
        let update = event(
            "Updated",
            json!({
                "timestamp": "2026-09-30T11:00:00Z",
                "evse": { "id": 1 },
                "meterValue": meter("2026-09-30T11:00:00Z", 20_000.0, 7_000.0)
            }),
        );
        assert!(send(&mut site, update));
        let s = site.session(TX).unwrap();
        // Delivered energy counts from the first reading the CSMS sees.
        assert_eq!(s.started.to_string(), "2026-09-30T11:00:00Z");
        assert_eq!(s.meter_start_wh, Some(20_000.0));
        assert_eq!(s.delivered_wh, 0.0);
    }

    #[test]
    fn a_full_ev_is_suspended_until_it_charges_again() {
        let mut site = new_site();
        started(&mut site);
        let state = |s: &str| {
            event(
                "Updated",
                json!({ "transactionInfo": { "transactionId": TX, "chargingState": s } }),
            )
        };
        assert!(send(&mut site, state("SuspendedEV")));
        assert!(site.session(TX).unwrap().ev_suspended);
        assert!(!send(&mut site, state("SuspendedEV")));
        // Held at 0 W by the CSMS: says nothing about the EV.
        assert!(!send(&mut site, state("SuspendedEVSE")));
        assert!(site.session(TX).unwrap().ev_suspended);
        assert!(send(&mut site, state("Charging")));
        assert!(!site.session(TX).unwrap().ev_suspended);
    }

    #[test]
    fn the_ev_max_is_learned_from_draws_below_the_limit() {
        let mut site = new_site();
        started(&mut site);
        site.set_effective(evse1(), StepProfile::constant(0, 22_000.0));
        send(
            &mut site,
            charging("2026-09-30T10:10:00Z", 2_000.0, 11_000.0),
        );
        assert_eq!(site.session(TX).unwrap().observed_max_w, Some(11_000.0));
    }

    #[test]
    fn an_ev_that_is_not_charging_teaches_nothing() {
        let mut site = new_site();
        started(&mut site);
        site.set_effective(evse1(), StepProfile::constant(0, 22_000.0));
        let connected = event(
            "Updated",
            json!({
                "transactionInfo": { "transactionId": TX, "chargingState": "EVConnected" },
                "meterValue": meter("2026-09-30T10:01:00Z", 1_000.0, 0.0)
            }),
        );
        send(&mut site, connected);
        assert_eq!(site.session(TX).unwrap().observed_max_w, None);
    }

    #[test]
    fn id_tokens_are_checked_in_the_response() {
        let config = Config {
            id_token_allowlist: Some(["GOOD".to_owned()].into()),
            ..Config::default()
        };
        let with_token = |token: &str| {
            event(
                "Started",
                json!({ "evse": { "id": 1 }, "idToken": { "idToken": token, "type": "ISO14443" } }),
            )
        };
        let mut site = new_site();
        let response = call(&config, &mut site, "TransactionEvent", with_token("BAD"));
        assert_eq!(response["idTokenInfo"]["status"], "Invalid");
        let response = call(
            &config,
            &mut site,
            "TransactionEvent",
            event("Updated", json!({})),
        );
        assert_eq!(response, json!({}));
    }
}
