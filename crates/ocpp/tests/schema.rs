//! Checks the message types against the official OCPP 2.0.1 JSON schemas in `specs/`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::{Draft, Validator};
use ocpp::bounded::{BoundedString, BoundedVec};
use ocpp::messages::authorize::{AuthorizeCertificateStatus, HashAlgorithm};
use ocpp::messages::boot_notification::{BootReason, RegistrationStatus};
use ocpp::messages::clear_charging_profile::ClearChargingProfileStatus;
use ocpp::messages::notify_ev_charging_needs::{EnergyTransferMode, NotifyEvChargingNeedsStatus};
use ocpp::messages::set_charging_profile::ChargingProfileStatus;
use ocpp::messages::status_notification::ConnectorStatus;
use ocpp::messages::transaction_event::{
    ChargingState, StoppedReason, TransactionEventKind, TriggerReason,
};
use ocpp::messages::*;
use ocpp::types::*;
use ocpp::{DateTime, DecodeError, ErrorCode, decode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// The schemas of the supported subset, one request and one response per action.
const SUBSET: [&str; 16] = [
    "AuthorizeRequest",
    "AuthorizeResponse",
    "BootNotificationRequest",
    "BootNotificationResponse",
    "ClearChargingProfileRequest",
    "ClearChargingProfileResponse",
    "HeartbeatRequest",
    "HeartbeatResponse",
    "NotifyEVChargingNeedsRequest",
    "NotifyEVChargingNeedsResponse",
    "SetChargingProfileRequest",
    "SetChargingProfileResponse",
    "StatusNotificationRequest",
    "StatusNotificationResponse",
    "TransactionEventRequest",
    "TransactionEventResponse",
];

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn schema(name: &str) -> Value {
    read_json(&crate_dir().join(format!("../../specs/ocpp-2.0.1/{name}.json")))
}

fn validator(name: &str) -> Validator {
    jsonschema::options()
        .with_draft(Draft::Draft6)
        .should_validate_formats(true)
        .build(&schema(name))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn schema_errors(name: &str, instance: &Value) -> Vec<String> {
    validator(name)
        .iter_errors(instance)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect()
}

/// Fixtures are named `<Schema>[.<variant>].json`.
fn fixtures() -> Vec<(String, PathBuf, Value)> {
    let dir = crate_dir().join("tests/fixtures");
    let mut out: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            let file = path.file_name().unwrap().to_str().unwrap();
            let schema = file.split('.').next().unwrap().to_owned();
            let value = read_json(&path);
            (schema, path, value)
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

fn rt<T: Serialize + DeserializeOwned>(value: Value) -> Result<Value, DecodeError> {
    decode::<T>(value).map(|typed| encode(&typed))
}

/// Decodes `value` into the Rust type for `schema` and encodes it again.
fn round_trip(schema: &str, value: Value) -> Result<Value, DecodeError> {
    match schema {
        "AuthorizeRequest" => rt::<AuthorizeRequest>(value),
        "AuthorizeResponse" => rt::<AuthorizeResponse>(value),
        "BootNotificationRequest" => rt::<BootNotificationRequest>(value),
        "BootNotificationResponse" => rt::<BootNotificationResponse>(value),
        "ClearChargingProfileRequest" => rt::<ClearChargingProfileRequest>(value),
        "ClearChargingProfileResponse" => rt::<ClearChargingProfileResponse>(value),
        "HeartbeatRequest" => rt::<HeartbeatRequest>(value),
        "HeartbeatResponse" => rt::<HeartbeatResponse>(value),
        "NotifyEVChargingNeedsRequest" => rt::<NotifyEvChargingNeedsRequest>(value),
        "NotifyEVChargingNeedsResponse" => rt::<NotifyEvChargingNeedsResponse>(value),
        "SetChargingProfileRequest" => rt::<SetChargingProfileRequest>(value),
        "SetChargingProfileResponse" => rt::<SetChargingProfileResponse>(value),
        "StatusNotificationRequest" => rt::<StatusNotificationRequest>(value),
        "StatusNotificationResponse" => rt::<StatusNotificationResponse>(value),
        "TransactionEventRequest" => rt::<TransactionEventRequest>(value),
        "TransactionEventResponse" => rt::<TransactionEventResponse>(value),
        other => panic!("no Rust type for schema {other}"),
    }
}

/// JSON equality that treats `1` and `1.0` as the same number.
fn same_json(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same_json(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_json(v, w)))
        }
        _ => a == b,
    }
}

#[test]
fn every_subset_schema_has_a_fixture() {
    let covered: BTreeSet<_> = fixtures().into_iter().map(|(schema, ..)| schema).collect();
    let missing: Vec<_> = SUBSET.iter().filter(|s| !covered.contains(**s)).collect();
    assert!(missing.is_empty(), "schemas without a fixture: {missing:?}");
    for schema in &covered {
        assert!(
            SUBSET.contains(&schema.as_str()),
            "fixture for unknown schema {schema}"
        );
    }
}

#[test]
fn fixtures_are_valid_against_their_schema() {
    for (schema, path, value) in fixtures() {
        let errors = schema_errors(&schema, &value);
        assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
    }
}

#[test]
fn fixtures_round_trip_losslessly() {
    for (schema, path, value) in fixtures() {
        let encoded = round_trip(&schema, value.clone())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            same_json(&value, &encoded),
            "{} changed in a round trip:\n{value:#}\n{encoded:#}",
            path.display()
        );
        let errors = schema_errors(&schema, &encoded);
        assert!(
            errors.is_empty(),
            "{} re-encoded: {errors:#?}",
            path.display()
        );
    }
}

/// `Definition.property` names declared by the subset schemas, excluding `customData`.
fn schema_properties() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |def: &str, node: &Value| {
        for prop in node["properties"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k)
        {
            if prop != "customData" {
                out.insert(format!("{def}.{prop}"));
            }
        }
    };
    for name in SUBSET {
        let root = schema(name);
        add(name, &root);
        for (def, node) in root["definitions"].as_object().unwrap() {
            add(def, node);
        }
    }
    out
}

/// Records every `Definition.property` a fixture populates.
fn fixture_properties(
    root: &Value,
    node: &Value,
    def: &str,
    value: &Value,
    out: &mut BTreeSet<String>,
) {
    let (node, def) = match node["$ref"].as_str() {
        Some(r) => {
            let name = r.trim_start_matches("#/definitions/");
            (&root["definitions"][name], name)
        }
        None => (node, def),
    };
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                out.insert(format!("{def}.{key}"));
                fixture_properties(root, &node["properties"][key], def, child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                fixture_properties(root, &node["items"], def, item, out);
            }
        }
        _ => {}
    }
}

#[test]
fn fixtures_populate_every_schema_property() {
    let mut seen = BTreeSet::new();
    for (schema_name, _, value) in fixtures() {
        let root = schema(&schema_name);
        fixture_properties(&root, &root, &schema_name, &value, &mut seen);
    }
    let missing: Vec<_> = schema_properties().difference(&seen).cloned().collect();
    assert!(
        missing.is_empty(),
        "properties no fixture sets: {missing:#?}"
    );
}

fn accepts<T: DeserializeOwned>(value: &str) -> bool {
    decode::<T>(json!(value)).is_ok()
}

/// Whether the Rust enum mapped to schema enum `def` accepts `value`, or `None` if unmapped.
fn enum_accepts(def: &str, value: &str) -> Option<bool> {
    Some(match def {
        "AuthorizationStatusEnumType" => accepts::<AuthorizationStatus>(value),
        "AuthorizeCertificateStatusEnumType" => accepts::<AuthorizeCertificateStatus>(value),
        "BootReasonEnumType" => accepts::<BootReason>(value),
        "ChargingProfileKindEnumType" => accepts::<ChargingProfileKind>(value),
        "ChargingProfilePurposeEnumType" => accepts::<ChargingProfilePurpose>(value),
        "ChargingProfileStatusEnumType" => accepts::<ChargingProfileStatus>(value),
        "ChargingRateUnitEnumType" => accepts::<ChargingRateUnit>(value),
        "ChargingStateEnumType" => accepts::<ChargingState>(value),
        "ClearChargingProfileStatusEnumType" => accepts::<ClearChargingProfileStatus>(value),
        "ConnectorStatusEnumType" => accepts::<ConnectorStatus>(value),
        "CostKindEnumType" => accepts::<CostKind>(value),
        "EnergyTransferModeEnumType" => accepts::<EnergyTransferMode>(value),
        "HashAlgorithmEnumType" => accepts::<HashAlgorithm>(value),
        "IdTokenEnumType" => accepts::<IdTokenKind>(value),
        "LocationEnumType" => accepts::<Location>(value),
        "MeasurandEnumType" => accepts::<Measurand>(value),
        "MessageFormatEnumType" => accepts::<MessageFormat>(value),
        "NotifyEVChargingNeedsStatusEnumType" => accepts::<NotifyEvChargingNeedsStatus>(value),
        "PhaseEnumType" => accepts::<Phase>(value),
        "ReadingContextEnumType" => accepts::<ReadingContext>(value),
        "ReasonEnumType" => accepts::<StoppedReason>(value),
        "RecurrencyKindEnumType" => accepts::<RecurrencyKind>(value),
        "RegistrationStatusEnumType" => accepts::<RegistrationStatus>(value),
        "TransactionEventEnumType" => accepts::<TransactionEventKind>(value),
        "TriggerReasonEnumType" => accepts::<TriggerReason>(value),
        _ => return None,
    })
}

#[test]
fn every_schema_enum_value_decodes() {
    let mut problems = Vec::new();
    for name in SUBSET {
        let root = schema(name);
        for (def, node) in root["definitions"].as_object().unwrap() {
            let Some(values) = node["enum"].as_array() else {
                continue;
            };
            for value in values.iter().map(|v| v.as_str().unwrap()) {
                match enum_accepts(def, value) {
                    None => problems.push(format!("{def} has no Rust enum")),
                    Some(false) => problems.push(format!("{def}: `{value}` is rejected")),
                    Some(true) => {}
                }
            }
        }
    }
    problems.dedup();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn invalid_payloads_are_rejected_by_schema_and_types() {
    let status = |patch: Value| {
        let mut v = json!({
            "timestamp": "2026-09-28T10:00:00Z",
            "connectorStatus": "Available",
            "evseId": 1,
            "connectorId": 1
        });
        for (k, val) in patch.as_object().unwrap() {
            if val.is_null() {
                v.as_object_mut().unwrap().remove(k);
            } else {
                v[k] = val.clone();
            }
        }
        v
    };
    let profile = |schedules: Value| {
        json!({
            "evseId": 1,
            "chargingProfile": {
                "id": 1, "stackLevel": 0,
                "chargingProfilePurpose": "TxProfile", "chargingProfileKind": "Absolute",
                "chargingSchedule": schedules
            }
        })
    };
    let schedule = |periods: Value| json!({"id": 1, "chargingRateUnit": "W", "chargingSchedulePeriod": periods});
    let period = json!({"startPeriod": 0, "limit": 1000.0});

    let cases = [
        (
            "StatusNotificationRequest",
            status(json!({"connectorId": null})),
            ErrorCode::OccurrenceConstraintViolation,
        ),
        (
            "StatusNotificationRequest",
            status(json!({"evseId": "one"})),
            ErrorCode::TypeConstraintViolation,
        ),
        (
            "StatusNotificationRequest",
            status(json!({"connectorStatus": "Broken"})),
            ErrorCode::PropertyConstraintViolation,
        ),
        (
            "StatusNotificationRequest",
            status(json!({"timestamp": "yesterday"})),
            ErrorCode::PropertyConstraintViolation,
        ),
        (
            "StatusNotificationRequest",
            status(json!({"power": 11000})),
            ErrorCode::FormatViolation,
        ),
        (
            "BootNotificationRequest",
            json!({"reason": "PowerUp", "chargingStation": {
                "model": "M".repeat(21), "vendorName": "ExampleEV"
            }}),
            ErrorCode::PropertyConstraintViolation,
        ),
        (
            "SetChargingProfileRequest",
            profile(json!([schedule(json!([]))])),
            ErrorCode::OccurrenceConstraintViolation,
        ),
        (
            "SetChargingProfileRequest",
            profile(json!([
                schedule(json!([period])),
                schedule(json!([period])),
                schedule(json!([period])),
                schedule(json!([period]))
            ])),
            ErrorCode::OccurrenceConstraintViolation,
        ),
        (
            "TransactionEventRequest",
            json!({
                "eventType": "Updated", "timestamp": "2026-09-28T10:00:00Z",
                "triggerReason": "MeterValuePeriodic", "seqNo": 1,
                "transactionInfo": {"transactionId": "tx-1"}, "meterValue": []
            }),
            ErrorCode::OccurrenceConstraintViolation,
        ),
    ];

    for (schema_name, payload, expected) in cases {
        assert!(
            !schema_errors(schema_name, &payload).is_empty(),
            "schema accepts {schema_name}: {payload}"
        );
        let err = round_trip(schema_name, payload.clone()).expect_err("types accept it");
        assert_eq!(err.code, expected, "{schema_name}: {payload}\n{err}");
    }
}

#[test]
fn profiles_built_in_rust_are_schema_valid() {
    let start: DateTime = decode(json!("2026-09-28T10:00:00Z")).unwrap();
    let periods = [(0, 22000.0), (900, 11000.0), (3600, 0.0)]
        .into_iter()
        .map(|(start_period, limit)| ChargingSchedulePeriod {
            custom_data: None,
            start_period,
            limit,
            number_phases: None,
            phase_to_use: None,
        })
        .collect();
    let request = SetChargingProfileRequest {
        custom_data: None,
        evse_id: 1,
        charging_profile: ChargingProfile {
            custom_data: None,
            id: 42,
            stack_level: 0,
            charging_profile_purpose: ChargingProfilePurpose::TxProfile,
            charging_profile_kind: ChargingProfileKind::Absolute,
            recurrency_kind: None,
            valid_from: None,
            valid_to: None,
            charging_schedule: BoundedVec::new(vec![ChargingSchedule {
                custom_data: None,
                id: 42,
                start_schedule: Some(start),
                duration: None,
                charging_rate_unit: ChargingRateUnit::W,
                charging_schedule_period: BoundedVec::new(periods).unwrap(),
                min_charging_rate: None,
                sales_tariff: None,
            }])
            .unwrap(),
            transaction_id: Some(BoundedString::new("tx-1").unwrap()),
        },
    };
    let errors = schema_errors("SetChargingProfileRequest", &encode(&request));
    assert!(errors.is_empty(), "{errors:#?}");

    let boot = BootNotificationResponse {
        custom_data: None,
        current_time: DateTime::now(),
        interval: 300,
        status: RegistrationStatus::Accepted,
        status_info: None,
    };
    let errors = schema_errors("BootNotificationResponse", &encode(&boot));
    assert!(errors.is_empty(), "{errors:#?}");
}
