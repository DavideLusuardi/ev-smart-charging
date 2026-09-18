//! The charging profiles the CSMS sends, in OCPP form.
//!
//! Each EVSE has one TxProfile id, so a new plan replaces the profile it had instead of
//! stacking on top of it.
// TODO(spec): that a SetChargingProfile with an existing id replaces that profile is how we
// read OCPP 2.0.1 Part 2 (K01); confirm it.

use ocpp::messages::SetChargingProfileRequest;
use ocpp::types::{
    ChargingProfile, ChargingProfileKind, ChargingProfilePurpose, ChargingRateUnit,
    ChargingSchedule, ChargingSchedulePeriod,
};
use ocpp::{BoundedString, BoundedVec, DateTime};
use optimizer::StepProfile;

/// The most periods a charging schedule can hold.
pub const MAX_PERIODS: usize = 1024;

/// The id of the TxDefaultProfile sent after boot.
pub const TX_DEFAULT_PROFILE_ID: i32 = 1;

/// The TxProfile id of an EVSE.
pub fn tx_profile_id(evse_id: i32) -> i32 {
    100 + evse_id
}

/// Why a profile can't be sent.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProfileError {
    #[error("the profile has {0} periods, more than a schedule holds")]
    TooManyPeriods(usize),
    #[error("a period starts {0} s in, too far to encode")]
    OffsetTooLarge(i64),
    #[error("the transaction id is too long")]
    TransactionId,
    #[error("the start time is out of range")]
    StartTime,
}

/// A TxProfile for the transaction on `evse_id`: an absolute schedule in W that starts at
/// the profile's start, with one period per step.
pub fn tx_profile(
    transaction_id: &str,
    evse_id: i32,
    profile: &StepProfile,
) -> Result<SetChargingProfileRequest, ProfileError> {
    let periods = profile
        .periods
        .iter()
        .map(|p| {
            Ok(ChargingSchedulePeriod {
                custom_data: None,
                start_period: i32::try_from(p.start_offset_secs)
                    .map_err(|_| ProfileError::OffsetTooLarge(p.start_offset_secs))?,
                limit: p.limit_w,
                number_phases: None,
                phase_to_use: None,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let count = periods.len();
    let periods = BoundedVec::new(periods).map_err(|_| ProfileError::TooManyPeriods(count))?;
    let start =
        chrono::DateTime::from_timestamp(profile.start_secs, 0).ok_or(ProfileError::StartTime)?;
    let id = tx_profile_id(evse_id);
    let schedule = ChargingSchedule {
        custom_data: None,
        id,
        start_schedule: Some(DateTime::from(start)),
        duration: None,
        charging_rate_unit: ChargingRateUnit::W,
        charging_schedule_period: periods,
        min_charging_rate: None,
        sales_tariff: None,
    };
    Ok(SetChargingProfileRequest {
        custom_data: None,
        evse_id,
        charging_profile: ChargingProfile {
            custom_data: None,
            id,
            stack_level: 0,
            charging_profile_purpose: ChargingProfilePurpose::TxProfile,
            charging_profile_kind: ChargingProfileKind::Absolute,
            recurrency_kind: None,
            valid_from: None,
            valid_to: None,
            charging_schedule: BoundedVec::new(vec![schedule]).expect("one schedule"),
            transaction_id: Some(
                BoundedString::new(transaction_id).map_err(|_| ProfileError::TransactionId)?,
            ),
        },
    })
}

/// A TxDefaultProfile for the whole station: every EVSE draws at most `limit_w` from `start`
/// until the CSMS sends it a TxProfile.
///
/// It is sent on `evseId` 0, which for a TxDefaultProfile "applies the profile to each
/// individual evse" (the `evseId` description in the OCA schema), not to the station as a
/// whole.
pub fn tx_default_profile(limit_w: f64, start: DateTime) -> SetChargingProfileRequest {
    let schedule = ChargingSchedule {
        custom_data: None,
        id: TX_DEFAULT_PROFILE_ID,
        start_schedule: Some(start),
        duration: None,
        charging_rate_unit: ChargingRateUnit::W,
        charging_schedule_period: BoundedVec::new(vec![ChargingSchedulePeriod {
            custom_data: None,
            start_period: 0,
            limit: limit_w,
            number_phases: None,
            phase_to_use: None,
        }])
        .expect("one period"),
        min_charging_rate: None,
        sales_tariff: None,
    };
    SetChargingProfileRequest {
        custom_data: None,
        evse_id: 0,
        charging_profile: ChargingProfile {
            custom_data: None,
            id: TX_DEFAULT_PROFILE_ID,
            stack_level: 0,
            charging_profile_purpose: ChargingProfilePurpose::TxDefaultProfile,
            charging_profile_kind: ChargingProfileKind::Absolute,
            recurrency_kind: None,
            valid_from: None,
            valid_to: None,
            charging_schedule: BoundedVec::new(vec![schedule]).expect("one schedule"),
            transaction_id: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocpp::messages::encode;
    use serde_json::Value;

    #[test]
    fn tx_profiles_match_the_ocpp_shape() {
        let start = "2026-09-28T10:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            .timestamp();
        let profile =
            StepProfile::from_steps(start, [(0, 150_000.0), (600, 97_500.5), (1_800, 0.0)]);
        let request = tx_profile("3f2b8d6e-1c0a-4d9b-9a57-6e1f0c2b7d44", 1, &profile).unwrap();
        let expected: Value = serde_json::from_str(include_str!(
            "../../ocpp/tests/fixtures/SetChargingProfileRequest.tx-profile.json"
        ))
        .unwrap();
        assert_eq!(encode(&request), expected);
    }

    #[test]
    fn the_tx_default_profile_matches_the_ocpp_shape() {
        let start: DateTime = ocpp::decode(serde_json::json!("2026-09-28T09:59:30Z")).unwrap();
        let expected: Value = serde_json::from_str(include_str!(
            "../../ocpp/tests/fixtures/SetChargingProfileRequest.tx-default.json"
        ))
        .unwrap();
        assert_eq!(encode(&tx_default_profile(1_400.0, start)), expected);
    }

    #[test]
    fn oversized_profiles_are_refused() {
        let steps = (0..=MAX_PERIODS as i64).map(|k| (k * 60, (k % 2) as f64));
        let profile = StepProfile::from_steps(0, steps);
        assert_eq!(
            tx_profile("tx", 1, &profile),
            Err(ProfileError::TooManyPeriods(MAX_PERIODS + 1))
        );
    }
}
