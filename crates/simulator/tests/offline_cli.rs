//! Runs the `sim` binary end to end.

use std::process::Command;

#[test]
fn offline_run_writes_its_outputs() {
    let dir = std::env::temp_dir().join(format!("sim-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sessions = dir.join("sessions.csv");
    let timeseries = dir.join("timeseries.csv");
    let summary = dir.join("summary.csv");

    let output = Command::new(env!("CARGO_BIN_EXE_sim"))
        .args([
            "offline",
            "--scenario",
            "hub",
            "--scheduler",
            "edf",
            "--seed",
            "3",
        ])
        .arg("--out")
        .arg(&sessions)
        .arg("--timeseries")
        .arg(&timeseries)
        .arg("--summary")
        .arg(&summary)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("scheduler  edf"), "{stdout}");

    let rows = |path| std::fs::read_to_string(path).unwrap().lines().count();
    assert!(rows(&sessions) > 100);
    assert!(rows(&timeseries) >= 24 * 360);
    assert_eq!(rows(&summary), 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rejects_unknown_schedulers_and_shares() {
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sim"))
            .args(args)
            .output()
            .unwrap()
    };
    assert!(
        !run(&["offline", "--scenario", "hub", "--scheduler", "fifo"])
            .status
            .success()
    );
    let share = run(&[
        "offline",
        "--scenario",
        "hub",
        "--scheduler",
        "edf",
        "--iso15118-share",
        "1.5",
    ]);
    assert!(!share.status.success());
    let missing = run(&[
        "offline",
        "--scenario",
        "nowhere.toml",
        "--scheduler",
        "edf",
    ]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("nowhere.toml"));
}
