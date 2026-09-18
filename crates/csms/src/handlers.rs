//! Answers to the CALLs a station sends.
//!
//! Stations don't have to send `BootNotification` before anything else: a station that
//! reconnects after a network drop carries on without booting again.

use ocpp::messages::AuthorizeRequest;
use ocpp::messages::authorize::AuthorizeResponse;
use ocpp::messages::boot_notification::{
    BootNotificationRequest, BootNotificationResponse, RegistrationStatus,
};
use ocpp::messages::heartbeat::HeartbeatResponse;
use ocpp::messages::status_notification::{StatusNotificationRequest, StatusNotificationResponse};
use ocpp::types::{AuthorizationStatus, IdToken, IdTokenInfo};
use ocpp::{DateTime, Frame, MessageId, StationRequest};
use serde_json::Value;
use tracing::info;

use ocpp::messages::status_notification::ConnectorStatus;
use optimizer::EvseRef;

use crate::state::{SiteState, StationInfo, StationState};
use crate::station_id::StationId;
use crate::{Config, charging_needs, transactions};

/// The answer to one CALL, and what it changed.
#[derive(Debug)]
pub(crate) struct Handled {
    /// The CALLRESULT or CALLERROR to send back.
    pub reply: Frame,
    /// Something changed that calls for a new plan.
    pub replan: bool,
    /// The station booted, so it needs its TxDefaultProfile.
    pub booted: bool,
}

impl Handled {
    fn reply(reply: Frame) -> Self {
        Self {
            reply,
            replan: false,
            booted: false,
        }
    }
}

/// Answers one CALL from `station`, received at `now`.
pub(crate) fn handle(
    config: &Config,
    site: &mut SiteState,
    station: &StationId,
    now: DateTime,
    id: MessageId,
    action: &str,
    payload: Value,
) -> Handled {
    let request = match StationRequest::decode(action, payload) {
        Ok(request) => request,
        Err(e) => return Handled::reply(Frame::call_error(id, e.code, e.message)),
    };
    match request {
        StationRequest::BootNotification(req) => {
            let response = boot_notification(config, site.touch(station, now), now, &req);
            Handled {
                booted: response.status == RegistrationStatus::Accepted,
                ..Handled::reply(Frame::call_result(id, &response))
            }
        }
        StationRequest::Heartbeat(_) => Handled::reply(Frame::call_result(
            id,
            &HeartbeatResponse {
                custom_data: None,
                current_time: now,
            },
        )),
        StationRequest::StatusNotification(req) => {
            let new_evse = status_notification(site.touch(station, now), &req);
            if req.connector_status == ConnectorStatus::Available {
                // The EV is gone; needs it reported must not stick to the next one.
                site.take_pending_needs(&EvseRef {
                    station_id: station.as_str().to_owned(),
                    evse_id: req.evse_id,
                });
            }
            Handled {
                replan: new_evse,
                ..Handled::reply(Frame::call_result(
                    id,
                    &StatusNotificationResponse::default(),
                ))
            }
        }
        StationRequest::Authorize(req) => {
            Handled::reply(Frame::call_result(id, &authorize(config, &req)))
        }
        StationRequest::TransactionEvent(req) => {
            let (response, replan) = transactions::transaction_event(config, site, station, &req);
            Handled {
                replan,
                ..Handled::reply(Frame::call_result(id, &response))
            }
        }
        StationRequest::NotifyEvChargingNeeds(req) => {
            let (response, replan) =
                charging_needs::notify_ev_charging_needs(&config.site, site, station, &req);
            Handled {
                replan,
                ..Handled::reply(Frame::call_result(id, &response))
            }
        }
    }
}

fn boot_notification(
    config: &Config,
    station: &mut StationState,
    now: DateTime,
    req: &BootNotificationRequest,
) -> BootNotificationResponse {
    let info = StationInfo::from(&req.charging_station);
    info!(
        vendor = info.vendor,
        model = info.model,
        reason = ?req.reason,
        "station booted"
    );
    station.info = Some(info);
    BootNotificationResponse {
        custom_data: None,
        current_time: now,
        interval: i32::try_from(config.heartbeat_interval.as_secs()).unwrap_or(i32::MAX),
        status: RegistrationStatus::Accepted,
        status_info: None,
    }
}

/// Records a connector's status. Returns whether the EVSE is new: each idle EVSE has
/// capacity reserved for it, so the plan changes.
fn status_notification(station: &mut StationState, req: &StatusNotificationRequest) -> bool {
    let new_evse = !station.evses.contains_key(&req.evse_id);
    station
        .evses
        .entry(req.evse_id)
        .or_default()
        .connectors
        .insert(req.connector_id, req.connector_status);
    new_evse
}

fn authorize(config: &Config, req: &AuthorizeRequest) -> AuthorizeResponse {
    AuthorizeResponse {
        custom_data: None,
        id_token_info: id_token_info(config, &req.id_token),
        certificate_status: None,
    }
}

/// Accepts a token if there is no allowlist or it is on it.
pub(crate) fn id_token_info(config: &Config, id_token: &IdToken) -> IdTokenInfo {
    let token = id_token.id_token.as_str();
    let accepted = config
        .id_token_allowlist
        .as_ref()
        .is_none_or(|allowed| allowed.contains(token));
    let status = if accepted {
        AuthorizationStatus::Accepted
    } else {
        info!(token, "id token not on the allowlist");
        AuthorizationStatus::Invalid
    };
    IdTokenInfo {
        custom_data: None,
        status,
        cache_expiry_date_time: None,
        charging_priority: None,
        language1: None,
        evse_id: None,
        group_id_token: None,
        language2: None,
        personal_message: None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocpp::ErrorCode;
    use serde_json::json;
    use std::collections::HashSet;
    use std::time::Duration;

    pub(crate) fn new_site() -> SiteState {
        SiteState::new(100_000.0)
    }

    pub(crate) fn cs1() -> StationId {
        StationId::try_from("CS1").unwrap()
    }

    /// Runs one CALL from CS1.
    pub(crate) fn run(
        config: &Config,
        site: &mut SiteState,
        action: &str,
        payload: Value,
    ) -> Handled {
        let id = MessageId::try_from("1").unwrap();
        handle(config, site, &cs1(), DateTime::now(), id, action, payload)
    }

    /// Runs one CALL and returns the CALLRESULT payload, failing on a CALLERROR.
    pub(crate) fn call(
        config: &Config,
        site: &mut SiteState,
        action: &str,
        payload: Value,
    ) -> Value {
        match run(config, site, action, payload).reply {
            Frame::CallResult { payload, .. } => payload,
            other => panic!("expected a CALLRESULT, got {other:?}"),
        }
    }

    fn call_error(site: &mut SiteState, action: &str, payload: Value) -> ErrorCode {
        match run(&Config::default(), site, action, payload).reply {
            Frame::CallError { code, .. } => code,
            other => panic!("expected a CALLERROR, got {other:?}"),
        }
    }

    fn authorize_status(config: &Config, token: &str) -> Value {
        let payload = json!({ "idToken": { "idToken": token, "type": "ISO14443" } });
        call(config, &mut new_site(), "Authorize", payload)["idTokenInfo"]["status"].clone()
    }

    #[test]
    fn boot_is_accepted_with_the_heartbeat_interval() {
        let config = Config {
            heartbeat_interval: Duration::from_secs(120),
            ..Config::default()
        };
        let mut site = new_site();
        let payload = json!({
            "chargingStation": {
                "model": "Wallbox 22",
                "vendorName": "Acme",
                "firmwareVersion": "1.2.3"
            },
            "reason": "PowerUp"
        });
        let handled = run(&config, &mut site, "BootNotification", payload.clone());
        assert!(handled.booted);
        let response = call(&config, &mut site, "BootNotification", payload);
        assert_eq!(response["status"], "Accepted");
        assert_eq!(response["interval"], 120);
        assert!(response["currentTime"].is_string());
        assert_eq!(
            site.station(&cs1()).unwrap().info,
            Some(StationInfo {
                vendor: "Acme".into(),
                model: "Wallbox 22".into(),
                serial_number: None,
                firmware_version: Some("1.2.3".into()),
            })
        );
    }

    #[test]
    fn heartbeat_returns_the_current_time() {
        let response = call(&Config::default(), &mut new_site(), "Heartbeat", json!({}));
        let time: DateTime = ocpp::decode(response["currentTime"].clone()).unwrap();
        assert!((*DateTime::now() - *time).num_seconds() < 5);
    }

    #[test]
    fn status_notification_tracks_connectors_per_evse() {
        let mut site = new_site();
        for (evse, connector, status, new_evse) in [
            (1, 1, "Available", true),
            (2, 1, "Occupied", true),
            (1, 1, "Faulted", false),
        ] {
            let payload = json!({
                "timestamp": "2026-09-30T08:00:00Z",
                "connectorStatus": status,
                "evseId": evse,
                "connectorId": connector
            });
            let handled = run(&Config::default(), &mut site, "StatusNotification", payload);
            assert!(
                matches!(&handled.reply, Frame::CallResult { payload, .. } if *payload == json!({}))
            );
            // Each new EVSE gets capacity reserved, so the plan must change.
            assert_eq!(handled.replan, new_evse);
        }
        let station = site.station(&cs1()).unwrap();
        assert_eq!(station.evses[&1].connectors[&1], ConnectorStatus::Faulted);
        assert_eq!(station.evses[&2].connectors[&1], ConnectorStatus::Occupied);
    }

    #[test]
    fn authorize_accepts_everyone_without_an_allowlist() {
        assert_eq!(authorize_status(&Config::default(), "ANY-TAG"), "Accepted");
    }

    #[test]
    fn authorize_checks_the_allowlist() {
        let config = Config {
            id_token_allowlist: Some(HashSet::from(["GOOD".to_owned()])),
            ..Config::default()
        };
        assert_eq!(authorize_status(&config, "GOOD"), "Accepted");
        assert_eq!(authorize_status(&config, "BAD"), "Invalid");
    }

    #[test]
    fn invalid_payloads_and_unknown_actions_get_call_errors() {
        let mut site = new_site();
        assert_eq!(
            call_error(&mut site, "StatusNotification", json!({ "evseId": 1 })),
            ErrorCode::OccurrenceConstraintViolation
        );
        assert_eq!(
            call_error(&mut site, "DataTransfer", json!({})),
            ErrorCode::NotImplemented
        );
        assert_eq!(
            call_error(&mut site, "SetChargingProfile", json!({})),
            ErrorCode::NotSupported
        );
    }
}
