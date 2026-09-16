//! Run results as CSV files and a summary table.

use std::fmt;
use std::fs::OpenOptions;
use std::io;
use std::path::Path;

use serde::Serialize;

use crate::engine::{RunResult, Sample, SessionOutcome};

/// What a run is, for the summary.
#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    pub scenario: String,
    pub scheduler: String,
    pub seed: u64,
    pub days: u32,
    pub iso15118_share: f64,
}

/// One row per run: the benchmark metrics.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub scenario: String,
    pub scheduler: String,
    pub seed: u64,
    pub days: u32,
    pub iso15118_share: f64,
    pub arrived: usize,
    pub plugged_in: usize,
    /// EVs that had to wait for a free EVSE, including those that gave up.
    pub queued: usize,
    pub gave_up: usize,
    /// Over the EVs that waited.
    pub mean_wait_secs: f64,
    pub max_wait_secs: i64,
    pub requested_wh: f64,
    /// What the EVs could physically have taken while plugged in.
    pub deliverable_wh: f64,
    pub delivered_wh: f64,
    pub satisfied: usize,
    pub peak_power_w: f64,
    pub limit_w: f64,
    pub plans: usize,
    pub mean_solve_secs: f64,
    pub p95_solve_secs: f64,
    pub profiles_sent: usize,
}

impl Summary {
    pub fn new(info: &RunInfo, limit_w: f64, result: &RunResult) -> Self {
        let sessions = &result.sessions;
        let waits: Vec<i64> = sessions
            .iter()
            .map(SessionOutcome::wait_secs)
            .filter(|&w| w > 0)
            .collect();
        let sum = |f: fn(&SessionOutcome) -> f64| sessions.iter().map(f).sum::<f64>();
        Self {
            scenario: info.scenario.clone(),
            scheduler: info.scheduler.clone(),
            seed: info.seed,
            days: info.days,
            iso15118_share: info.iso15118_share,
            arrived: sessions.len(),
            plugged_in: sessions.iter().filter(|o| !o.gave_up()).count(),
            queued: waits.len(),
            gave_up: sessions.iter().filter(|o| o.gave_up()).count(),
            mean_wait_secs: mean(waits.iter().map(|&w| w as f64)),
            max_wait_secs: waits.iter().copied().max().unwrap_or(0),
            requested_wh: sum(|o| o.ev.energy_wh),
            deliverable_wh: sum(SessionOutcome::max_deliverable_wh),
            delivered_wh: sum(|o| o.delivered_wh),
            satisfied: sessions.iter().filter(|o| o.satisfied()).count(),
            peak_power_w: result.peak_power_w(),
            limit_w,
            plans: result.solve_secs.len(),
            mean_solve_secs: mean(result.solve_secs.iter().copied()),
            p95_solve_secs: percentile(&result.solve_secs, 0.95),
            profiles_sent: result.profiles_sent,
        }
    }
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kwh = |wh: f64| wh / 1e3;
        let pct = |part: f64, whole: f64| {
            if whole > 0.0 {
                100.0 * part / whole
            } else {
                100.0
            }
        };
        writeln!(
            f,
            "scenario   {} (seed {}, {} days, ISO 15118 share {:.0} %)",
            self.scenario,
            self.seed,
            self.days,
            100.0 * self.iso15118_share
        )?;
        writeln!(f, "scheduler  {}", self.scheduler)?;
        writeln!(
            f,
            "EVs        {} arrived, {} plugged in, {} queued, {} gave up",
            self.arrived, self.plugged_in, self.queued, self.gave_up
        )?;
        writeln!(
            f,
            "waiting    mean {:.1} min, max {:.1} min (EVs that queued)",
            self.mean_wait_secs / 60.0,
            self.max_wait_secs as f64 / 60.0
        )?;
        writeln!(
            f,
            "energy     {:.1} kWh requested, {:.1} deliverable, {:.1} delivered",
            kwh(self.requested_wh),
            kwh(self.deliverable_wh),
            kwh(self.delivered_wh)
        )?;
        writeln!(
            f,
            "           {:.2} % of requested, {:.2} % of deliverable",
            pct(self.delivered_wh, self.requested_wh),
            pct(self.delivered_wh, self.deliverable_wh)
        )?;
        writeln!(f, "satisfied  {} of {} EVs", self.satisfied, self.arrived)?;
        writeln!(
            f,
            "power      peak {:.1} kW, limit {:.1} kW",
            self.peak_power_w / 1e3,
            self.limit_w / 1e3
        )?;
        writeln!(
            f,
            "planner    {} plans, solve mean {:.3} ms, p95 {:.3} ms",
            self.plans,
            self.mean_solve_secs * 1e3,
            self.p95_solve_secs * 1e3
        )?;
        write!(
            f,
            "profiles   {} SetChargingProfile sent",
            self.profiles_sent
        )
    }
}

#[derive(Serialize)]
struct SessionRow<'a> {
    id: &'a str,
    iso15118: bool,
    arrival_secs: i64,
    plugged_secs: Option<i64>,
    left_secs: i64,
    departure_secs: i64,
    wait_secs: i64,
    gave_up: bool,
    ev_max_w: f64,
    requested_wh: f64,
    deliverable_wh: f64,
    delivered_wh: f64,
    satisfied: bool,
}

/// One row per EV. Times are unix seconds; `plugged_secs` is empty for EVs that gave up.
pub fn write_sessions(out: impl io::Write, sessions: &[SessionOutcome]) -> csv::Result<()> {
    let mut w = csv::Writer::from_writer(out);
    for o in sessions {
        w.serialize(SessionRow {
            id: &o.ev.id,
            iso15118: o.ev.iso15118,
            arrival_secs: o.ev.arrival_secs,
            plugged_secs: o.plugged_secs,
            left_secs: o.left_secs,
            departure_secs: o.ev.departure_secs,
            wait_secs: o.wait_secs(),
            gave_up: o.gave_up(),
            ev_max_w: o.ev.max_power_w,
            requested_wh: o.ev.energy_wh,
            deliverable_wh: o.max_deliverable_wh(),
            delivered_wh: o.delivered_wh,
            satisfied: o.satisfied(),
        })?;
    }
    w.flush()?;
    Ok(())
}

#[derive(Serialize)]
struct SampleRow {
    t_secs: i64,
    power_w: f64,
    limit_w: f64,
    sessions: usize,
    queued: usize,
}

/// One row per time step.
pub fn write_timeseries(out: impl io::Write, samples: &[Sample]) -> csv::Result<()> {
    let mut w = csv::Writer::from_writer(out);
    for s in samples {
        w.serialize(SampleRow {
            t_secs: s.t_secs,
            power_w: s.power_w,
            limit_w: s.limit_w,
            sessions: s.sessions,
            queued: s.queued,
        })?;
    }
    w.flush()?;
    Ok(())
}

/// Appends the summary as one CSV row, writing the header first if the file is new or
/// empty, so several runs can collect into one file.
pub fn append_summary(path: &Path, summary: &Summary) -> csv::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let empty = file.metadata()?.len() == 0;
    let mut w = csv::WriterBuilder::new()
        .has_headers(empty)
        .from_writer(file);
    w.serialize(summary)?;
    w.flush()?;
    Ok(())
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = values.fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if n == 0 { 0.0 } else { sum / n as f64 }
}

/// Nearest-rank percentile: the smallest value with at least `q` of the values at or
/// below it.
fn percentile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrivals::{EvArrival, generate};
    use crate::engine::{run, scheduler_by_name};
    use crate::scenario::Scenario;

    fn outcome(id: &str, plugged: Option<i64>, delivered_wh: f64) -> SessionOutcome {
        SessionOutcome {
            ev: EvArrival {
                id: id.into(),
                arrival_secs: 0,
                departure_secs: 7_200,
                energy_wh: 10_000.0,
                max_power_w: 11_000.0,
                iso15118: false,
            },
            plugged_secs: plugged,
            left_secs: plugged.map_or(600, |_| 7_200),
            delivered_wh,
            evse_max_w: 22_000.0,
        }
    }

    fn info() -> RunInfo {
        RunInfo {
            scenario: "test".into(),
            scheduler: "edf".into(),
            seed: 1,
            days: 1,
            iso15118_share: 0.3,
        }
    }

    #[test]
    fn percentile_uses_the_nearest_rank() {
        let v: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.95), 19.0);
        assert_eq!(percentile(&v, 1.0), 20.0);
        assert_eq!(percentile(&[3.0], 0.95), 3.0);
        assert_eq!(percentile(&[], 0.95), 0.0);
    }

    #[test]
    fn summary_counts_evs_and_energy() {
        let result = RunResult {
            sessions: vec![
                outcome("a", Some(0), 10_000.0),
                outcome("b", Some(1_800), 4_000.0),
                outcome("c", None, 0.0),
            ],
            timeseries: vec![],
            solve_secs: vec![0.001, 0.003],
            profiles_sent: 5,
        };
        let s = Summary::new(&info(), 50_000.0, &result);
        assert_eq!((s.arrived, s.plugged_in, s.queued, s.gave_up), (3, 2, 2, 1));
        assert_eq!(s.mean_wait_secs, 1_200.0);
        assert_eq!(s.max_wait_secs, 1_800);
        assert_eq!(s.satisfied, 1);
        assert_eq!(s.requested_wh, 30_000.0);
        assert_eq!(s.delivered_wh, 14_000.0);
        // a: 10 kWh, b: 11 kW for 1.5 h caps at 10 kWh, c: nothing.
        assert_eq!(s.deliverable_wh, 20_000.0);
        assert_eq!(s.plans, 2);
        assert!((s.mean_solve_secs - 0.002).abs() < 1e-12);
        assert_eq!(s.p95_solve_secs, 0.003);
    }

    #[test]
    fn session_csv_has_a_row_per_ev() {
        let mut out = Vec::new();
        write_sessions(
            &mut out,
            &[outcome("a", Some(0), 10_000.0), outcome("c", None, 0.0)],
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("id,iso15118,arrival_secs,plugged_secs,"));
        assert!(lines[2].starts_with("c,false,0,,600,"));
    }

    #[test]
    fn same_seed_same_session_csv() {
        let scenario = Scenario::load("hub").unwrap();
        let csv = || {
            let evs = generate(&scenario, 9, 1, scenario.ev.iso15118_share);
            let result = run(&scenario, scheduler_by_name("edf").unwrap(), &evs).unwrap();
            let mut out = Vec::new();
            write_sessions(&mut out, &result.sessions).unwrap();
            out
        };
        assert_eq!(csv(), csv());
    }

    #[test]
    fn summaries_append_under_one_header() {
        let path = std::env::temp_dir().join(format!("sim-summary-{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let result = RunResult {
            sessions: vec![outcome("a", Some(0), 10_000.0)],
            timeseries: vec![],
            solve_secs: vec![],
            profiles_sent: 0,
        };
        let s = Summary::new(&info(), 50_000.0, &result);
        append_summary(&path, &s).unwrap();
        append_summary(&path, &s).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("scenario,scheduler,seed,"));
        assert_eq!(lines[1], lines[2]);
    }
}
