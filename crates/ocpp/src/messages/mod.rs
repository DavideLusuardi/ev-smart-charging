//! Typed requests and responses for the smart-charging subset of OCPP 2.0.1.
//!
//! Each action has a marker type implementing [`Action`], which ties its wire name to its
//! request and response payloads. [`StationRequest`] and [`CsmsRequest`] decode an incoming
//! CALL for the side that receives it.

pub mod authorize;
pub mod boot_notification;
pub mod clear_charging_profile;
pub mod heartbeat;
pub mod notify_ev_charging_needs;
pub mod set_charging_profile;
pub mod status_notification;
pub mod transaction_event;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{DecodeError, ErrorCode, decode};
use crate::rpc::{Frame, MessageId};

pub use authorize::{AuthorizeRequest, AuthorizeResponse};
pub use boot_notification::{BootNotificationRequest, BootNotificationResponse};
pub use clear_charging_profile::{ClearChargingProfileRequest, ClearChargingProfileResponse};
pub use heartbeat::{HeartbeatRequest, HeartbeatResponse};
pub use notify_ev_charging_needs::{NotifyEvChargingNeedsRequest, NotifyEvChargingNeedsResponse};
pub use set_charging_profile::{SetChargingProfileRequest, SetChargingProfileResponse};
pub use status_notification::{StatusNotificationRequest, StatusNotificationResponse};
pub use transaction_event::{TransactionEventRequest, TransactionEventResponse};

/// An OCPP action: its wire name and the payload types of its CALL and CALLRESULT.
pub trait Action {
    /// The action name as it appears in a CALL frame.
    const NAME: &'static str;
    /// The CALL payload.
    type Request: Serialize + DeserializeOwned;
    /// The CALLRESULT payload.
    type Response: Serialize + DeserializeOwned;
}

macro_rules! actions {
    ($($(#[$doc:meta])* $marker:ident = $name:literal ($req:ty, $resp:ty);)*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $marker;

            impl Action for $marker {
                const NAME: &'static str = $name;
                type Request = $req;
                type Response = $resp;
            }
        )*
    };
}

actions! {
    /// Station → CSMS.
    Authorize = "Authorize" (AuthorizeRequest, AuthorizeResponse);
    /// Station → CSMS.
    BootNotification = "BootNotification" (BootNotificationRequest, BootNotificationResponse);
    /// Station → CSMS.
    Heartbeat = "Heartbeat" (HeartbeatRequest, HeartbeatResponse);
    /// Station → CSMS.
    NotifyEvChargingNeeds = "NotifyEVChargingNeeds"
        (NotifyEvChargingNeedsRequest, NotifyEvChargingNeedsResponse);
    /// Station → CSMS.
    StatusNotification = "StatusNotification"
        (StatusNotificationRequest, StatusNotificationResponse);
    /// Station → CSMS.
    TransactionEvent = "TransactionEvent" (TransactionEventRequest, TransactionEventResponse);
    /// CSMS → station.
    ClearChargingProfile = "ClearChargingProfile"
        (ClearChargingProfileRequest, ClearChargingProfileResponse);
    /// CSMS → station.
    SetChargingProfile = "SetChargingProfile"
        (SetChargingProfileRequest, SetChargingProfileResponse);
}

/// Serializes a payload. Payload types contain only JSON-representable data, so this can't fail.
pub fn encode<T: Serialize>(payload: &T) -> Value {
    serde_json::to_value(payload).expect("OCPP payloads always serialize")
}

impl Frame {
    /// A CALL for action `A`.
    pub fn call<A: Action>(id: MessageId, request: &A::Request) -> Self {
        Self::Call {
            id,
            action: A::NAME.to_owned(),
            payload: encode(request),
        }
    }

    /// A CALLRESULT answering the CALL with `id`.
    pub fn call_result<T: Serialize>(id: MessageId, response: &T) -> Self {
        Self::CallResult {
            id,
            payload: encode(response),
        }
    }
}

/// A CALL a station sends and the CSMS handles.
#[derive(Debug, Clone, PartialEq)]
pub enum StationRequest {
    Authorize(AuthorizeRequest),
    BootNotification(BootNotificationRequest),
    Heartbeat(HeartbeatRequest),
    NotifyEvChargingNeeds(NotifyEvChargingNeedsRequest),
    StatusNotification(StatusNotificationRequest),
    TransactionEvent(TransactionEventRequest),
}

impl StationRequest {
    /// Decodes the payload of an incoming CALL.
    ///
    /// Actions the CSMS sends itself are `NotSupported`; any other unknown action is
    /// `NotImplemented`.
    pub fn decode(action: &str, payload: Value) -> Result<Self, DecodeError> {
        Ok(match action {
            Authorize::NAME => Self::Authorize(decode(payload)?),
            BootNotification::NAME => Self::BootNotification(decode(payload)?),
            Heartbeat::NAME => Self::Heartbeat(decode(payload)?),
            NotifyEvChargingNeeds::NAME => Self::NotifyEvChargingNeeds(decode(payload)?),
            StatusNotification::NAME => Self::StatusNotification(decode(payload)?),
            TransactionEvent::NAME => Self::TransactionEvent(decode(payload)?),
            ClearChargingProfile::NAME | SetChargingProfile::NAME => {
                return Err(wrong_direction(action));
            }
            _ => return Err(not_implemented(action)),
        })
    }

    /// The action name of this request.
    pub fn action(&self) -> &'static str {
        match self {
            Self::Authorize(_) => Authorize::NAME,
            Self::BootNotification(_) => BootNotification::NAME,
            Self::Heartbeat(_) => Heartbeat::NAME,
            Self::NotifyEvChargingNeeds(_) => NotifyEvChargingNeeds::NAME,
            Self::StatusNotification(_) => StatusNotification::NAME,
            Self::TransactionEvent(_) => TransactionEvent::NAME,
        }
    }
}

/// A CALL the CSMS sends and a station handles.
#[derive(Debug, Clone, PartialEq)]
pub enum CsmsRequest {
    ClearChargingProfile(ClearChargingProfileRequest),
    SetChargingProfile(SetChargingProfileRequest),
}

impl CsmsRequest {
    /// Decodes the payload of an incoming CALL.
    ///
    /// Actions a station sends itself are `NotSupported`; any other unknown action is
    /// `NotImplemented`.
    pub fn decode(action: &str, payload: Value) -> Result<Self, DecodeError> {
        Ok(match action {
            ClearChargingProfile::NAME => Self::ClearChargingProfile(decode(payload)?),
            SetChargingProfile::NAME => Self::SetChargingProfile(decode(payload)?),
            Authorize::NAME
            | BootNotification::NAME
            | Heartbeat::NAME
            | NotifyEvChargingNeeds::NAME
            | StatusNotification::NAME
            | TransactionEvent::NAME => return Err(wrong_direction(action)),
            _ => return Err(not_implemented(action)),
        })
    }

    /// The action name of this request.
    pub fn action(&self) -> &'static str {
        match self {
            Self::ClearChargingProfile(_) => ClearChargingProfile::NAME,
            Self::SetChargingProfile(_) => SetChargingProfile::NAME,
        }
    }
}

fn wrong_direction(action: &str) -> DecodeError {
    DecodeError {
        code: ErrorCode::NotSupported,
        message: format!("{action} is not accepted in this direction"),
    }
}

fn not_implemented(action: &str) -> DecodeError {
    DecodeError {
        code: ErrorCode::NotImplemented,
        message: format!("action {action} is not implemented"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datetime::DateTime;
    use serde_json::json;

    #[test]
    fn decodes_station_requests() {
        let req = StationRequest::decode("Heartbeat", json!({})).unwrap();
        assert_eq!(req, StationRequest::Heartbeat(HeartbeatRequest::default()));
        assert_eq!(req.action(), "Heartbeat");

        let status = json!({
            "timestamp": "2026-09-28T10:00:00Z",
            "connectorStatus": "Occupied",
            "evseId": 1,
            "connectorId": 1
        });
        let req = StationRequest::decode("StatusNotification", status).unwrap();
        assert_eq!(req.action(), "StatusNotification");
    }

    #[test]
    fn unknown_actions_are_not_implemented() {
        let err = StationRequest::decode("DataTransfer", json!({})).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        let err = CsmsRequest::decode("Reset", json!({})).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }

    #[test]
    fn wrong_direction_is_not_supported() {
        let err = StationRequest::decode("SetChargingProfile", json!({})).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
        let err = CsmsRequest::decode("BootNotification", json!({})).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
    }

    #[test]
    fn payload_errors_keep_their_classification() {
        let err = StationRequest::decode("StatusNotification", json!({"evseId": 1})).unwrap_err();
        assert_eq!(err.code, ErrorCode::OccurrenceConstraintViolation);
    }

    #[test]
    fn builds_typed_frames() {
        let id = MessageId::try_from("1").unwrap();
        let call = Frame::call::<Heartbeat>(id.clone(), &HeartbeatRequest::default());
        assert_eq!(call.to_json(), r#"[2,"1","Heartbeat",{}]"#);

        let time: DateTime = decode(json!("2026-09-28T10:00:00Z")).unwrap();
        let response = HeartbeatResponse {
            custom_data: None,
            current_time: time,
        };
        let result = Frame::call_result(id, &response);
        assert_eq!(
            result.to_json(),
            r#"[3,"1",{"currentTime":"2026-09-28T10:00:00Z"}]"#
        );
    }
}
