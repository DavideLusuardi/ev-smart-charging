//! Sending CALLs to a connected station.

use ocpp::messages::{Action, encode};
use ocpp::{DecodeError, ErrorCode, decode};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// Why a CALL to a station produced no response.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CallError {
    /// The station didn't answer within the call timeout.
    #[error("the station did not answer in time")]
    Timeout,
    /// The station answered with a CALLERROR.
    #[error("the station answered {code}: {description}")]
    Rejected {
        code: ErrorCode,
        description: String,
    },
    /// The CALLRESULT doesn't match the action's response type.
    #[error("the station's response is invalid: {0}")]
    Decode(DecodeError),
    /// The station disconnected before answering, or wasn't connected.
    #[error("the station is not connected")]
    Disconnected,
}

/// A CALL waiting to be sent on a connection.
#[derive(Debug)]
pub(crate) struct OutboundCall {
    pub action: &'static str,
    pub payload: Value,
    pub reply: oneshot::Sender<Result<Value, CallError>>,
}

/// Sends CALLs to one connected station. Cheap to clone.
///
/// A connection has at most one CALL in flight: further calls wait in a queue until the
/// previous one is answered or times out. The timeout only starts once a call is sent.
#[derive(Debug, Clone)]
pub struct StationHandle {
    calls: mpsc::Sender<OutboundCall>,
}

impl StationHandle {
    pub(crate) fn new(calls: mpsc::Sender<OutboundCall>) -> Self {
        Self { calls }
    }

    /// Sends a CALL for action `A` and waits for the station's response.
    pub async fn call<A: Action>(&self, request: &A::Request) -> Result<A::Response, CallError> {
        let (reply, response) = oneshot::channel();
        let call = OutboundCall {
            action: A::NAME,
            payload: encode(request),
            reply,
        };
        self.calls
            .send(call)
            .await
            .map_err(|_| CallError::Disconnected)?;
        let payload = response.await.map_err(|_| CallError::Disconnected)??;
        decode(payload).map_err(CallError::Decode)
    }
}
