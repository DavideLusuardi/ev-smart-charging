//! OCPP 2.0.1 datatypes and enumerations shared by several messages.
//!
//! Field names and enum values follow the OCA JSON schemas exactly. Every object rejects
//! unknown properties (`additionalProperties: false`) except [`CustomData`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::bounded::{BoundedString, BoundedVec, NonEmptyVec};
use crate::datetime::DateTime;

/// `CustomDataType`: vendor-specific extension data, allowed on every object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomData {
    pub vendor_id: BoundedString<255>,
    /// Any further vendor-defined properties.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `StatusInfoType`: more detail about a response status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub reason_code: BoundedString<20>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_info: Option<BoundedString<512>>,
}

/// `EVSEType`: an EVSE, optionally narrowed to one connector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub id: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<i32>,
}

// ---------------------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------------------

/// `IdTokenEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdTokenKind {
    Central,
    #[serde(rename = "eMAID")]
    EMaid,
    #[serde(rename = "ISO14443")]
    Iso14443,
    #[serde(rename = "ISO15693")]
    Iso15693,
    KeyCode,
    Local,
    MacAddress,
    NoAuthorization,
}

/// `AdditionalInfoType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdditionalInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub additional_id_token: BoundedString<36>,
    #[serde(rename = "type")]
    pub kind: BoundedString<50>,
}

/// `IdTokenType`: the identifier a driver presents (RFID card, app, contract…).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdToken {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_info: Option<NonEmptyVec<AdditionalInfo>>,
    pub id_token: BoundedString<36>,
    #[serde(rename = "type")]
    pub kind: IdTokenKind,
}

/// `AuthorizationStatusEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthorizationStatus {
    Accepted,
    Blocked,
    ConcurrentTx,
    Expired,
    Invalid,
    NoCredit,
    #[serde(rename = "NotAllowedTypeEVSE")]
    NotAllowedTypeEvse,
    NotAtThisLocation,
    NotAtThisTime,
    Unknown,
}

/// `MessageFormatEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageFormat {
    #[serde(rename = "ASCII")]
    Ascii,
    #[serde(rename = "HTML")]
    Html,
    #[serde(rename = "URI")]
    Uri,
    #[serde(rename = "UTF8")]
    Utf8,
}

/// `MessageContentType`: a message to show to the driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub format: MessageFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<BoundedString<8>>,
    pub content: BoundedString<512>,
}

/// `IdTokenInfoType`: the CSMS's verdict on an [`IdToken`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdTokenInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub status: AuthorizationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_expiry_date_time: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language1: Option<BoundedString<8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_id: Option<NonEmptyVec<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id_token: Option<IdToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language2: Option<BoundedString<8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_message: Option<MessageContent>,
}

// ---------------------------------------------------------------------------------------
// Meter values
// ---------------------------------------------------------------------------------------

/// `ReadingContextEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReadingContext {
    #[serde(rename = "Interruption.Begin")]
    InterruptionBegin,
    #[serde(rename = "Interruption.End")]
    InterruptionEnd,
    Other,
    #[serde(rename = "Sample.Clock")]
    SampleClock,
    #[serde(rename = "Sample.Periodic")]
    SamplePeriodic,
    #[serde(rename = "Transaction.Begin")]
    TransactionBegin,
    #[serde(rename = "Transaction.End")]
    TransactionEnd,
    Trigger,
}

/// `MeasurandEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Measurand {
    #[serde(rename = "Current.Export")]
    CurrentExport,
    #[serde(rename = "Current.Import")]
    CurrentImport,
    #[serde(rename = "Current.Offered")]
    CurrentOffered,
    #[serde(rename = "Energy.Active.Export.Register")]
    EnergyActiveExportRegister,
    #[serde(rename = "Energy.Active.Import.Register")]
    EnergyActiveImportRegister,
    #[serde(rename = "Energy.Reactive.Export.Register")]
    EnergyReactiveExportRegister,
    #[serde(rename = "Energy.Reactive.Import.Register")]
    EnergyReactiveImportRegister,
    #[serde(rename = "Energy.Active.Export.Interval")]
    EnergyActiveExportInterval,
    #[serde(rename = "Energy.Active.Import.Interval")]
    EnergyActiveImportInterval,
    #[serde(rename = "Energy.Active.Net")]
    EnergyActiveNet,
    #[serde(rename = "Energy.Reactive.Export.Interval")]
    EnergyReactiveExportInterval,
    #[serde(rename = "Energy.Reactive.Import.Interval")]
    EnergyReactiveImportInterval,
    #[serde(rename = "Energy.Reactive.Net")]
    EnergyReactiveNet,
    #[serde(rename = "Energy.Apparent.Net")]
    EnergyApparentNet,
    #[serde(rename = "Energy.Apparent.Import")]
    EnergyApparentImport,
    #[serde(rename = "Energy.Apparent.Export")]
    EnergyApparentExport,
    Frequency,
    #[serde(rename = "Power.Active.Export")]
    PowerActiveExport,
    #[serde(rename = "Power.Active.Import")]
    PowerActiveImport,
    #[serde(rename = "Power.Factor")]
    PowerFactor,
    #[serde(rename = "Power.Offered")]
    PowerOffered,
    #[serde(rename = "Power.Reactive.Export")]
    PowerReactiveExport,
    #[serde(rename = "Power.Reactive.Import")]
    PowerReactiveImport,
    SoC,
    Voltage,
}

/// `PhaseEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Phase {
    L1,
    L2,
    L3,
    N,
    #[serde(rename = "L1-N")]
    L1N,
    #[serde(rename = "L2-N")]
    L2N,
    #[serde(rename = "L3-N")]
    L3N,
    #[serde(rename = "L1-L2")]
    L1L2,
    #[serde(rename = "L2-L3")]
    L2L3,
    #[serde(rename = "L3-L1")]
    L3L1,
}

/// `LocationEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Location {
    Body,
    Cable,
    #[serde(rename = "EV")]
    Ev,
    Inlet,
    Outlet,
}

/// `SignedMeterValueType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedMeterValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub signed_meter_data: BoundedString<2500>,
    pub signing_method: BoundedString<50>,
    pub encoding_method: BoundedString<50>,
    pub public_key: BoundedString<2500>,
}

/// `UnitOfMeasureType`. When absent the spec's defaults apply (unit `Wh`, multiplier 0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitOfMeasure {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<BoundedString<20>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<i32>,
}

/// `SampledValueType`: one measured value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SampledValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ReadingContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurand: Option<Measurand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_meter_value: Option<SignedMeterValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<UnitOfMeasure>,
}

/// `MeterValueType`: sampled values taken at one point in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeterValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub sampled_value: NonEmptyVec<SampledValue>,
    pub timestamp: DateTime,
}

// ---------------------------------------------------------------------------------------
// Charging profiles
// ---------------------------------------------------------------------------------------

/// `ChargingProfilePurposeEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChargingProfilePurpose {
    ChargingStationExternalConstraints,
    ChargingStationMaxProfile,
    TxDefaultProfile,
    TxProfile,
}

/// `ChargingProfileKindEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChargingProfileKind {
    Absolute,
    Recurring,
    Relative,
}

/// `RecurrencyKindEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecurrencyKind {
    Daily,
    Weekly,
}

/// `ChargingRateUnitEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChargingRateUnit {
    W,
    A,
}

/// `CostKindEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CostKind {
    CarbonDioxideEmission,
    RelativePricePercentage,
    RenewableGenerationPercentage,
}

/// `CostType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub cost_kind: CostKind,
    pub amount: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_multiplier: Option<i32>,
}

/// `ConsumptionCostType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsumptionCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub start_value: f64,
    pub cost: BoundedVec<Cost, 1, 3>,
}

/// `RelativeTimeIntervalType`: seconds relative to the schedule start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelativeTimeInterval {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub start: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<i32>,
}

/// `SalesTariffEntryType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SalesTariffEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub relative_time_interval: RelativeTimeInterval,
    /// Schema minimum 0; not enforced by the type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e_price_level: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumption_cost: Option<BoundedVec<ConsumptionCost, 1, 3>>,
}

/// `SalesTariffType`: an ISO 15118 tariff passed through to the EV.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SalesTariff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub id: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sales_tariff_description: Option<BoundedString<32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_e_price_levels: Option<i32>,
    pub sales_tariff_entry: BoundedVec<SalesTariffEntry, 1, 1024>,
}

/// `ChargingSchedulePeriodType`: a limit that holds from `start_period` seconds after the
/// schedule start until the next period.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChargingSchedulePeriod {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub start_period: i32,
    /// In the schedule's [`ChargingRateUnit`].
    pub limit: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_phases: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_to_use: Option<i32>,
}

/// `ChargingScheduleType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChargingSchedule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub id: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_schedule: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<i32>,
    pub charging_rate_unit: ChargingRateUnit,
    pub charging_schedule_period: BoundedVec<ChargingSchedulePeriod, 1, 1024>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_charging_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sales_tariff: Option<SalesTariff>,
}

/// `ChargingProfileType`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChargingProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub id: i32,
    pub stack_level: i32,
    pub charging_profile_purpose: ChargingProfilePurpose,
    pub charging_profile_kind: ChargingProfileKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrency_kind: Option<RecurrencyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<DateTime>,
    pub charging_schedule: BoundedVec<ChargingSchedule, 1, 3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<BoundedString<36>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorCode, decode};
    use serde_json::json;

    fn spelling<T: Serialize>(value: T) -> String {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn renamed_enum_values_match_the_schema() {
        assert_eq!(spelling(IdTokenKind::EMaid), "eMAID");
        assert_eq!(spelling(IdTokenKind::Iso14443), "ISO14443");
        assert_eq!(
            spelling(AuthorizationStatus::NotAllowedTypeEvse),
            "NotAllowedTypeEVSE"
        );
        assert_eq!(spelling(MessageFormat::Utf8), "UTF8");
        assert_eq!(spelling(ReadingContext::SamplePeriodic), "Sample.Periodic");
        assert_eq!(
            spelling(Measurand::EnergyActiveImportRegister),
            "Energy.Active.Import.Register"
        );
        assert_eq!(spelling(Measurand::SoC), "SoC");
        assert_eq!(spelling(Phase::L3L1), "L3-L1");
        assert_eq!(spelling(Location::Ev), "EV");
        assert_eq!(spelling(ChargingRateUnit::W), "W");
    }

    #[test]
    fn custom_data_keeps_vendor_properties() {
        let json = json!({"vendorId": "com.example", "firmwareFlavor": "beta", "n": 3});
        let data: CustomData = decode(json.clone()).unwrap();
        assert_eq!(data.vendor_id.as_str(), "com.example");
        assert_eq!(serde_json::to_value(&data).unwrap(), json);
    }

    #[test]
    fn optional_fields_are_omitted() {
        let evse = Evse {
            custom_data: None,
            id: 1,
            connector_id: None,
        };
        assert_eq!(serde_json::to_value(&evse).unwrap(), json!({"id": 1}));
    }

    #[test]
    fn nested_errors_are_classified() {
        let period = |limit: Value| {
            json!({
                "id": 1,
                "chargingRateUnit": "W",
                "chargingSchedulePeriod": [{"startPeriod": 0, "limit": limit}]
            })
        };
        assert!(decode::<ChargingSchedule>(period(json!(11000.0))).is_ok());
        let err = decode::<ChargingSchedule>(period(json!("fast"))).unwrap_err();
        assert_eq!(err.code, ErrorCode::TypeConstraintViolation);

        let no_periods = json!({"id": 1, "chargingRateUnit": "W", "chargingSchedulePeriod": []});
        let err = decode::<ChargingSchedule>(no_periods).unwrap_err();
        assert_eq!(err.code, ErrorCode::OccurrenceConstraintViolation);

        let bad_unit = json!({"id": 1, "chargingRateUnit": "kW", "chargingSchedulePeriod": [
            {"startPeriod": 0, "limit": 1.0}
        ]});
        let err = decode::<ChargingSchedule>(bad_unit).unwrap_err();
        assert_eq!(err.code, ErrorCode::PropertyConstraintViolation);
    }

    #[test]
    fn unknown_properties_are_rejected() {
        let err = decode::<Evse>(json!({"id": 1, "power": 11})).unwrap_err();
        assert_eq!(err.code, ErrorCode::FormatViolation);
    }
}
