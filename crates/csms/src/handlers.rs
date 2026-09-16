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
use ocpp::types::{AuthorizationStatus, IdTokenInfo};
use ocpp::{DateTime, ErrorCode, Frame, MessageId, StationRequest};
use serde_json::Value;
use tracing::info;

use crate::Config;
use crate::state::{StationInfo, StationState};

/// Answers one CALL from `station` with a CALLRESULT or a CALLERROR.
pub(crate) fn handle(
    config: &Config,
    station: &mut StationState,
    id: MessageId,
    action: &str,
    payload: Value,
) -> Frame {
    let request = match StationRequest::decode(action, payload) {
        Ok(request) => request,
        Err(e) => return Frame::call_error(id, e.code, e.message),
    };
    match request {
        StationRequest::BootNotification(req) => {
            Frame::call_result(id, &boot_notification(config, station, &req))
        }
        StationRequest::Heartbeat(_) => Frame::call_result(
            id,
            &HeartbeatResponse {
                custom_data: None,
                current_time: DateTime::now(),
            },
        ),
        StationRequest::StatusNotification(req) => {
            Frame::call_result(id, &status_notification(station, &req))
        }
        StationRequest::Authorize(req) => Frame::call_result(id, &authorize(config, &req)),
        StationRequest::TransactionEvent(_) | StationRequest::NotifyEvChargingNeeds(_) => {
            Frame::call_error(
                id,
                ErrorCode::NotSupported,
                format!("{} is not handled yet", request.action()),
            )
        }
    }
}

fn boot_notification(
    config: &Config,
    station: &mut StationState,
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
        current_time: DateTime::now(),
        interval: i32::try_from(config.heartbeat_interval.as_secs()).unwrap_or(i32::MAX),
        status: RegistrationStatus::Accepted,
        status_info: None,
    }
}

fn status_notification(
    station: &mut StationState,
    req: &StatusNotificationRequest,
) -> StatusNotificationResponse {
    station
        .evses
        .entry(req.evse_id)
        .or_default()
        .connectors
        .insert(req.connector_id, req.connector_status);
    StatusNotificationResponse::default()
}

fn authorize(config: &Config, req: &AuthorizeRequest) -> AuthorizeResponse {
    let token = req.id_token.id_token.as_str();
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
    AuthorizeResponse {
        custom_data: None,
        id_token_info: IdTokenInfo {
            custom_data: None,
            status,
            cache_expiry_date_time: None,
            charging_priority: None,
            language1: None,
            evse_id: None,
            group_id_token: None,
            language2: None,
            personal_message: None,
        },
        certificate_status: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocpp::messages::status_notification::ConnectorStatus;
    use serde_json::json;
    use std::collections::{BTreeMap, HashSet};
    use std::time::Duration;

    fn new_station() -> StationState {
        StationState {
            info: None,
            last_seen: DateTime::now(),
            evses: BTreeMap::new(),
        }
    }

    /// Runs one CALL and returns the CALLRESULT payload, failing on a CALLERROR.
    fn call(config: &Config, station: &mut StationState, action: &str, payload: Value) -> Value {
        let id = MessageId::try_from("1").unwrap();
        match handle(config, station, id, action, payload) {
            Frame::CallResult { payload, .. } => payload,
            other => panic!("expected a CALLRESULT, got {other:?}"),
        }
    }

    fn call_error(station: &mut StationState, action: &str, payload: Value) -> ErrorCode {
        let id = MessageId::try_from("1").unwrap();
        match handle(&Config::default(), station, id, action, payload) {
            Frame::CallError { code, .. } => code,
            other => panic!("expected a CALLERROR, got {other:?}"),
        }
    }

    fn authorize_status(config: &Config, token: &str) -> Value {
        let payload = json!({ "idToken": { "idToken": token, "type": "ISO14443" } });
        call(config, &mut new_station(), "Authorize", payload)["idTokenInfo"]["status"].clone()
    }

    #[test]
    fn boot_is_accepted_with_the_heartbeat_interval() {
        let config = Config {
            heartbeat_interval: Duration::from_secs(120),
            ..Config::default()
        };
        let mut station = new_station();
        let payload = json!({
            "chargingStation": {
                "model": "Wallbox 22",
                "vendorName": "Acme",
                "firmwareVersion": "1.2.3"
            },
            "reason": "PowerUp"
        });
        let response = call(&config, &mut station, "BootNotification", payload);
        assert_eq!(response["status"], "Accepted");
        assert_eq!(response["interval"], 120);
        assert!(response["currentTime"].is_string());
        assert_eq!(
            station.info,
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
        let response = call(
            &Config::default(),
            &mut new_station(),
            "Heartbeat",
            json!({}),
        );
        let time: DateTime = ocpp::decode(response["currentTime"].clone()).unwrap();
        assert!((*DateTime::now() - *time).num_seconds() < 5);
    }

    #[test]
    fn status_notification_tracks_connectors_per_evse() {
        let mut station = new_station();
        for (evse, connector, status) in
            [(1, 1, "Available"), (2, 1, "Occupied"), (1, 1, "Faulted")]
        {
            let payload = json!({
                "timestamp": "2026-09-30T08:00:00Z",
                "connectorStatus": status,
                "evseId": evse,
                "connectorId": connector
            });
            let response = call(
                &Config::default(),
                &mut station,
                "StatusNotification",
                payload,
            );
            assert_eq!(response, json!({}));
        }
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
    fn transaction_messages_are_not_handled_yet() {
        let fixtures = [
            (
                "TransactionEvent",
                include_str!("../../ocpp/tests/fixtures/TransactionEventRequest.started.json"),
            ),
            (
                "NotifyEVChargingNeeds",
                include_str!("../../ocpp/tests/fixtures/NotifyEVChargingNeedsRequest.dc.json"),
            ),
        ];
        for (action, fixture) in fixtures {
            let payload = serde_json::from_str(fixture).unwrap();
            assert_eq!(
                call_error(&mut new_station(), action, payload),
                ErrorCode::NotSupported
            );
        }
    }

    #[test]
    fn invalid_payloads_and_unknown_actions_get_call_errors() {
        let mut station = new_station();
        assert_eq!(
            call_error(&mut station, "StatusNotification", json!({ "evseId": 1 })),
            ErrorCode::OccurrenceConstraintViolation
        );
        assert_eq!(
            call_error(&mut station, "DataTransfer", json!({})),
            ErrorCode::NotImplemented
        );
        assert_eq!(
            call_error(&mut station, "SetChargingProfile", json!({})),
            ErrorCode::NotSupported
        );
    }
}
