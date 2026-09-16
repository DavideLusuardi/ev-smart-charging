//! The `sim` binary.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::builder::PossibleValuesParser;
use clap::{Args, Parser, Subcommand};
use simulator::arrivals::generate;
use simulator::engine::{SCHEDULERS, run, scheduler_by_name};
use simulator::report::{RunInfo, Summary, append_summary, write_sessions, write_timeseries};
use simulator::scenario::Scenario;

#[derive(Parser)]
#[command(version, about = "EV charging-station simulator")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Simulate a scenario offline, as fast as the planner allows.
    Offline(OfflineArgs),
}

#[derive(Args)]
struct OfflineArgs {
    /// A built-in scenario (workplace, hub) or the path to a scenario TOML file.
    #[arg(long)]
    scenario: String,
    /// The scheduling strategy.
    #[arg(long, value_parser = PossibleValuesParser::new(SCHEDULERS))]
    scheduler: String,
    /// Seed for the EV arrivals.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Days of arrivals to simulate; the run goes on until the last EV has left.
    #[arg(long, default_value_t = 1)]
    days: u32,
    /// Share of EVs that report their needs over ISO 15118 (default: the scenario's).
    #[arg(long, value_parser = parse_share)]
    iso15118_share: Option<f64>,
    /// Write one CSV row per EV to this file.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Write one CSV row per time step to this file.
    #[arg(long)]
    timeseries: Option<PathBuf>,
    /// Append the summary as a CSV row to this file.
    #[arg(long)]
    summary: Option<PathBuf>,
}

fn parse_share(s: &str) -> Result<f64, String> {
    let share: f64 = s.parse().map_err(|e| format!("{e}"))?;
    if (0.0..=1.0).contains(&share) {
        Ok(share)
    } else {
        Err("must be between 0 and 1".into())
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Offline(args) => offline(&args),
    }
}

fn offline(args: &OfflineArgs) -> Result<()> {
    let scenario = Scenario::load(&args.scenario)?;
    let iso15118_share = args.iso15118_share.unwrap_or(scenario.ev.iso15118_share);
    let evs = generate(&scenario, args.seed, args.days, iso15118_share);
    let scheduler = scheduler_by_name(&args.scheduler).expect("clap checked the name");
    let result = run(&scenario, scheduler, &evs)?;

    let info = RunInfo {
        scenario: scenario.name.clone(),
        scheduler: args.scheduler.clone(),
        seed: args.seed,
        days: args.days,
        iso15118_share,
    };
    let summary = Summary::new(&info, scenario.site.grid_limit_w, &result);
    println!("{summary}");

    if let Some(path) = &args.out {
        write_sessions(create(path)?, &result.sessions)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    if let Some(path) = &args.timeseries {
        write_timeseries(create(path)?, &result.timeseries)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    if let Some(path) = &args.summary {
        append_summary(path, &summary).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

fn create(path: &Path) -> Result<BufWriter<File>> {
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    Ok(BufWriter::new(file))
}
