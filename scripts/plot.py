#!/usr/bin/env python3
"""Plots the benchmark results and prints the tables for docs/benchmark.md.

Reads docs/benchmark/summary.csv and voi.csv, and the seed-1 time series in
target/benchmark/, all written by scripts/benchmark.sh. Writes the PNGs to docs/benchmark/
and the markdown tables to stdout.

Usage: python3 scripts/plot.py   (needs matplotlib: pip install -r scripts/requirements.txt)
"""

import csv
import statistics
import tomllib
from collections import defaultdict
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs" / "benchmark"
RAW = ROOT / "target" / "benchmark"

SCENARIOS = ["workplace", "hub"]
SCHEDULERS = ["equal", "edf", "lp"]
# Categorical slots 1-3 of the reference palette, validated all-pairs on the light surface.
COLOR = {"equal": "#2a78d6", "edf": "#eb6834", "lp": "#1baf7a"}
SURFACE = "#fcfcfb"
INK = "#0b0b0b"
INK_2 = "#52514e"
GRID = "#e4e3df"
# The first timestamp of every run: Monday 2026-01-05 00:00 UTC.
START_SECS = 1_767_571_200

plt.rcParams.update(
    {
        "figure.facecolor": SURFACE,
        "axes.facecolor": SURFACE,
        "savefig.facecolor": SURFACE,
        "axes.edgecolor": GRID,
        "axes.labelcolor": INK_2,
        "axes.titlecolor": INK,
        "axes.titlesize": 11,
        "axes.titleweight": "bold",
        "axes.labelsize": 9,
        "axes.grid": True,
        "axes.axisbelow": True,
        "axes.spines.top": False,
        "axes.spines.right": False,
        "grid.color": GRID,
        "grid.linewidth": 0.8,
        "xtick.color": INK_2,
        "ytick.color": INK_2,
        "xtick.labelsize": 9,
        "ytick.labelsize": 9,
        "legend.frameon": False,
        "legend.fontsize": 9,
        "font.size": 9,
        "lines.solid_capstyle": "round",
        "lines.solid_joinstyle": "round",
    }
)


def read(path):
    with open(path, newline="") as f:
        return list(csv.DictReader(f))


def pct(num, den):
    return 100.0 * num / den if den else 100.0


def metrics(row):
    """The derived measures of one summary row."""
    return {
        "delivered": pct(float(row["delivered_wh"]), float(row["deliverable_wh"])),
        "satisfied": pct(float(row["satisfied"]), float(row["plugged_in"])),
        "gave_up": float(row["gave_up"]),
        "profiles": float(row["profiles_sent"]),
        "plans": float(row["plans"]),
        "mean_ms": 1000 * float(row["mean_solve_secs"]),
        "p95_ms": 1000 * float(row["p95_solve_secs"]),
        "peak_kw": float(row["peak_power_w"]) / 1000,
    }


def by_run(rows):
    """{(scenario, scheduler): {seed: metrics}}"""
    out = defaultdict(dict)
    for r in rows:
        out[(r["scenario"], r["scheduler"])][int(r["seed"])] = metrics(r)
    return out


def column(runs, key):
    return [m[key] for _, m in sorted(runs.items())]


def mean_sd(values):
    return statistics.mean(values), statistics.stdev(values) if len(values) > 1 else 0.0


def save(fig, name):
    fig.savefig(DOCS / name, dpi=160, bbox_inches="tight")
    plt.close(fig)


def plot_results(data):
    """Delivered energy and satisfied EVs, one dot per seed, same seeds joined."""
    measures = [
        ("delivered", "Energy delivered, % of deliverable"),
        ("satisfied", "EVs satisfied, %"),
    ]
    fig, axes = plt.subplots(2, 2, figsize=(8, 6.4))
    for col, scenario in enumerate(SCENARIOS):
        for row, (key, label) in enumerate(measures):
            ax = axes[row][col]
            seeds = sorted(data[(scenario, SCHEDULERS[0])])
            for seed in seeds:
                ax.plot(
                    range(len(SCHEDULERS)),
                    [data[(scenario, s)][seed][key] for s in SCHEDULERS],
                    color=GRID,
                    linewidth=1,
                    zorder=1,
                )
            for x, s in enumerate(SCHEDULERS):
                values = column(data[(scenario, s)], key)
                ax.scatter(
                    [x] * len(values),
                    values,
                    s=36,
                    color=COLOR[s],
                    edgecolor=SURFACE,
                    linewidth=1.5,
                    zorder=3,
                )
                m = statistics.mean(values)
                ax.plot([x - 0.22, x + 0.22], [m, m], color=INK, linewidth=2, zorder=4)
                ax.annotate(
                    f"{m:.2f}",
                    (x + 0.26, m),
                    va="center",
                    fontsize=8,
                    color=INK_2,
                )
            ax.set_xticks(range(len(SCHEDULERS)), SCHEDULERS)
            ax.set_xlim(-0.5, len(SCHEDULERS) - 0.2)
            ax.grid(axis="x", visible=False)
            ax.set_ylabel(label)
            if row == 0:
                ax.set_title(scenario)
    fig.text(
        0.5,
        -0.01,
        "Dots: one 7-day run per seed (gray lines join the same arrivals); black ticks: mean over 10 seeds.",
        ha="center",
        color=INK_2,
        fontsize=8,
    )
    fig.tight_layout()
    save(fig, "results.png")


def plot_solve_times(data):
    """Mean and p95 time per plan, log scale."""
    fig, ax = plt.subplots(figsize=(8, 2.8))
    labels = []
    y = 0
    for scenario in SCENARIOS:
        for s in SCHEDULERS:
            runs = data[(scenario, s)]
            mean = statistics.mean(column(runs, "mean_ms"))
            p95 = statistics.mean(column(runs, "p95_ms"))
            ax.plot([mean, p95], [y, y], color=COLOR[s], linewidth=2, zorder=2)
            ax.scatter([mean], [y], s=48, color=COLOR[s], edgecolor=SURFACE, lw=1.5, zorder=3)
            ax.scatter(
                [p95], [y], s=48, facecolor=SURFACE, edgecolor=COLOR[s], lw=2, zorder=3
            )
            ax.annotate(f"{p95:.3g} ms", (p95 * 1.25, y), va="center", fontsize=8, color=INK_2)
            labels.append(f"{scenario} · {s}")
            y += 1
        y += 0.6
    ticks = [i + (i // 3) * 0.6 for i in range(len(labels))]
    ax.set_yticks(ticks, labels)
    ax.invert_yaxis()
    ax.set_xscale("log")
    ax.set_xlim(right=ax.get_xlim()[1] * 4)
    ax.grid(axis="y", visible=False)
    ax.set_xlabel("Time per plan, ms (log scale): filled = mean, hollow = 95th percentile")
    ax.set_title("Planner solve time", loc="left")
    fig.tight_layout()
    save(fig, "solve-time.png")


def read_timeseries(scenario, scheduler):
    """Per minute: hours since start, mean site power in kW, and the capacity left to plan
    in kW (the grid limit minus the fallback reserve of the EVSEs with no EV); and the grid
    limit in kW."""
    site = tomllib.loads((ROOT / "config" / "scenarios" / f"{scenario}.toml").read_text())["site"]
    evses = site["stations"] * site["evses_per_station"]
    rows = read(RAW / f"timeseries-{scenario}-{scheduler}.csv")
    power = defaultdict(list)
    plannable = defaultdict(list)
    for r in rows:
        minute = (int(r["t_secs"]) - START_SECS) // 60
        limit_w = float(r["limit_w"])
        power[minute].append(float(r["power_w"]))
        idle = evses - int(r["sessions"])
        plannable[minute].append(limit_w - idle * site["fallback_limit_w"])
    t = sorted(power)
    return (
        [m / 60 for m in t],
        [statistics.mean(power[m]) / 1000 for m in t],
        [min(plannable[m]) / 1000 for m in t],
        float(rows[0]["limit_w"]) / 1000,
    )


def busiest_day(hours, power):
    """The day with the most energy delivered."""
    energy = defaultdict(float)
    for h, p in zip(hours, power):
        energy[int(h // 24)] += p
    return max(energy, key=energy.get)


def plot_power(scenario):
    """Site power against the limit over the busiest day, one panel per scheduler."""
    series = {s: read_timeseries(scenario, s) for s in SCHEDULERS}
    day = busiest_day(*series["edf"][:2])
    fig, axes = plt.subplots(len(SCHEDULERS), 1, figsize=(8, 5.4), sharex=True, sharey=True)
    for ax, s in zip(axes, SCHEDULERS):
        hours, power, plannable, limit = series[s]
        points = [
            (h - 24 * day, p, c)
            for h, p, c in zip(hours, power, plannable)
            if 24 * day <= h < 24 * (day + 1)
        ]
        xs, ys, cs = zip(*points)
        ax.fill_between(xs, ys, color=COLOR[s], alpha=0.12, linewidth=0)
        ax.plot(xs, ys, color=COLOR[s], linewidth=1.5)
        ax.axhline(limit, color=INK_2, linewidth=1)
        ax.plot(xs, cs, color=INK_2, linewidth=0.8, linestyle=(0, (1, 1.5)))
        ax.annotate("grid limit", (0.2, limit), xytext=(0, 3), textcoords="offset points",
                    fontsize=8, color=INK_2)
        ax.set_ylabel(f"{s}\nkW")
        ax.set_ylim(0, limit * 1.12)
        ax.set_xlim(0, 24)
    axes[-1].set_xticks(range(0, 25, 3))
    axes[-1].set_xlabel(
        "Hour of the day (UTC), 1-minute averages. Dotted: grid limit minus the fallback "
        "reserve of the EVSEs with no EV"
    )
    weekday = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
    axes[0].set_title(
        f"{scenario}: site power on its busiest day (seed 1, a {weekday[day % 7]})", loc="left"
    )
    fig.tight_layout()
    save(fig, f"power-{scenario}.png")
    return day


def plot_voi(rows):
    """Delivered energy against the share of EVs that report their needs."""
    by = defaultdict(list)
    for r in rows:
        by[(r["scheduler"], float(r["iso15118_share"]))].append(metrics(r)["delivered"])
    shares = sorted({share for _, share in by})
    fig, ax = plt.subplots(figsize=(8, 3.6))
    for s in SCHEDULERS:
        means = [statistics.mean(by[(s, x)]) for x in shares]
        xs = [100 * x for x in shares]
        ax.plot(xs, means, color=COLOR[s], linewidth=2, label=s)
        ax.scatter(xs, means, s=36, color=COLOR[s], edgecolor=SURFACE, lw=1.5, zorder=3)
    ax.set_xticks([100 * x for x in shares], [f"{100 * x:.0f} %" for x in shares])
    ax.set_xlim(-4, 104)
    ax.set_xlabel("EVs that report their needs (ISO 15118)")
    ax.set_ylabel("Energy delivered,\n% of deliverable")
    ax.set_title("workplace: value of knowing each EV's needs", loc="left")
    ax.legend(loc="lower right", ncols=3, bbox_to_anchor=(1, 1.0))
    ax.text(0, -0.28, "Mean over 10 seeds of 7 days; the same arrivals at every share.",
            transform=ax.transAxes, fontsize=8, color=INK_2)
    fig.tight_layout()
    save(fig, "voi.png")
    return by


def results_table(data):
    print("| Scenario | Scheduler | Delivered, % of deliverable | EVs satisfied, % | Gave up waiting | Profiles sent | Mean plan time |")
    print("|---|---|---|---|---|---|---|")
    for scenario in SCENARIOS:
        for s in SCHEDULERS:
            runs = data[(scenario, s)]
            d, d_sd = mean_sd(column(runs, "delivered"))
            sat, sat_sd = mean_sd(column(runs, "satisfied"))
            print(
                f"| {scenario} | {s} | {d:.2f} ± {d_sd:.2f} | {sat:.1f} ± {sat_sd:.1f} "
                f"| {statistics.mean(column(runs, 'gave_up')):.1f} "
                f"| {statistics.mean(column(runs, 'profiles')):,.0f} "
                f"| {statistics.mean(column(runs, 'mean_ms')):.3f} ms |"
            )


def paired_table(data):
    print("| Scenario | Difference | Mean, pp | Lowest seed | Highest seed | Seeds where the first is ahead |")
    print("|---|---|---|---|---|---|")
    for scenario in SCENARIOS:
        for a, b in [("edf", "equal"), ("lp", "edf"), ("lp", "equal")]:
            seeds = sorted(data[(scenario, a)])
            diff = [
                data[(scenario, a)][k]["delivered"] - data[(scenario, b)][k]["delivered"]
                for k in seeds
            ]
            ahead = sum(x > 0 for x in diff)
            print(
                f"| {scenario} | {a} − {b} | {statistics.mean(diff):+.2f} | {min(diff):+.2f} "
                f"| {max(diff):+.2f} | {ahead} of {len(diff)} |"
            )


def voi_table(by):
    shares = sorted({share for _, share in by})
    print("| Scheduler | " + " | ".join(f"{100 * x:.0f} %" for x in shares) + " |")
    print("|---|" + "---|" * len(shares))
    for s in SCHEDULERS:
        cells = " | ".join(f"{statistics.mean(by[(s, x)]):.2f}" for x in shares)
        print(f"| {s} | {cells} |")


def main():
    data = by_run(read(DOCS / "summary.csv"))
    plot_results(data)
    plot_solve_times(data)
    days = {scenario: plot_power(scenario) for scenario in SCENARIOS}
    by = plot_voi(read(DOCS / "voi.csv"))

    print("## Results (mean ± standard deviation over seeds)\n")
    results_table(data)
    print("\n## Paired differences in delivered energy (same arrivals)\n")
    paired_table(data)
    print("\n## Value of information: delivered, % of deliverable (workplace, mean over seeds)\n")
    voi_table(by)
    print(f"\nBusiest days (0 = Monday): {days}")


if __name__ == "__main__":
    main()
