//! `StatusNotification`: a connector's availability changed.

use serde::{Deserialize, Serialize};

use crate::datetime::DateTime;
use crate::types::CustomData;

/// `ConnectorStatusEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConnectorStatus {
    Available,
    Occupied,
    Reserved,
    Unavailable,
    Faulted,
}

/// `StatusNotificationRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusNotificationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub timestamp: DateTime,
    pub connector_status: ConnectorStatus,
    pub evse_id: i32,
    pub connector_id: i32,
}

/// `StatusNotificationResponse`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusNotificationResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
}
