//! `BootNotification`: the first message a station sends after connecting.

use serde::{Deserialize, Serialize};

use crate::bounded::BoundedString;
use crate::datetime::DateTime;
use crate::types::{CustomData, StatusInfo};

/// `BootReasonEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BootReason {
    ApplicationReset,
    FirmwareUpdate,
    LocalReset,
    PowerUp,
    RemoteReset,
    ScheduledReset,
    Triggered,
    Unknown,
    Watchdog,
}

/// `ModemType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Modem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iccid: Option<BoundedString<20>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imsi: Option<BoundedString<20>>,
}

/// `ChargingStationType`: what the station reports about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChargingStation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<BoundedString<25>>,
    pub model: BoundedString<20>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modem: Option<Modem>,
    pub vendor_name: BoundedString<50>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<BoundedString<50>>,
}

/// `BootNotificationRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootNotificationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub charging_station: ChargingStation,
    pub reason: BootReason,
}

/// `RegistrationStatusEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegistrationStatus {
    Accepted,
    Pending,
    Rejected,
}

/// `BootNotificationResponse`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootNotificationResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub current_time: DateTime,
    /// Heartbeat interval in seconds when accepted; otherwise the minimum wait before the
    /// next `BootNotification`.
    pub interval: i32,
    pub status: RegistrationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_info: Option<StatusInfo>,
}
