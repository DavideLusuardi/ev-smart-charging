//! `NotifyEVChargingNeeds`: an ISO 15118 EV reports how much energy it wants and by when.

use serde::{Deserialize, Serialize};

use crate::datetime::DateTime;
use crate::types::{CustomData, StatusInfo};

/// `EnergyTransferModeEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnergyTransferMode {
    #[serde(rename = "DC")]
    Dc,
    #[serde(rename = "AC_single_phase")]
    AcSinglePhase,
    #[serde(rename = "AC_two_phase")]
    AcTwoPhase,
    #[serde(rename = "AC_three_phase")]
    AcThreePhase,
}

/// `ACChargingParametersType`: energy in Wh, currents in A (per phase), voltage in V.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcChargingParameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub energy_amount: i32,
    pub ev_min_current: i32,
    pub ev_max_current: i32,
    pub ev_max_voltage: i32,
}

/// `DCChargingParametersType`: energy in Wh, current in A, voltage in V, power in W,
/// state of charge in percent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DcChargingParameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub ev_max_current: i32,
    pub ev_max_voltage: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_amount: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ev_max_power: Option<i32>,
    /// Schema range 0..=100; not enforced by the type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_of_charge: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ev_energy_capacity: Option<i32>,
    /// Schema range 0..=100; not enforced by the type.
    #[serde(rename = "fullSoC", default, skip_serializing_if = "Option::is_none")]
    pub full_soc: Option<i32>,
    /// Schema range 0..=100; not enforced by the type.
    #[serde(rename = "bulkSoC", default, skip_serializing_if = "Option::is_none")]
    pub bulk_soc: Option<i32>,
}

/// `ChargingNeedsType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChargingNeeds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ac_charging_parameters: Option<AcChargingParameters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dc_charging_parameters: Option<DcChargingParameters>,
    pub requested_energy_transfer: EnergyTransferMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub departure_time: Option<DateTime>,
}

/// `NotifyEVChargingNeedsRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotifyEvChargingNeedsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_schedule_tuples: Option<i32>,
    pub charging_needs: ChargingNeeds,
    pub evse_id: i32,
}

/// `NotifyEVChargingNeedsStatusEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NotifyEvChargingNeedsStatus {
    Accepted,
    Rejected,
    Processing,
}

/// `NotifyEVChargingNeedsResponse`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotifyEvChargingNeedsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub status: NotifyEvChargingNeedsStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_info: Option<StatusInfo>,
}
