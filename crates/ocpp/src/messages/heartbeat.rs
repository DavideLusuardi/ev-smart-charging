//! `Heartbeat`: a keep-alive that also syncs the station's clock.

use serde::{Deserialize, Serialize};

use crate::datetime::DateTime;
use crate::types::CustomData;

/// `HeartbeatRequest`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeartbeatRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
}

/// `HeartbeatResponse`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeartbeatResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub current_time: DateTime,
}
