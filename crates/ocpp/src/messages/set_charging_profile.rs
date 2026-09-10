//! `SetChargingProfile`: the CSMS installs a charging profile on an EVSE.

use serde::{Deserialize, Serialize};

use crate::types::{ChargingProfile, CustomData, StatusInfo};

/// `SetChargingProfileRequest`. `evse_id` 0 targets the whole station.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetChargingProfileRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub evse_id: i32,
    pub charging_profile: ChargingProfile,
}

/// `ChargingProfileStatusEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChargingProfileStatus {
    Accepted,
    Rejected,
}

/// `SetChargingProfileResponse`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetChargingProfileResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub status: ChargingProfileStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_info: Option<StatusInfo>,
}
