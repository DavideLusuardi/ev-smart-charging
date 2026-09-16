//! Answers to the CALLs a station sends.

use ocpp::{ErrorCode, Frame, MessageId, StationRequest};
use serde_json::Value;

/// Answers one CALL with a CALLRESULT or a CALLERROR.
pub(crate) fn handle(id: MessageId, action: &str, payload: Value) -> Frame {
    match StationRequest::decode(action, payload) {
        Ok(request) => Frame::call_error(
            id,
            ErrorCode::NotSupported,
            format!("{} is not handled yet", request.action()),
        ),
        Err(e) => Frame::call_error(id, e.code, e.message),
    }
}
