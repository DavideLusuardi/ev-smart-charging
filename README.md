# ev-smart-charging

A Charging Station Management System (CSMS) in Rust that speaks OCPP 2.0.1 and shares a
site's limited grid power across connected EVs with a scheduling optimizer.

## Status

Work in progress, not yet released.

Working today:

- OCPP-J framing and the OCPP 2.0.1 messages needed for smart charging
- Three scheduling strategies and the receding-horizon planner
- An offline simulator that benchmarks them on generated EV arrivals
- The CSMS: station connections, boot, heartbeat, status and authorization, transactions and
  meter values, EV charging needs, and a control loop that sends charging profiles

## Crates

| Crate | Purpose |
|---|---|
| `crates/ocpp` | OCPP-J RPC framing and the OCPP 2.0.1 message subset used for smart charging |
| `crates/optimizer` | Pure scheduling library (equal share, EDF, LP) and the receding-horizon planner |
| `crates/csms` | Server binary: OCPP WebSocket endpoint, station and session state, control loop |
| `crates/simulator` | Binary `sim`: offline simulation of a site, to benchmark the schedulers |

## Building

You need:

- Rust 1.98.1. With [rustup](https://rustup.rs), `rust-toolchain.toml` installs it (with
  rustfmt and clippy) on the first `cargo` command.
- cmake and a C++ compiler, for the LP scheduler: its `lp` feature, on by default, builds the
  HiGHS solver from source. On Debian or Ubuntu: `sudo apt install cmake g++`.

```sh
cargo build --workspace
cargo test --workspace
```

Without cmake, build without the LP scheduler; `equal` and `edf` are still available:

```sh
cargo build --workspace --no-default-features
```

## Running the CSMS

```sh
cargo run -p csms -- --listen 0.0.0.0:9000
```

Stations connect to `ws://localhost:9000/ocpp/{stationId}` with the `ocpp2.0.1`
subprotocol. The CSMS tracks their sessions, plans the site every 30 s and after each event,
and sends every EVSE its share of the grid limit as a charging profile:

```sh
cargo run -p csms -- --grid-limit-w 150000 --evse-max-w 22000 --scheduler edf
```

`cargo run -p csms -- --help` lists the options (site limits, scheduler, slot length and
tick, heartbeat interval, call timeout, id token allowlist); `RUST_LOG=csms=debug` logs every
frame and plan. The supported messages, the defaults for EVs that don't report their needs,
and how the control loop stays within the limit are in
[docs/ocpp-subset.md](docs/ocpp-subset.md).

## Running the simulator

`sim offline` simulates a site with generated EV arrivals, as fast as the planner allows. The
simulated site runs the same planner and profile logic as the CSMS, with EVs that draw what
their profile allows.

```sh
cargo run --release -p simulator -- offline --scenario workplace --scheduler edf
```

Two scenarios are built in (their files are in [`config/scenarios/`](config/scenarios/)):

| Scenario | Site | EVs |
|---|---|---|
| `workplace` | 20 AC EVSEs of 22 kW, 100 kW grid limit | Arrive mostly 7 to 9 on weekdays and stay about 8 h, until their departure time |
| `hub` | 8 DC outlets of 300 kW, 1 MW grid limit | Arrive all day, stay 15 to 45 min, leave as soon as they are full; drivers queue for a free outlet |

For your own scenario, copy one of those files, change it, and pass its path to `--scenario`.
A scenario sets the site (EVSEs, their rating, the grid and fallback limits), the control loop
(slot length, horizon, tick), the arrival rate for each hour of the day, the distributions of
dwell time, energy and EV max power, and the defaults assumed for EVs that don't report their
needs. The fields are documented in
[`crates/simulator/src/scenario.rs`](crates/simulator/src/scenario.rs).

Options:

| Option | Default | Meaning |
|---|---|---|
| `--scenario` | required | `workplace`, `hub`, or the path to a scenario file |
| `--scheduler` | required | `equal`, `edf` or `lp` (`lp` needs the `lp` feature, on by default) |
| `--seed` | 1 | Seed for the arrivals: the same seed gives the same EVs |
| `--days` | 1 | Days of arrivals; the run continues until the last EV has left |
| `--iso15118-share` | the scenario's | Share of EVs (0 to 1) that report their needs; the others get the defaults |
| `--out` | none | CSV file with one row per EV |
| `--timeseries` | none | CSV file with one row per time step: site power, limit, EVs charging and EVs queued |
| `--summary` | none | CSV file to append the summary to, one row per run |

The summary printed at the end:

| Line | Meaning |
|---|---|
| `EVs` | EVs that arrived, got an EVSE, had to wait for one, and gave up waiting |
| `waiting` | Mean and longest wait for an EVSE, over the EVs that waited |
| `energy` | Energy requested by the drivers, deliverable, and delivered. *Deliverable* is the most any scheduler could have given each EV: its max power for as long as it was plugged in, capped by what it asked for. The percentage of deliverable is the fair score between schedulers |
| `satisfied` | EVs that got the energy their driver asked for |
| `power` | Peak site power against the grid limit |
| `planner` | Number of plans, and mean and 95th-percentile time to compute one |
| `profiles` | `SetChargingProfile` messages sent |

To compare the schedulers over several seeds, collect the summaries in one file:

```sh
for s in equal edf lp; do
  for seed in 1 2 3 4 5; do
    cargo run -q --release -p simulator -- offline --scenario hub --scheduler $s \
      --seed $seed --days 5 --summary results.csv
  done
done
```

## The optimizer

`crates/optimizer` decides how much power each EV gets. It is pure (no IO, no async), so the
CSMS and the offline simulator run exactly the same code.

On every control tick the **planner** takes the site state (the grid limit, the sessions
with their departure time, remaining energy and max power, and the idle EVSEs) and:

1. reserves each idle EVSE's fallback limit, so an EV that plugs in before the next plan
   can't push the site over its limit, and holds EVs that stopped taking power (full or
   paused) at that limit too, so they can start again;
2. splits time into fixed slots (5 min by default in the CSMS) from now up to the latest
   departure, capped at 144 slots (12 h; OCPP allows at most 1024 periods per profile);
3. asks a **scheduler** for a power per session and slot, within the site capacity, each
   EV's max power and its remaining energy;
4. turns each session's row into a step charging profile that ends with 0 W at departure.

Only the start of each plan is really used: the next tick (every 30 s in the CSMS, and after
each event) plans again from the new state (receding horizon), so arrivals, early departures
and wrong estimates are corrected within one tick. The simulator takes the slot length,
horizon and tick from the scenario: the `workplace` preset plans 5 min slots every 5 min, the
`hub` preset 1 min slots every minute, both also after each arrival, departure and full EV.

### Schedulers

Pick one with `--scheduler` on both the CSMS and the simulator.

| | `equal` | `edf` (default) | `lp` |
|---|---|---|---|
| Idea | Split each slot's capacity equally among the EVs present | Serve the EVs leaving soonest first | Solve a linear program over the whole horizon |
| Uses departures | No | Yes, to order EVs | Yes, as constraints |
| Looks ahead | No, slot by slot | No, slot by slot | Yes |
| Objective | Fairness | Urgency | Most energy delivered, as early as possible |
| Cost | Trivial | Trivial | A HiGHS solve per plan |

All three are work-conserving within a slot: capacity an EV can't use (because it is at its
max power or full) goes to the others.

- **Equal share** is the baseline and stands in for static load management. It is the
  fairest, but it ignores departures: an EV about to leave gets the same share as one staying
  all day, even when the latter could easily catch up later.
- **Earliest deadline first** gives the whole slot to the most urgent EVs (ties: the one
  needing more energy). Without per-EV power limits it delivers the maximum energy. With
  them it can fill an urgent EV and leave a slow one that can't make up the lost time later.
- **LP** plans every slot at once: it finds the schedule that delivers the most energy
  over the horizon and, among those, the one that delivers it earliest. See
  [The LP model](#the-lp-model).

Two small examples, with 1 h slots:

| Instance | `equal` | `edf` | `lp` |
|---|---|---|---|
| 10 W site, 3 slots. A: leaves after 2, 10 W max, needs 10 Wh. B: leaves after 3, 5 W max, needs 15 Wh | 25 Wh | **20 Wh**: A takes all of slot 0, B is capped at 5 W afterwards | 25 Wh |
| 10 W site, 2 slots. A: leaves after 1, needs 10 Wh. B: leaves after 2, needs 10 Wh. Both 10 W max | **15 Wh**: A gets only half before leaving | 20 Wh | 20 Wh |

For the sessions known at planning time the LP never delivers less than the heuristics (a
property test checks this). It pulls ahead when the site is congested and EVs differ in both
urgency and max power. It doesn't help when capacity is ample (everyone charges at full
power) or when all EVs leave together. Two caveats:

- The LP optimizes total energy only: when several schedules deliver the same total, which EV
  gets it is arbitrary, and one can be starved. Equal share is the one to use if fairness
  matters more than energy.
- The LP is optimal only for one snapshot. Under the receding horizon, with EVs arriving
  that no plan knew about, and with assumed needs for the EVs that don't report theirs, it
  loses that edge. Energy delivered, as % of deliverable, averaged over 10 seeds of 7 days
  ([full results](docs/benchmark.md)):

  | | `equal` | `edf` | `lp` |
  |---|---|---|---|
  | `workplace` (30 % of EVs report their needs) | 97.36 % | **98.12 %** | 96.78 % |
  | `hub` (50 % report) | 99.31 % | **99.47 %** | 99.08 % |
  | `workplace`, every EV reports | 97.41 % | 99.88 % | **99.99 %** |

  That is why EDF is the default: it is best with the partial information a site has
  today, and the LP only pulls ahead when nearly every EV reports its needs.

To compare them yourself, see [Running the simulator](#running-the-simulator), or rerun the
whole benchmark with `scripts/benchmark.sh` ([docs/benchmark.md](docs/benchmark.md#setup)).

### The LP model

**Data.** $T$ slots of $\Delta$ hours each, numbered $t = 0, \dots, T-1$ from now. For each
slot, the capacity $C_t$ (the grid limit minus what is reserved for idle EVSEs). For each
session $i$:

- $P_i$, its max power: the lower of the EVSE's and the EV's;
- $E_i$, the energy it still needs;
- $d_i$, the number of horizon slots before it leaves (at most $T$).

**Variables.** $p_{i,t}$, the power given to session $i$ in slot $t$, only for $t < d_i$: an
EV gets no variables after its departure, and a full EV ($E_i = 0$) gets none at all.

**Model.**

```math
\begin{aligned}
\max\quad & \sum_i \sum_{t < d_i} w_t\, p_{i,t}, \qquad w_t = 1 - \varepsilon \frac{t}{T} \\
\text{s.t.}\quad & \sum_i p_{i,t} \le C_t && \text{for every slot } t \quad \text{(site capacity)} \\
& \Delta \sum_{t < d_i} p_{i,t} \le E_i && \text{for every session } i \quad \text{(no overcharging)} \\
& 0 \le p_{i,t} \le P_i && \text{(EV and EVSE rating)}
\end{aligned}
```

**Objective.** Without the weights ($w_t = 1$) the objective is the total energy delivered,
divided by $\Delta$. The weights fall slightly with time, from $1$ in the first slot to just
above $1 - \varepsilon$ in the last ($\varepsilon = 10^{-4}$), so a watt now is worth a little
more than a watt later. Because they are so close to 1, they act as a tie-break: among the
schedules that deliver the most energy, the LP picks the one that delivers it earliest. That keeps
capacity free later for EVs that haven't arrived yet and that no plan knows about, and it gets
energy into EVs before they can leave early. The price is small and bounded: since every
$w_t \ge 1 - \varepsilon$, the LP's schedule delivers at least $(1 - \varepsilon)$ times the
maximum energy.

**What it doesn't model.**

- Demand is an upper bound, not a target: there is no penalty for leaving an EV short, and no
  fairness term. When several schedules deliver the same energy at the same time, they are
  equally good to the LP, and the solver returns one of the extremes, which can give one EV
  everything and another nothing.
- Only the sessions known now: future arrivals, early departures and wrong estimates are
  handled by planning again on the next tick, not in the model.
- The capacity is taken as constant over the horizon (the planner repeats the current grid
  limit), although the model accepts a different $C_t$ per slot.

**Why EDF and equal share can do worse.** The model is a flow problem: energy flows from each
session (at most $E_i/\Delta$) through the slots it is present in (at most $P_i$ per slot) into
the site (at most $C_t$ per slot). The heuristics fill that network one slot at a time and never
revisit a slot, so they can use a slot's capacity on an EV that could have charged later and
starve one that couldn't. The LP routes the flow over all slots together.

**Solving.** There are at most $N \cdot T$ variables and $N + T$ constraints for $N$ sessions
(20 EVs over 144 slots: at most 2,880 variables), solved with HiGHS through `good_lp` on one
thread. The all-zero schedule is always feasible, so a failure is a solver problem: the
scheduler then falls back to equal share. After solving, the power is clamped to the bounds
and scaled down wherever the solver's floating-point tolerance let a limit be exceeded by a
hair. The LP needs the `lp` feature, which is on by default and builds HiGHS from source, so it
needs cmake and a C++ compiler.

## License

Licensed under the [MIT license](LICENSE).
