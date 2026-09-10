//! `TransactionEvent`: a transaction started, progressed or ended.

use serde::{Deserialize, Serialize};

use crate::bounded::{BoundedString, NonEmptyVec};
use crate::datetime::DateTime;
use crate::types::{CustomData, Evse, IdToken, IdTokenInfo, MessageContent, MeterValue};

/// `TransactionEventEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransactionEventKind {
    Ended,
    Started,
    Updated,
}

/// `TriggerReasonEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerReason {
    Authorized,
    CablePluggedIn,
    ChargingRateChanged,
    ChargingStateChanged,
    Deauthorized,
    EnergyLimitReached,
    #[serde(rename = "EVCommunicationLost")]
    EvCommunicationLost,
    #[serde(rename = "EVConnectTimeout")]
    EvConnectTimeout,
    MeterValueClock,
    MeterValuePeriodic,
    TimeLimitReached,
    Trigger,
    UnlockCommand,
    StopAuthorized,
    #[serde(rename = "EVDeparted")]
    EvDeparted,
    #[serde(rename = "EVDetected")]
    EvDetected,
    RemoteStop,
    RemoteStart,
    AbnormalCondition,
    SignedDataReceived,
    ResetCommand,
}

/// `ChargingStateEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChargingState {
    Charging,
    #[serde(rename = "EVConnected")]
    EvConnected,
    #[serde(rename = "SuspendedEV")]
    SuspendedEv,
    #[serde(rename = "SuspendedEVSE")]
    SuspendedEvse,
    Idle,
}

/// `ReasonEnumType`: why a transaction stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoppedReason {
    DeAuthorized,
    EmergencyStop,
    EnergyLimitReached,
    #[serde(rename = "EVDisconnected")]
    EvDisconnected,
    GroundFault,
    ImmediateReset,
    Local,
    LocalOutOfCredit,
    MasterPass,
    Other,
    OvercurrentFault,
    PowerLoss,
    PowerQuality,
    Reboot,
    Remote,
    #[serde(rename = "SOCLimitReached")]
    SocLimitReached,
    #[serde(rename = "StoppedByEV")]
    StoppedByEv,
    TimeLimitReached,
    Timeout,
}

/// `TransactionType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transaction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub transaction_id: BoundedString<36>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_state: Option<ChargingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_spent_charging: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_reason: Option<StoppedReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_start_id: Option<i32>,
}

/// `TransactionEventRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransactionEventRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub event_type: TransactionEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_value: Option<NonEmptyVec<MeterValue>>,
    pub timestamp: DateTime,
    pub trigger_reason: TriggerReason,
    pub seq_no: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_of_phases_used: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cable_max_current: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<i32>,
    pub transaction_info: Transaction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse: Option<Evse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<IdToken>,
}

/// `TransactionEventResponse`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransactionEventResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token_info: Option<IdTokenInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_personal_message: Option<MessageContent>,
}
