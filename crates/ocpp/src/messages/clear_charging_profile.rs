//! `ClearChargingProfile`: the CSMS removes charging profiles from a station.

use serde::{Deserialize, Serialize};

use crate::types::{ChargingProfilePurpose, CustomData, StatusInfo};

/// `ClearChargingProfileType`: which profiles to clear when no id is given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClearChargingProfileCriteria {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_profile_purpose: Option<ChargingProfilePurpose>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_level: Option<i32>,
}

/// `ClearChargingProfileRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClearChargingProfileRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_profile_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_profile_criteria: Option<ClearChargingProfileCriteria>,
}

/// `ClearChargingProfileStatusEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClearChargingProfileStatus {
    Accepted,
    Unknown,
}

/// `ClearChargingProfileResponse`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClearChargingProfileResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub status: ClearChargingProfileStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_info: Option<StatusInfo>,
}
