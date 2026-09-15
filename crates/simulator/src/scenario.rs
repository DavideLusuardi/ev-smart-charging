//! Scenario files: the site, the control settings, and how EVs arrive and behave.
//!
//! Units follow the rest of the code: W, Wh and seconds. The presets in
//! `config/scenarios/` are built into the binary, so they work from any directory.

use std::path::Path;

use optimizer::{ControlConfig, NeedsDefaults};
use serde::Deserialize;

const PRESETS: &[(&str, &str)] = &[
    (
        "workplace",
        include_str!("../../../config/scenarios/workplace.toml"),
    ),
    ("hub", include_str!("../../../config/scenarios/hub.toml")),
];

/// Why a scenario can't be used.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("cannot read scenario {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid scenario TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid scenario: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    pub site: Site,
    pub control: Control,
    pub arrivals: Arrivals,
    pub ev: Ev,
    /// What the controller assumes for EVs that don't report their needs.
    pub defaults: Defaults,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Site {
    pub stations: u32,
    pub evses_per_station: u32,
    pub evse_max_w: f64,
    pub grid_limit_w: f64,
    /// The TxDefaultProfile limit: what an EVSE may draw without a plan.
    pub fallback_limit_w: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub slot_secs: u32,
    pub max_horizon_slots: usize,
    /// Re-plan at least this often; arrivals and departures also trigger a plan.
    pub tick_secs: u32,
    /// Simulation time step. Must divide `slot_secs` and `tick_secs`.
    #[serde(default = "default_step_secs")]
    pub step_secs: u32,
    #[serde(default = "default_min_change_w")]
    pub min_change_w: f64,
    #[serde(default = "default_rel_change")]
    pub rel_change: f64,
}

fn default_step_secs() -> u32 {
    10
}

fn default_min_change_w() -> f64 {
    ControlConfig::default().min_change_w
}

fn default_rel_change() -> f64 {
    ControlConfig::default().rel_change
}

impl Control {
    pub fn config(&self) -> ControlConfig {
        ControlConfig {
            min_change_w: self.min_change_w,
            rel_change: self.rel_change,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arrivals {
    /// Expected EVs per hour, for each hour of the day (UTC), on weekdays.
    pub rate_per_hour: Vec<f64>,
    /// Multiplies the rate on Saturdays and Sundays.
    pub weekend_factor: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ev {
    /// Time from arrival to departure. The driver leaves then, whether or not the EV
    /// has charged.
    pub dwell_secs: TruncatedLogNormal,
    /// Energy the driver wants.
    pub energy_wh: TruncatedLogNormal,
    /// The EV's own max charging power, as a weighted mix.
    pub max_power_w: Vec<Weighted>,
    /// Share of EVs that report their needs over ISO 15118.
    pub iso15118_share: f64,
    /// Whether EVs leave as soon as they are full (a fast-charging hub) or stay until
    /// their departure (a workplace car park).
    pub leave_when_full: bool,
    /// How long a driver waits for a free EVSE before giving up.
    pub max_wait_secs: i64,
}

/// A lognormal distribution with the given median and log-space standard deviation,
/// restricted to `[min, max]`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TruncatedLogNormal {
    pub median: f64,
    pub sigma: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weighted {
    pub value: f64,
    pub weight: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub dwell_secs: i64,
    pub energy_wh: f64,
}

impl Defaults {
    pub fn needs(&self) -> NeedsDefaults {
        NeedsDefaults {
            dwell_secs: self.dwell_secs,
            energy_wh: self.energy_wh,
        }
    }
}

impl Scenario {
    /// The names of the built-in presets.
    pub fn preset_names() -> impl Iterator<Item = &'static str> {
        PRESETS.iter().map(|(name, _)| *name)
    }

    /// Loads a built-in preset by name, or else a TOML file at that path.
    pub fn load(name_or_path: &str) -> Result<Self, ScenarioError> {
        if let Some((_, toml)) = PRESETS.iter().find(|(name, _)| *name == name_or_path) {
            return Self::from_toml(toml);
        }
        let path = Path::new(name_or_path);
        let toml = std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml(&toml)
    }

    /// Parses and validates a scenario.
    pub fn from_toml(toml: &str) -> Result<Self, ScenarioError> {
        let scenario: Self = toml::from_str(toml)?;
        scenario.validate()?;
        Ok(scenario)
    }

    /// Number of EVSEs on the site.
    pub fn evse_count(&self) -> usize {
        (self.site.stations * self.site.evses_per_station) as usize
    }

    fn validate(&self) -> Result<(), ScenarioError> {
        let fail = |msg: String| Err(ScenarioError::Invalid(msg));
        let site = &self.site;
        let control = &self.control;
        if self.evse_count() == 0 {
            return fail("the site needs at least one EVSE".into());
        }
        let powers = [
            ("evse_max_w", site.evse_max_w),
            ("grid_limit_w", site.grid_limit_w),
            ("fallback_limit_w", site.fallback_limit_w),
        ];
        if let Some((name, _)) = powers.iter().find(|(_, w)| !(w.is_finite() && *w >= 0.0)) {
            return fail(format!("{name} must be a non-negative number"));
        }
        if control.slot_secs == 0 || control.tick_secs == 0 || control.step_secs == 0 {
            return fail("slot_secs, tick_secs and step_secs must be positive".into());
        }
        if !control.slot_secs.is_multiple_of(control.step_secs)
            || !control.tick_secs.is_multiple_of(control.step_secs)
        {
            return fail("step_secs must divide slot_secs and tick_secs".into());
        }
        if control.max_horizon_slots == 0 {
            return fail("max_horizon_slots must be positive".into());
        }
        let rates = &self.arrivals.rate_per_hour;
        if rates.len() != 24 {
            return fail(format!(
                "rate_per_hour needs 24 values, got {}",
                rates.len()
            ));
        }
        if rates
            .iter()
            .chain([&self.arrivals.weekend_factor])
            .any(|r| !(r.is_finite() && *r >= 0.0))
        {
            return fail("arrival rates and weekend_factor must be non-negative".into());
        }
        for (name, d) in [
            ("dwell_secs", &self.ev.dwell_secs),
            ("energy_wh", &self.ev.energy_wh),
        ] {
            if !(d.median > 0.0 && d.sigma >= 0.0 && 0.0 <= d.min && d.min <= d.max) {
                return fail(format!(
                    "{name} needs median > 0, sigma >= 0 and 0 <= min <= max"
                ));
            }
        }
        let mix = &self.ev.max_power_w;
        if mix.is_empty()
            || mix.iter().any(|w| !(w.value > 0.0 && w.weight >= 0.0))
            || mix.iter().all(|w| w.weight == 0.0)
        {
            return fail("max_power_w needs positive values and some positive weight".into());
        }
        if !(0.0..=1.0).contains(&self.ev.iso15118_share) {
            return fail("iso15118_share must be between 0 and 1".into());
        }
        if self.ev.max_wait_secs < 0 || self.defaults.dwell_secs <= 0 {
            return fail("max_wait_secs must be >= 0 and defaults.dwell_secs > 0".into());
        }
        if !(self.defaults.energy_wh.is_finite() && self.defaults.energy_wh >= 0.0) {
            return fail("defaults.energy_wh must be non-negative".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_parse() {
        for name in Scenario::preset_names() {
            let s = Scenario::load(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(s.name, name);
        }
        let workplace = Scenario::load("workplace").unwrap();
        assert_eq!(workplace.evse_count(), 20);
        assert_eq!(workplace.site.grid_limit_w, 150_000.0);
        let hub = Scenario::load("hub").unwrap();
        assert_eq!(hub.evse_count(), 8);
        assert_eq!(hub.site.grid_limit_w, 1_000_000.0);
        assert!(hub.ev.leave_when_full);
    }

    #[test]
    fn missing_file_is_an_io_error() {
        assert!(matches!(
            Scenario::load("no/such/scenario.toml"),
            Err(ScenarioError::Io { .. })
        ));
    }

    fn workplace_with(from: &str, to: &str) -> Result<Scenario, ScenarioError> {
        let toml = PRESETS[0].1;
        assert!(toml.contains(from), "preset lacks {from:?}");
        Scenario::from_toml(&toml.replacen(from, to, 1))
    }

    #[test]
    fn rejects_bad_values() {
        assert!(matches!(
            workplace_with("step_secs = 10", "step_secs = 7"),
            Err(ScenarioError::Invalid(_))
        ));
        assert!(matches!(
            workplace_with("iso15118_share = 0.3", "iso15118_share = 1.5"),
            Err(ScenarioError::Invalid(_))
        ));
        assert!(matches!(
            workplace_with("rate_per_hour = [", "rate_per_hour = [1.0, "),
            Err(ScenarioError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(matches!(
            workplace_with("[site]", "[site]\ncolour = \"red\""),
            Err(ScenarioError::Parse(_))
        ));
    }
}
