//! OCPP-J RPC framing (OCPP 2.0.1 Part 4): CALL, CALLRESULT and CALLERROR frames.
//!
//! Payloads stay as untyped [`Value`]s here. A CALLRESULT doesn't name its action, so the
//! typed payload can only be decoded once the caller has matched it to the pending CALL.

use std::fmt;

use serde::ser::{Serialize, SerializeSeq, Serializer};
use serde_json::{Map, Value};

use crate::error::ErrorCode;

/// Longest message id the specification allows.
pub const MAX_MESSAGE_ID_LEN: usize = 36;

/// The message type number in the first position of every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageType {
    /// `2`: a request.
    Call,
    /// `3`: a successful response.
    CallResult,
    /// `4`: an error response.
    CallError,
}

impl MessageType {
    /// The number used on the wire.
    pub fn type_id(self) -> u64 {
        match self {
            Self::Call => 2,
            Self::CallResult => 3,
            Self::CallError => 4,
        }
    }

    fn from_type_id(id: u64) -> Option<Self> {
        match id {
            2 => Some(Self::Call),
            3 => Some(Self::CallResult),
            4 => Some(Self::CallError),
            _ => None,
        }
    }

    /// Number of elements in a frame of this type.
    fn arity(self) -> usize {
        match self {
            Self::Call => 4,
            Self::CallResult => 3,
            Self::CallError => 5,
        }
    }
}

/// A message id: 1 to 36 characters, unique per outstanding CALL on a connection.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageId(String);

impl MessageId {
    /// A fresh random id (UUID v4, exactly 36 characters).
    pub fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Returned when a message id is empty or longer than [`MAX_MESSAGE_ID_LEN`] characters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("message id must be 1 to {MAX_MESSAGE_ID_LEN} characters, got {0}")]
pub struct InvalidMessageId(pub usize);

impl TryFrom<String> for MessageId {
    type Error = InvalidMessageId;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        let len = id.chars().count();
        if (1..=MAX_MESSAGE_ID_LEN).contains(&len) {
            Ok(Self(id))
        } else {
            Err(InvalidMessageId(len))
        }
    }
}

impl TryFrom<&str> for MessageId {
    type Error = InvalidMessageId;

    fn try_from(id: &str) -> Result<Self, Self::Error> {
        Self::try_from(id.to_owned())
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// One OCPP-J frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// `[2, id, action, payload]`
    Call {
        id: MessageId,
        action: String,
        payload: Value,
    },
    /// `[3, id, payload]`
    CallResult { id: MessageId, payload: Value },
    /// `[4, id, errorCode, errorDescription, errorDetails]`
    CallError {
        id: MessageId,
        code: ErrorCode,
        description: String,
        details: Value,
    },
}

impl Frame {
    /// A CALLERROR with an empty details object.
    pub fn call_error(id: MessageId, code: ErrorCode, description: impl Into<String>) -> Self {
        Self::CallError {
            id,
            code,
            description: description.into(),
            details: Value::Object(Map::new()),
        }
    }

    /// The frame's message id.
    pub fn id(&self) -> &MessageId {
        match self {
            Self::Call { id, .. } | Self::CallResult { id, .. } | Self::CallError { id, .. } => id,
        }
    }

    /// The frame's message type.
    pub fn message_type(&self) -> MessageType {
        match self {
            Self::Call { .. } => MessageType::Call,
            Self::CallResult { .. } => MessageType::CallResult,
            Self::CallError { .. } => MessageType::CallError,
        }
    }

    /// Parses one WebSocket text message into a frame.
    pub fn parse(text: &str) -> Result<Self, FrameError> {
        let value: Value = serde_json::from_str(text).map_err(|e| {
            FrameError::new(
                None,
                None,
                ErrorCode::RpcFrameworkError,
                format!("not valid JSON: {e}"),
            )
        })?;
        let Value::Array(mut items) = value else {
            return Err(FrameError::new(
                None,
                None,
                ErrorCode::RpcFrameworkError,
                "frame is not a JSON array",
            ));
        };

        // Read the id first so every later error can still be answered with it.
        let id = match items.get(1) {
            Some(Value::String(s)) => MessageId::try_from(s.as_str()).ok(),
            _ => None,
        };

        let type_id = items.first().and_then(Value::as_u64).ok_or_else(|| {
            FrameError::new(
                None,
                id.clone(),
                ErrorCode::RpcFrameworkError,
                "first element is not a message type number",
            )
        })?;
        let Some(kind) = MessageType::from_type_id(type_id) else {
            return Err(FrameError::new(
                None,
                id,
                ErrorCode::MessageTypeNotSupported,
                format!("message type {type_id} is not supported"),
            ));
        };
        let err =
            |id: Option<MessageId>, code, msg: String| FrameError::new(Some(kind), id, code, msg);

        if items.len() != kind.arity() {
            return Err(err(
                id,
                ErrorCode::RpcFrameworkError,
                format!("expected {} elements, got {}", kind.arity(), items.len()),
            ));
        }
        let Some(id) = id else {
            let msg = match &items[1] {
                Value::String(s) => InvalidMessageId(s.chars().count()).to_string(),
                _ => "message id is not a string".to_owned(),
            };
            return Err(err(None, ErrorCode::RpcFrameworkError, msg));
        };

        match kind {
            MessageType::Call => {
                let payload = items.pop().expect("arity checked");
                let Value::String(action) = items.pop().expect("arity checked") else {
                    return Err(err(
                        Some(id),
                        ErrorCode::RpcFrameworkError,
                        "action is not a string".to_owned(),
                    ));
                };
                if !payload.is_object() {
                    return Err(err(
                        Some(id),
                        ErrorCode::FormatViolation,
                        "payload is not a JSON object".to_owned(),
                    ));
                }
                Ok(Self::Call {
                    id,
                    action,
                    payload,
                })
            }
            MessageType::CallResult => {
                let payload = items.pop().expect("arity checked");
                if !payload.is_object() {
                    return Err(err(
                        Some(id),
                        ErrorCode::FormatViolation,
                        "payload is not a JSON object".to_owned(),
                    ));
                }
                Ok(Self::CallResult { id, payload })
            }
            MessageType::CallError => {
                let details = items.pop().expect("arity checked");
                let description = items.pop().expect("arity checked");
                let code = items.pop().expect("arity checked");
                let code = code
                    .as_str()
                    .and_then(|c| c.parse::<ErrorCode>().ok())
                    .ok_or_else(|| {
                        err(
                            Some(id.clone()),
                            ErrorCode::RpcFrameworkError,
                            format!("unknown error code {code}"),
                        )
                    })?;
                let Value::String(description) = description else {
                    return Err(err(
                        Some(id),
                        ErrorCode::RpcFrameworkError,
                        "error description is not a string".to_owned(),
                    ));
                };
                if !details.is_object() {
                    return Err(err(
                        Some(id),
                        ErrorCode::RpcFrameworkError,
                        "error details is not a JSON object".to_owned(),
                    ));
                }
                Ok(Self::CallError {
                    id,
                    code,
                    description,
                    details,
                })
            }
        }
    }

    /// Serializes the frame to its JSON text form.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("frames always serialize")
    }
}

impl Serialize for Frame {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.message_type().arity()))?;
        seq.serialize_element(&self.message_type().type_id())?;
        seq.serialize_element(self.id())?;
        match self {
            Self::Call {
                action, payload, ..
            } => {
                seq.serialize_element(action)?;
                seq.serialize_element(payload)?;
            }
            Self::CallResult { payload, .. } => seq.serialize_element(payload)?,
            Self::CallError {
                code,
                description,
                details,
                ..
            } => {
                seq.serialize_element(code)?;
                seq.serialize_element(description)?;
                seq.serialize_element(details)?;
            }
        }
        seq.end()
    }
}

/// A frame that could not be parsed, with whatever could still be read from it.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct FrameError {
    /// The message type, if the type number was readable and supported.
    pub message_type: Option<MessageType>,
    /// The message id, if it was readable and valid.
    pub id: Option<MessageId>,
    /// The error code to report back.
    pub code: ErrorCode,
    /// A human-readable explanation.
    pub message: String,
}

impl FrameError {
    fn new(
        message_type: Option<MessageType>,
        id: Option<MessageId>,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            message_type,
            id,
            code,
            message: message.into(),
        }
    }

    /// The CALLERROR to send back, or `None` when the broken frame was itself a response
    /// (a CALLRESULT or CALLERROR is never answered).
    ///
    /// When the id is unreadable the reply uses `"-1"`.
    // TODO(spec): confirm the "-1" convention for unreadable message ids in Part 4.
    pub fn reply(&self) -> Option<Frame> {
        match self.message_type {
            Some(MessageType::CallResult | MessageType::CallError) => None,
            Some(MessageType::Call) | None => {
                let id = self
                    .id
                    .clone()
                    .unwrap_or_else(|| MessageId("-1".to_owned()));
                Some(Frame::call_error(id, self.code, &self.message))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn parse_err(text: &str) -> FrameError {
        Frame::parse(text).expect_err("frame should be rejected")
    }

    #[test]
    fn parses_call() {
        let frame = Frame::parse(r#"[2,"abc","Heartbeat",{}]"#).unwrap();
        assert_eq!(
            frame,
            Frame::Call {
                id: MessageId::try_from("abc").unwrap(),
                action: "Heartbeat".into(),
                payload: json!({}),
            }
        );
    }

    #[test]
    fn parses_call_result_and_call_error() {
        let result = Frame::parse(r#"[3,"abc",{"currentTime":"2026-09-28T10:00:00Z"}]"#).unwrap();
        assert_eq!(result.message_type(), MessageType::CallResult);

        let error = Frame::parse(r#"[4,"abc","NotImplemented","no such action",{}]"#).unwrap();
        assert_eq!(
            error,
            Frame::call_error(
                MessageId::try_from("abc").unwrap(),
                ErrorCode::NotImplemented,
                "no such action"
            )
        );
    }

    #[test]
    fn serializes_to_ocpp_j_arrays() {
        let id = MessageId::try_from("42").unwrap();
        let call = Frame::Call {
            id: id.clone(),
            action: "Heartbeat".into(),
            payload: json!({}),
        };
        assert_eq!(call.to_json(), r#"[2,"42","Heartbeat",{}]"#);
        let error = Frame::call_error(id, ErrorCode::GenericError, "boom");
        assert_eq!(error.to_json(), r#"[4,"42","GenericError","boom",{}]"#);
    }

    #[test]
    fn rejects_invalid_json_and_non_arrays() {
        for text in ["", "[2,", r#"{"a":1}"#, "42", r#""call""#] {
            let e = parse_err(text);
            assert_eq!(e.code, ErrorCode::RpcFrameworkError, "{text}");
            assert_eq!(e.id, None);
        }
    }

    #[test]
    fn rejects_wrong_arity_keeping_the_id() {
        for text in [
            r#"[2,"x","Heartbeat"]"#,
            r#"[2,"x","Heartbeat",{},{}]"#,
            r#"[3,"x"]"#,
            r#"[3,"x",{},{}]"#,
            r#"[4,"x","GenericError","d"]"#,
        ] {
            let e = parse_err(text);
            assert_eq!(e.code, ErrorCode::RpcFrameworkError, "{text}");
            assert_eq!(e.id.as_ref().map(MessageId::as_str), Some("x"), "{text}");
        }
    }

    #[test]
    fn rejects_unknown_message_type() {
        let e = parse_err(r#"[5,"x","Heartbeat",{}]"#);
        assert_eq!(e.code, ErrorCode::MessageTypeNotSupported);
        assert_eq!(e.id.as_ref().map(MessageId::as_str), Some("x"));

        let e = parse_err(r#"["2","x","Heartbeat",{}]"#);
        assert_eq!(e.code, ErrorCode::RpcFrameworkError);
        assert_eq!(parse_err("[]").code, ErrorCode::RpcFrameworkError);
    }

    #[test]
    fn rejects_bad_message_ids() {
        let too_long = "a".repeat(MAX_MESSAGE_ID_LEN + 1);
        for text in [
            format!(r#"[2,"{too_long}","Heartbeat",{{}}]"#),
            r#"[2,"","Heartbeat",{}]"#.to_owned(),
            r#"[2,7,"Heartbeat",{}]"#.to_owned(),
        ] {
            let e = parse_err(&text);
            assert_eq!(e.code, ErrorCode::RpcFrameworkError, "{text}");
            assert_eq!(e.id, None);
        }
        let max = "a".repeat(MAX_MESSAGE_ID_LEN);
        assert!(Frame::parse(&format!(r#"[2,"{max}","Heartbeat",{{}}]"#)).is_ok());
    }

    #[test]
    fn rejects_bad_call_fields() {
        let e = parse_err(r#"[2,"x",1,{}]"#);
        assert_eq!(e.code, ErrorCode::RpcFrameworkError);
        let e = parse_err(r#"[2,"x","Heartbeat",[]]"#);
        assert_eq!(e.code, ErrorCode::FormatViolation);
        let e = parse_err(r#"[3,"x",null]"#);
        assert_eq!(e.code, ErrorCode::FormatViolation);
        let e = parse_err(r#"[4,"x","Oops","d",{}]"#);
        assert_eq!(e.code, ErrorCode::RpcFrameworkError);
    }

    #[test]
    fn reply_answers_calls_only() {
        let reply = parse_err(r#"[2,"x","Heartbeat",[]]"#).reply().unwrap();
        assert_eq!(reply.id().as_str(), "x");
        assert!(matches!(
            reply,
            Frame::CallError {
                code: ErrorCode::FormatViolation,
                ..
            }
        ));

        assert_eq!(parse_err("nope").reply().unwrap().id().as_str(), "-1");
        assert_eq!(parse_err(r#"[3,"x",null]"#).reply(), None);
    }

    #[test]
    fn new_v4_ids_are_valid() {
        let id = MessageId::new_v4();
        assert_eq!(id.as_str().len(), MAX_MESSAGE_ID_LEN);
        assert!(MessageId::try_from(id.as_str()).is_ok());
    }

    fn arb_payload() -> impl Strategy<Value = Value> {
        prop::collection::btree_map(
            "[a-zA-Z]{1,12}",
            prop_oneof![
                any::<i64>().prop_map(Value::from),
                ".{0,20}".prop_map(Value::from),
                any::<bool>().prop_map(Value::from),
            ],
            0..6,
        )
        .prop_map(|m| Value::Object(m.into_iter().collect()))
    }

    fn arb_frame() -> impl Strategy<Value = Frame> {
        let id = "[a-zA-Z0-9-]{1,36}".prop_map(|s| MessageId::try_from(s).unwrap());
        let code = prop::sample::select(ErrorCode::ALL.to_vec());
        prop_oneof![
            (id.clone(), "[A-Za-z0-9]{1,30}", arb_payload()).prop_map(|(id, action, payload)| {
                Frame::Call {
                    id,
                    action,
                    payload,
                }
            }),
            (id.clone(), arb_payload()).prop_map(|(id, payload)| Frame::CallResult { id, payload }),
            (id, code, ".{0,40}", arb_payload()).prop_map(|(id, code, description, details)| {
                Frame::CallError {
                    id,
                    code,
                    description,
                    details,
                }
            }),
        ]
    }

    proptest! {
        #[test]
        fn frames_round_trip(frame in arb_frame()) {
            prop_assert_eq!(Frame::parse(&frame.to_json()).unwrap(), frame);
        }
    }
}
