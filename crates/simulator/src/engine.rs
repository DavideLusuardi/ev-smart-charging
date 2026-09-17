//! Discrete-time simulation of a site under a scheduling strategy.
//!
//! The engine steps through time, plugs EVs in and out, and re-plans exactly as the CSMS
//! does: it estimates each session's needs with [`SessionState::request`], runs the
//! [`Planner`], and sends profile updates chosen by [`dispatch`]. The stations accept every
//! update at once, so both dispatch phases are applied in the same step. Between plans, each
//! EV draws the lowest of its EVSE's effective limit, its own max power, and what it still
//! needs, and every step's draw is a meter reading the controller learns the EV's max power
//! from.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use optimizer::types::tolerance;
use optimizer::{
    EvseRef, KnownNeeds, PlanRequest, Planner, Scheduler, SessionState, StepProfile, Violation,
    dispatch, observe_draw,
};

use crate::arrivals::{EvArrival, START_SECS};
use crate::scenario::Scenario;

/// A broken invariant. Either one means a bug, so the run stops.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("invalid schedule at t={at_secs}: {violation}")]
    InvalidSchedule { at_secs: i64, violation: Violation },
    #[error("site draws {total_w} W at t={at_secs}, above the {limit_w} W limit")]
    OverLimit {
        at_secs: i64,
        total_w: f64,
        limit_w: f64,
    },
}

/// What happened to one EV.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionOutcome {
    pub ev: EvArrival,
    /// When it plugged in; `None` if the driver gave up waiting.
    pub plugged_secs: Option<i64>,
    /// When it left the site (unplugged, or gave up).
    pub left_secs: i64,
    pub delivered_wh: f64,
    /// The EVSE's max power, which caps what the EV can draw.
    pub evse_max_w: f64,
}

impl SessionOutcome {
    pub fn gave_up(&self) -> bool {
        self.plugged_secs.is_none()
    }

    /// Time spent waiting for a free EVSE.
    pub fn wait_secs(&self) -> i64 {
        self.plugged_secs.unwrap_or(self.left_secs) - self.ev.arrival_secs
    }

    /// Whether the EV got what the driver asked for (within 0.1 % or 1 Wh).
    pub fn satisfied(&self) -> bool {
        self.delivered_wh >= self.ev.energy_wh - (1e-3 * self.ev.energy_wh).max(1.0)
    }

    /// The most any strategy could have delivered once the EV was plugged in: its max
    /// power until departure, capped by the energy asked for.
    pub fn max_deliverable_wh(&self) -> f64 {
        self.plugged_secs.map_or(0.0, |plugged| {
            let hours = (self.ev.departure_secs - plugged) as f64 / 3600.0;
            (self.ev.max_power_w.min(self.evse_max_w) * hours).min(self.ev.energy_wh)
        })
    }
}

/// The site at one time step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub t_secs: i64,
    pub power_w: f64,
    pub limit_w: f64,
    pub sessions: usize,
    pub queued: usize,
}

/// The outcome of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunResult {
    /// One per EV, in arrival order.
    pub sessions: Vec<SessionOutcome>,
    pub timeseries: Vec<Sample>,
    /// Planner runs, each with its wall-clock time in seconds.
    pub solve_secs: Vec<f64>,
    /// SetChargingProfile messages the CSMS would have sent.
    pub profiles_sent: usize,
}

impl RunResult {
    pub fn peak_power_w(&self) -> f64 {
        self.timeseries
            .iter()
            .map(|s| s.power_w)
            .fold(0.0, f64::max)
    }
}

struct Ev {
    plugged_secs: Option<i64>,
    left_secs: Option<i64>,
    delivered_wh: f64,
    /// The EV's max power as the controller has learned it from meter readings.
    observed_max_w: Option<f64>,
    full: bool,
}

/// Simulates `evs` (sorted by arrival) until the last one has left.
pub fn run(
    scenario: &Scenario,
    scheduler: Box<dyn Scheduler>,
    evs: &[EvArrival],
) -> Result<RunResult, EngineError> {
    let site = &scenario.site;
    let control = &scenario.control;
    let step = i64::from(control.step_secs);
    let step_hours = step as f64 / 3600.0;
    let defaults = scenario.defaults.needs();
    let config = control.config();
    let planner = Planner::new(scheduler, control.max_horizon_slots);

    let evses: Vec<EvseRef> = (1..=site.stations)
        .flat_map(|s| {
            (1..=site.evses_per_station as i32).map(move |e| EvseRef {
                station_id: format!("st-{s:02}"),
                evse_id: e,
            })
        })
        .collect();
    let mut occupant: Vec<Option<usize>> = vec![None; evses.len()];
    let mut state: Vec<Ev> = evs
        .iter()
        .map(|_| Ev {
            plugged_secs: None,
            left_secs: None,
            delivered_wh: 0.0,
            observed_max_w: None,
            full: false,
        })
        .collect();
    let mut queue: VecDeque<usize> = VecDeque::new();
    let mut effective: BTreeMap<EvseRef, StepProfile> = BTreeMap::new();
    let mut next_arrival = 0;
    let mut replan = false;
    let mut result = RunResult {
        sessions: Vec::new(),
        timeseries: Vec::new(),
        solve_secs: Vec::new(),
        profiles_sent: 0,
    };

    let mut t = START_SECS;
    loop {
        // Departures: the driver's time is up, or a hub EV is full.
        for (evse, slot) in occupant.iter_mut().enumerate() {
            let Some(i) = *slot else { continue };
            if t >= evs[i].departure_secs || (scenario.ev.leave_when_full && state[i].full) {
                state[i].left_secs = Some(t);
                *slot = None;
                effective.remove(&evses[evse]);
                replan = true;
            }
        }

        // Arrivals join the queue; the queue fills free EVSEs in order.
        while next_arrival < evs.len() && evs[next_arrival].arrival_secs <= t {
            queue.push_back(next_arrival);
            next_arrival += 1;
        }
        while let Some(evse) = occupant.iter().position(Option::is_none) {
            let Some(i) = queue.pop_front() else { break };
            if t >= evs[i].departure_secs {
                state[i].left_secs = Some(t);
                continue;
            }
            occupant[evse] = Some(i);
            state[i].plugged_secs = Some(t);
            replan = true;
        }
        queue.retain(|&i| {
            let ev = &evs[i];
            let deadline = (ev.arrival_secs + scenario.ev.max_wait_secs).min(ev.departure_secs);
            let waiting = t < deadline;
            if !waiting {
                state[i].left_secs = Some(t);
            }
            waiting
        });

        let plugged: Vec<(usize, usize)> = occupant
            .iter()
            .enumerate()
            .filter_map(|(evse, slot)| slot.map(|i| (evse, i)))
            .collect();
        let tick = (t - START_SECS) % i64::from(control.tick_secs) == 0;
        if (replan || tick) && !plugged.is_empty() {
            let sessions = plugged
                .iter()
                .map(|&(evse, i)| {
                    let ev = &evs[i];
                    SessionState {
                        id: ev.id.as_str().into(),
                        evse: evses[evse].clone(),
                        started_secs: state[i].plugged_secs.expect("plugged in"),
                        evse_max_w: site.evse_max_w,
                        delivered_wh: state[i].delivered_wh,
                        needs: ev.iso15118.then_some(KnownNeeds {
                            departure_secs: ev.departure_secs,
                            energy_wh: ev.energy_wh,
                            max_power_w: Some(ev.max_power_w),
                        }),
                        observed_max_w: state[i].observed_max_w,
                        ev_suspended: state[i].full,
                    }
                    .request(t, control.slot_secs, &defaults)
                })
                .collect();
            let request = PlanRequest {
                now_secs: t,
                slot_secs: control.slot_secs,
                capacity_w: site.grid_limit_w,
                idle_evses: evses.len() - plugged.len(),
                fallback_limit_w: site.fallback_limit_w,
                sessions,
            };

            let started = Instant::now();
            let plan = planner.plan(&request);
            result.solve_secs.push(started.elapsed().as_secs_f64());
            plan.schedule
                .validate(&plan.site, &request.sessions)
                .map_err(|violation| EngineError::InvalidSchedule {
                    at_secs: t,
                    violation,
                })?;

            let updates = dispatch(
                t,
                request.planning_capacity_w(),
                &plan.limits,
                &effective,
                site.fallback_limit_w,
                &config,
            );
            for update in updates.phase1.into_iter().chain(updates.phase2) {
                effective.insert(update.evse, update.profile);
                result.profiles_sent += 1;
            }
        }
        replan = false;

        // Charging over [t, t + step).
        let mut total_w = 0.0;
        for &(evse, i) in &plugged {
            let ev = &evs[i];
            let s = &mut state[i];
            let limit = effective
                .get(&evses[evse])
                .map_or(site.fallback_limit_w, |p| p.value_at(t))
                .min(site.evse_max_w);
            let wanted = (ev.energy_wh - s.delivered_wh).max(0.0) / step_hours;
            let power = limit.min(ev.max_power_w).min(wanted).max(0.0);
            s.delivered_wh += power * step_hours;
            total_w += power;
            if s.full {
                continue;
            }
            if s.delivered_wh >= ev.energy_wh - 1e-6 {
                // The EV stops; its last, partial draw says nothing about its max power.
                s.full = true;
                replan = true;
            } else {
                s.observed_max_w = observe_draw(s.observed_max_w, limit, power);
            }
        }
        if total_w > site.grid_limit_w + tolerance(site.grid_limit_w) {
            return Err(EngineError::OverLimit {
                at_secs: t,
                total_w,
                limit_w: site.grid_limit_w,
            });
        }
        result.timeseries.push(Sample {
            t_secs: t,
            power_w: total_w,
            limit_w: site.grid_limit_w,
            sessions: plugged.len(),
            queued: queue.len(),
        });

        t += step;
        if next_arrival == evs.len() && queue.is_empty() && occupant.iter().all(Option::is_none) {
            break;
        }
    }

    result.sessions = evs
        .iter()
        .zip(state)
        .map(|(ev, s)| SessionOutcome {
            ev: ev.clone(),
            plugged_secs: s.plugged_secs,
            left_secs: s.left_secs.expect("every EV leaves before the run ends"),
            delivered_wh: s.delivered_wh,
            evse_max_w: site.evse_max_w,
        })
        .collect();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrivals::generate;
    use optimizer::{SCHEDULERS, scheduler_by_name};

    const HOUR: i64 = 3600;

    /// A small AC site: `evses` EVSEs of 22 kW, `limit_w` grid limit.
    fn site(evses: u32, limit_w: f64, leave_when_full: bool, max_wait_secs: i64) -> Scenario {
        Scenario::from_toml(&format!(
            r#"
            name = "test"
            [site]
            stations = 1
            evses_per_station = {evses}
            evse_max_w = 22_000.0
            grid_limit_w = {limit_w:.1}
            fallback_limit_w = 1_400.0
            [control]
            slot_secs = 300
            max_horizon_slots = 48
            tick_secs = 300
            [arrivals]
            rate_per_hour = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 4.0, 2.0, 1.0, 1.0,
                             1.0, 1.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
            weekend_factor = 1.0
            [ev]
            dwell_secs = {{ median = 10_800.0, sigma = 0.5, min = 1_800.0, max = 21_600.0 }}
            energy_wh = {{ median = 15_000.0, sigma = 0.5, min = 2_000.0, max = 50_000.0 }}
            max_power_w = [{{ value = 7_400.0, weight = 1.0 }}, {{ value = 11_000.0, weight = 2.0 }}]
            iso15118_share = 0.5
            leave_when_full = {leave_when_full}
            max_wait_secs = {max_wait_secs}
            [defaults]
            dwell_secs = 3_600
            energy_wh = 5_000.0
            "#
        ))
        .unwrap()
    }

    fn ev(
        id: &str,
        arrive_h: f64,
        stay_h: f64,
        energy_wh: f64,
        max_w: f64,
        iso: bool,
    ) -> EvArrival {
        let arrival_secs = START_SECS + (arrive_h * HOUR as f64) as i64;
        EvArrival {
            id: id.into(),
            arrival_secs,
            departure_secs: arrival_secs + (stay_h * HOUR as f64) as i64,
            energy_wh,
            max_power_w: max_w,
            iso15118: iso,
        }
    }

    fn run_with(scenario: &Scenario, name: &str, evs: &[EvArrival]) -> RunResult {
        run(scenario, scheduler_by_name(name).unwrap(), evs).unwrap()
    }

    #[test]
    fn a_lone_ev_charges_at_its_own_max() {
        let s = site(2, 150_000.0, false, 0);
        let r = run_with(&s, "equal", &[ev("a", 1.0, 4.0, 22_000.0, 11_000.0, true)]);
        let a = &r.sessions[0];
        assert_eq!(a.plugged_secs, Some(a.ev.arrival_secs));
        assert!(a.satisfied());
        // 22 kWh at 11 kW takes two hours.
        let charging = r.timeseries.iter().filter(|x| x.power_w > 0.0).count() as i64;
        assert_eq!(charging * 10, 2 * HOUR);
        assert!(r.timeseries.iter().all(|x| x.power_w <= 11_000.0));
        assert_eq!(a.left_secs, a.ev.departure_secs);
    }

    #[test]
    fn the_limit_holds_for_every_strategy() {
        let s = site(6, 40_000.0, false, 900);
        let evs = generate(&s, 11, 1, 0.5);
        assert!(evs.len() > 10);
        for name in SCHEDULERS {
            let r = run_with(&s, name, &evs);
            assert!(r.peak_power_w() <= 40_000.0 + 1e-3, "{name}");
            assert!(r.peak_power_w() > 30_000.0, "{name}: the limit should bind");
            for o in &r.sessions {
                assert!(o.delivered_wh <= o.ev.energy_wh + 1e-6, "{name}");
                assert!(o.delivered_wh <= o.max_deliverable_wh() + 1e-6, "{name}");
            }
        }
    }

    /// With capacity to spare, the information model must cost nothing: every EV, ISO 15118
    /// or not, gets what it could physically take.
    #[test]
    fn a_roomy_site_delivers_everything_deliverable() {
        let s = site(6, 1_000_000.0, false, 900);
        let evs = generate(&s, 4, 2, 0.5);
        for name in SCHEDULERS {
            let r = run_with(&s, name, &evs);
            for o in r.sessions.iter().filter(|o| !o.gave_up()) {
                assert!(
                    o.delivered_wh >= o.max_deliverable_wh() - 1.0,
                    "{name}: {o:?}"
                );
            }
        }
    }

    #[test]
    fn the_queue_fills_freed_evses_and_drivers_give_up() {
        let s = site(1, 150_000.0, false, 1_800);
        let evs = [
            ev("a", 1.0, 1.0, 5_000.0, 11_000.0, true),
            // Arrives while a is plugged in and gets the EVSE when a leaves.
            ev("b", 1.75, 2.0, 5_000.0, 11_000.0, true),
            // Arrives while b is plugged in and gives up after 30 minutes.
            ev("c", 2.5, 2.0, 5_000.0, 11_000.0, true),
        ];
        let r = run_with(&s, "edf", &evs);
        let [a, b, c] = [&r.sessions[0], &r.sessions[1], &r.sessions[2]];
        assert_eq!(b.plugged_secs, Some(a.left_secs));
        assert_eq!(b.wait_secs(), HOUR / 4);
        assert!(b.satisfied());
        assert!(c.gave_up());
        assert_eq!(c.wait_secs(), 1_800);
        assert_eq!(c.delivered_wh, 0.0);
        assert_eq!(c.max_deliverable_wh(), 0.0);
    }

    #[test]
    fn hub_evs_leave_when_full() {
        let s = site(1, 150_000.0, true, 0);
        let r = run_with(&s, "equal", &[ev("a", 1.0, 3.0, 11_000.0, 11_000.0, true)]);
        let a = &r.sessions[0];
        assert!(a.satisfied());
        // Full after one hour, and gone right then instead of at its departure.
        assert_eq!(a.left_secs - a.ev.arrival_secs, HOUR);
    }

    #[test]
    fn estimates_roll_forward_for_evs_without_iso15118() {
        // The defaults assume 1 h and 5 kWh; the EV stays 3 h and wants 20 kWh.
        let s = site(1, 150_000.0, false, 0);
        let r = run_with(&s, "edf", &[ev("a", 1.0, 3.0, 20_000.0, 11_000.0, false)]);
        assert!(r.sessions[0].satisfied());
    }

    #[test]
    fn the_max_power_of_evs_without_iso15118_is_learned() {
        // Two EVs on a 20 kW site: the ISO one reports 22 kW, the other is assumed to take
        // 22 kW but only draws 7.4 kW. Equal share first plans 10 kW each, so 2.6 kW goes
        // unused, until the next tick plans b at what it was seen to draw (plus a margin).
        let s = site(2, 20_000.0, false, 0);
        let evs = [
            ev("a", 1.0, 4.0, 60_000.0, 22_000.0, true),
            ev("b", 1.0, 4.0, 60_000.0, 7_400.0, false),
        ];
        let r = run_with(&s, "equal", &evs);
        let both: Vec<&Sample> = r.timeseries.iter().filter(|x| x.sessions == 2).collect();
        assert!((both[0].power_w - 17_400.0).abs() < 1e-6, "{:?}", both[0]);
        // 300 s later: b planned at 7.9 kW, a gets the other 12.1 kW, 19.5 kW drawn.
        assert!((both[30].power_w - 19_500.0).abs() < 1e-6, "{:?}", both[30]);
    }

    #[test]
    fn runs_are_deterministic() {
        let s = site(4, 30_000.0, false, 900);
        let evs = generate(&s, 2, 1, 0.5);
        let a = run_with(&s, "edf", &evs);
        let b = run_with(&s, "edf", &evs);
        assert_eq!(a.sessions, b.sessions);
        assert_eq!(a.timeseries, b.timeseries);
        assert_eq!(a.profiles_sent, b.profiles_sent);
    }
}
