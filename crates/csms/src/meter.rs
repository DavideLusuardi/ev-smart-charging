//! Reading energy and power out of OCPP meter values.

use ocpp::DateTime;
use ocpp::types::{Measurand, MeterValue, SampledValue};
use tracing::debug;

/// What one meter value says about a session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Reading {
    pub timestamp: DateTime,
    /// The energy import register, in Wh.
    pub energy_wh: Option<f64>,
    /// The power drawn, in W.
    pub power_w: Option<f64>,
}

/// Reads the most recent of `values`.
///
/// Only whole-EVSE values count: a sampled value with a `phase` is one phase of the total.
/// A sampled value without a measurand is an energy register reading: the OCA schema gives
/// `measurand` the default `Energy.Active.Import.Register`.
pub(crate) fn latest(values: &[MeterValue]) -> Option<Reading> {
    let latest = values.iter().max_by_key(|v| v.timestamp)?;
    let mut reading = Reading {
        timestamp: latest.timestamp,
        energy_wh: None,
        power_w: None,
    };
    for sample in latest.sampled_value.iter().filter(|s| s.phase.is_none()) {
        match sample
            .measurand
            .unwrap_or(Measurand::EnergyActiveImportRegister)
        {
            Measurand::EnergyActiveImportRegister => {
                reading.energy_wh = scaled(sample, "Wh", "kWh").or(reading.energy_wh);
            }
            Measurand::PowerActiveImport => {
                reading.power_w = scaled(sample, "W", "kW").or(reading.power_w);
            }
            _ => {}
        }
    }
    Some(reading)
}

/// The value in the base unit, applying the unit and its power-of-ten multiplier. A missing
/// unit means the base unit; an unexpected one makes the value unusable.
fn scaled(sample: &SampledValue, base: &str, kilo: &str) -> Option<f64> {
    let unit = sample.unit_of_measure.as_ref();
    let multiplier = unit.and_then(|u| u.multiplier).unwrap_or(0);
    let factor = match unit.and_then(|u| u.unit.as_deref()) {
        None => 1.0,
        Some(u) if u == base => 1.0,
        Some(u) if u == kilo => 1_000.0,
        Some(u) => {
            debug!(unit = u, "ignoring a meter value in an unexpected unit");
            return None;
        }
    };
    Some(sample.value * factor * 10f64.powi(multiplier))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn values(value: serde_json::Value) -> Vec<MeterValue> {
        ocpp::decode(value).unwrap()
    }

    #[test]
    fn reads_energy_and_power_from_the_latest_value() {
        let v = values(json!([
            {
                "timestamp": "2026-09-30T10:10:00Z",
                "sampledValue": [
                    { "value": 26520.5, "measurand": "Energy.Active.Import.Register" },
                    { "value": 11000.0, "measurand": "Power.Active.Import" },
                    { "value": 3600.0, "measurand": "Power.Active.Import", "phase": "L1" },
                    { "value": 62.0, "measurand": "SoC" }
                ]
            },
            {
                "timestamp": "2026-09-30T10:00:00Z",
                "sampledValue": [{ "value": 1.0 }]
            }
        ]));
        let r = latest(&v).unwrap();
        assert_eq!(r.timestamp.to_string(), "2026-09-30T10:10:00Z");
        assert_eq!(r.energy_wh, Some(26520.5));
        assert_eq!(r.power_w, Some(11000.0));
    }

    #[test]
    fn applies_units_and_multipliers() {
        let v = values(json!([{
            "timestamp": "2026-09-30T10:00:00Z",
            "sampledValue": [
                // No measurand: the energy register.
                { "value": 26.5, "unitOfMeasure": { "unit": "kWh" } },
                { "value": 11.0, "measurand": "Power.Active.Import",
                  "unitOfMeasure": { "unit": "W", "multiplier": 3 } }
            ]
        }]));
        let r = latest(&v).unwrap();
        assert_eq!(r.energy_wh, Some(26_500.0));
        assert_eq!(r.power_w, Some(11_000.0));
    }

    #[test]
    fn ignores_unexpected_units() {
        let v = values(json!([{
            "timestamp": "2026-09-30T10:00:00Z",
            "sampledValue": [
                { "value": 5.0, "measurand": "Power.Active.Import", "unitOfMeasure": { "unit": "A" } }
            ]
        }]));
        assert_eq!(latest(&v).unwrap().power_w, None);
        assert_eq!(latest(&[]), None);
    }
}
