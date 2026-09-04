//! OCPP-J error codes (OCPP 2.0.1 Part 4, "RPC Framework").

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
