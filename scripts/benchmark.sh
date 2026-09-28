#!/usr/bin/env bash
# Runs the offline benchmark behind docs/benchmark.md:
#
# - every scheduler on both preset scenarios, over SEEDS seeds of DAYS days each;
# - a value-of-information sweep: the share of EVs that report their needs (ISO 15118),
#   on the workplace scenario.
#
# The summaries go to docs/benchmark/ (summary.csv, voi.csv); the raw per-run files and the
# time series of seed 1 go to target/benchmark/. Every run is deterministic, so a rerun gives
# the same CSVs except for the solve times.
#
# Usage: scripts/benchmark.sh   (SEEDS, DAYS and JOBS override the defaults)
set -euo pipefail

cd "$(dirname "$0")/.."
seeds=${SEEDS:-10}
days=${DAYS:-7}
jobs=${JOBS:-$(getconf _NPROCESSORS_ONLN)}
out=target/benchmark
docs=docs/benchmark

cargo build --release --quiet -p simulator
rm -rf "$out"
mkdir -p "$out/runs" "$docs"

# One line per run: kind scenario scheduler seed share ("-" for the scenario's own).
{
    for scenario in workplace hub; do
        for scheduler in equal edf lp; do
            for seed in $(seq "$seeds"); do
                echo "main $scenario $scheduler $seed -"
            done
        done
    done
    for share in 0 0.25 0.5 0.75 1; do
        for scheduler in equal edf lp; do
            for seed in $(seq "$seeds"); do
                echo "voi workplace $scheduler $seed $share"
            done
        done
    done
} >"$out/runs.txt"

run() {
    local kind=$1 scenario=$2 scheduler=$3 seed=$4 share=$5
    local tag="$kind-$scenario-$scheduler-$seed-$share"
    local args=(offline --scenario "$scenario" --scheduler "$scheduler" --seed "$seed"
        --days "$days" --summary "$out/runs/$tag.csv")
    if [[ $share != - ]]; then
        args+=(--iso15118-share "$share")
    fi
    if [[ $kind == main && $seed == 1 ]]; then
        args+=(--timeseries "$out/timeseries-$scenario-$scheduler.csv")
    fi
    target/release/sim "${args[@]}" >/dev/null
}
export -f run
export out days

echo "running $(wc -l <"$out/runs.txt") simulations on $jobs threads" >&2
xargs -P "$jobs" -L 1 bash -c 'run "$@"' _ <"$out/runs.txt"

# One header, then the rows sorted by scenario, scheduler, ISO 15118 share and seed.
collect() {
    local kind=$1 dest=$2
    local files=("$out/runs/$kind"-*.csv)
    head -n 1 "${files[0]}" >"$dest"
    tail -q -n +2 "${files[@]}" | sort -t, -k1,1 -k2,2 -k5,5g -k3,3n >>"$dest"
}
collect main "$docs/summary.csv"
collect voi "$docs/voi.csv"
echo "wrote $docs/summary.csv and $docs/voi.csv" >&2
