//! OCPP-J RPC framing and the OCPP 2.0.1 message subset used for smart charging.

pub mod error;
pub mod rpc;

pub use error::{DecodeError, ErrorCode, decode};
pub use rpc::{Frame, FrameError, MessageId, MessageType};
