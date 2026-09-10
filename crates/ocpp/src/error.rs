//! OCPP-J error codes (OCPP 2.0.1 Part 4, "RPC Framework") and typed payload decoding.

use std::fmt;
use std::str::FromStr;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `errorCode` of a CALLERROR frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    /// Payload for the action is syntactically incorrect.
    FormatViolation,
    /// Any other error not covered by a more specific code.
    GenericError,
    /// The receiver hit an internal error while processing the action.
    InternalError,
    /// The message type number is not supported by this implementation.
    MessageTypeNotSupported,
    /// The requested action is not known by the receiver.
    NotImplemented,
    /// The requested action is recognized but not supported by the receiver.
    NotSupported,
    /// A field violates occurrence constraints (for example a required field is missing).
    OccurrenceConstraintViolation,
    /// A field contains an invalid value.
    PropertyConstraintViolation,
    /// The payload does not conform to the PDU structure.
    ProtocolError,
    /// The content is not a valid RPC request (for example the message id can't be read).
    RpcFrameworkError,
    /// A security issue prevented the receiver from completing the action.
    SecurityError,
    /// A field violates data type constraints (for example a string where a number is expected).
    TypeConstraintViolation,
}

impl ErrorCode {
    /// All error codes, in the order the specification lists them.
    pub const ALL: [ErrorCode; 12] = [
        Self::FormatViolation,
        Self::GenericError,
        Self::InternalError,
        Self::MessageTypeNotSupported,
        Self::NotImplemented,
        Self::NotSupported,
        Self::OccurrenceConstraintViolation,
        Self::PropertyConstraintViolation,
        Self::ProtocolError,
        Self::RpcFrameworkError,
        Self::SecurityError,
        Self::TypeConstraintViolation,
    ];

    /// The exact wire spelling of the code.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FormatViolation => "FormatViolation",
            Self::GenericError => "GenericError",
            Self::InternalError => "InternalError",
            Self::MessageTypeNotSupported => "MessageTypeNotSupported",
            Self::NotImplemented => "NotImplemented",
            Self::NotSupported => "NotSupported",
            Self::OccurrenceConstraintViolation => "OccurrenceConstraintViolation",
            Self::PropertyConstraintViolation => "PropertyConstraintViolation",
            Self::ProtocolError => "ProtocolError",
            Self::RpcFrameworkError => "RpcFrameworkError",
            Self::SecurityError => "SecurityError",
            Self::TypeConstraintViolation => "TypeConstraintViolation",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returned when a string is not one of the twelve OCPP error codes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown OCPP error code `{0}`")]
pub struct UnknownErrorCode(pub String);

impl FromStr for ErrorCode {
    type Err = UnknownErrorCode;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|code| code.as_str() == s)
            .ok_or_else(|| UnknownErrorCode(s.to_owned()))
    }
}

/// Prefix of the error raised when a string is longer than its schema's `maxLength`.
pub(crate) const TOO_LONG_PREFIX: &str = "string exceeds maxLength";
/// Prefix of the error raised when a `date-time` field is not valid RFC 3339.
pub(crate) const BAD_DATE_TIME_PREFIX: &str = "invalid date-time";

/// A payload that doesn't match its message type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct DecodeError {
    /// The error code to report back in a CALLERROR.
    pub code: ErrorCode,
    /// serde's description of the problem.
    pub message: String,
}

/// Decodes a CALL or CALLRESULT payload into its typed form.
pub fn decode<T: DeserializeOwned>(payload: Value) -> Result<T, DecodeError> {
    serde_json::from_value(payload).map_err(|e| {
        let message = e.to_string();
        DecodeError {
            code: classify(&message),
            message,
        }
    })
}

/// Maps a serde error message to the closest OCPP error code.
///
/// serde_json exposes no structured error kind, so this matches the stable message prefixes
/// of `serde::de::Error` plus the ones this crate raises itself. The unit tests pin every
/// mapping, so a wording change in serde shows up as a test failure.
fn classify(message: &str) -> ErrorCode {
    const RULES: &[(&str, ErrorCode)] = &[
        ("missing field", ErrorCode::OccurrenceConstraintViolation),
        ("duplicate field", ErrorCode::OccurrenceConstraintViolation),
        ("invalid length", ErrorCode::OccurrenceConstraintViolation),
        ("invalid type", ErrorCode::TypeConstraintViolation),
        ("unknown variant", ErrorCode::PropertyConstraintViolation),
        ("invalid value", ErrorCode::PropertyConstraintViolation),
        (TOO_LONG_PREFIX, ErrorCode::PropertyConstraintViolation),
        (BAD_DATE_TIME_PREFIX, ErrorCode::PropertyConstraintViolation),
    ];
    RULES
        .iter()
        .find(|(prefix, _)| message.starts_with(prefix))
        .map_or(ErrorCode::FormatViolation, |&(_, code)| code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serde_spelling_matches_as_str() {
        for code in ErrorCode::ALL {
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, code.as_str());
            assert_eq!(code.as_str().parse::<ErrorCode>().unwrap(), code);
        }
    }

    #[test]
    fn unknown_code_is_rejected() {
        assert!("FormationViolation".parse::<ErrorCode>().is_err());
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    #[allow(dead_code)]
    struct Probe {
        evse_id: i32,
        status: Status,
        #[serde(default)]
        tags: Option<[u8; 2]>,
    }

    #[derive(Debug, Deserialize)]
    enum Status {
        Accepted,
    }

    fn code_of(payload: Value) -> ErrorCode {
        decode::<Probe>(payload)
            .expect_err("payload should be rejected")
            .code
    }

    #[test]
    fn decodes_a_valid_payload() {
        assert!(decode::<Probe>(json!({"evseId": 1, "status": "Accepted"})).is_ok());
    }

    #[test]
    fn missing_field_is_occurrence_violation() {
        assert_eq!(
            code_of(json!({"status": "Accepted"})),
            ErrorCode::OccurrenceConstraintViolation
        );
    }

    #[test]
    fn wrong_array_length_is_occurrence_violation() {
        let payload = json!({"evseId": 1, "status": "Accepted", "tags": [1]});
        assert_eq!(code_of(payload), ErrorCode::OccurrenceConstraintViolation);
    }

    #[test]
    fn wrong_json_type_is_type_violation() {
        let payload = json!({"evseId": "one", "status": "Accepted"});
        assert_eq!(code_of(payload), ErrorCode::TypeConstraintViolation);
        let payload = json!({"evseId": 1.5, "status": "Accepted"});
        assert_eq!(code_of(payload), ErrorCode::TypeConstraintViolation);
    }

    #[test]
    fn bad_value_is_property_violation() {
        let payload = json!({"evseId": 1, "status": "Maybe"});
        assert_eq!(code_of(payload), ErrorCode::PropertyConstraintViolation);
        let payload = json!({"evseId": 3_000_000_000_u64, "status": "Accepted"});
        assert_eq!(code_of(payload), ErrorCode::PropertyConstraintViolation);
    }

    #[test]
    fn own_prefixes_are_property_violations() {
        assert_eq!(
            classify(&format!("{TOO_LONG_PREFIX} 20")),
            ErrorCode::PropertyConstraintViolation
        );
        assert_eq!(
            classify(&format!("{BAD_DATE_TIME_PREFIX}: x")),
            ErrorCode::PropertyConstraintViolation
        );
    }

    #[test]
    fn unknown_field_is_format_violation() {
        let payload = json!({"evseId": 1, "status": "Accepted", "extra": true});
        assert_eq!(code_of(payload), ErrorCode::FormatViolation);
        assert_eq!(classify("something else"), ErrorCode::FormatViolation);
    }
}
