//! Property tests over random instances.

use std::collections::BTreeMap;

use optimizer::{
    ControlConfig, Edf, EqualShare, EvseLimit, EvseRef, PlanRequest, Planner, Schedule, Scheduler,
    SessionRequest, SiteInput, StepProfile, dispatch,
};
use proptest::prelude::*;

const NOW: i64 = 1_000;

fn arb_site() -> impl Strategy<Value = SiteInput> {
    let slot_secs = prop::sample::select(vec![60u32, 300, 900, 3600]);
    let capacity = prop_oneof![
        1 => Just(0.0),
        6 => 0.0..200_000.0f64,
    ];
    (slot_secs, prop::collection::vec(capacity, 1..=48)).prop_map(|(slot_secs, capacity_w)| {
        SiteInput {
            slot_secs,
            now: NOW,
            capacity_w,
        }
    })
}

fn arb_sessions(horizon: usize) -> impl Strategy<Value = Vec<SessionRequest>> {
    let session = (
        -2..=horizon as i64 + 3,
        prop_oneof![1 => Just(0.0), 6 => 1_000.0..50_000.0f64],
        prop_oneof![1 => Just(0.0), 6 => 100.0..100_000.0f64],
    );
    prop::collection::vec(session, 0..=8).prop_map(|specs| {
        specs
            .into_iter()
            .enumerate()
            .map(
                |(i, (departs_after, max_power_w, remaining_wh))| SessionRequest {
                    id: format!("s{i}").as_str().into(),
                    evse: EvseRef {
                        station_id: "st".into(),
                        evse_id: i as i32 + 1,
                    },
                    departure: NOW + departs_after,
                    remaining_wh,
                    max_power_w,
                },
            )
            .collect()
    })
}

fn arb_instance() -> impl Strategy<Value = (SiteInput, Vec<SessionRequest>)> {
    arb_site().prop_flat_map(|site| {
        let horizon = site.horizon();
        (Just(site), arb_sessions(horizon))
    })
}

fn strategies() -> Vec<Box<dyn Scheduler>> {
    vec![
        Box::new(EqualShare),
        Box::new(Edf),
        #[cfg(feature = "lp")]
        Box::new(optimizer::Lp::default()),
    ]
}

/// In every slot, a work-conserving schedule either uses all the capacity or gives every
/// present session as much as it can take.
fn is_work_conserving(
    schedule: &Schedule,
    site: &SiteInput,
    sessions: &[SessionRequest],
) -> Result<(), String> {
    let hours = site.slot_hours();
    let mut needed: Vec<f64> = sessions.iter().map(|s| s.remaining_wh.max(0.0)).collect();
    for (k, &capacity) in site.capacity_w.iter().enumerate() {
        let mut demand = 0.0;
        for (i, s) in sessions.iter().enumerate() {
            if k < s.active_slots(site) {
                demand += s.max_power_w.min(needed[i] / hours).max(0.0);
            }
            needed[i] -= schedule.power_w[&s.id][k] * hours;
        }
        let used = schedule.slot_total_w(k);
        let target = demand.min(capacity.max(0.0));
        if used < target - 1e-6 * target - 1e-3 {
            return Err(format!(
                "slot {k}: used {used} W of an achievable {target} W"
            ));
        }
    }
    Ok(())
}

proptest! {
    #[test]
    fn every_strategy_keeps_the_invariants((site, sessions) in arb_instance()) {
        for strategy in strategies() {
            let schedule = strategy.schedule(&site, &sessions);
            prop_assert_eq!(schedule.validate(&site, &sessions), Ok(()), "{}", strategy.name());
        }
    }

    #[test]
    fn heuristics_are_work_conserving((site, sessions) in arb_instance()) {
        for strategy in [&EqualShare as &dyn Scheduler, &Edf] {
            let schedule = strategy.schedule(&site, &sessions);
            if let Err(e) = is_work_conserving(&schedule, &site, &sessions) {
                prop_assert!(false, "{}: {}", strategy.name(), e);
            }
        }
    }
}

#[cfg(feature = "lp")]
proptest! {
    /// On a static instance (no future arrivals) the LP is optimal up to its earliness
    /// weights, so it delivers at least as much as either heuristic.
    #[test]
    fn lp_delivers_at_least_as_much_as_the_heuristics((site, sessions) in arb_instance()) {
        let lp = optimizer::Lp::default().schedule(&site, &sessions).total_energy_wh(&site);
        for strategy in [&EqualShare as &dyn Scheduler, &Edf] {
            let heuristic = strategy.schedule(&site, &sessions).total_energy_wh(&site);
            prop_assert!(
                lp >= heuristic * (1.0 - 2e-4) - 1e-3,
                "lp {} Wh < {} {} Wh", lp, strategy.name(), heuristic
            );
        }
    }
}

fn arb_plan_request() -> impl Strategy<Value = PlanRequest> {
    let slot_secs = prop::sample::select(vec![60u32, 300, 900]);
    (
        slot_secs,
        0..3600i64,
        0.0..300_000.0f64,
        0..6usize,
        0.0..3_000.0f64,
    )
        .prop_flat_map(
            |(slot_secs, into_slot, capacity_w, idle_evses, fallback_limit_w)| {
                let now_secs = NOW * i64::from(slot_secs) + into_slot % i64::from(slot_secs);
                arb_sessions(60).prop_map(move |sessions| PlanRequest {
                    now_secs,
                    slot_secs,
                    capacity_w,
                    idle_evses,
                    fallback_limit_w,
                    sessions,
                    held: Vec::new(),
                })
            },
        )
}

proptest! {
    /// At every time a profile can change, the planned limits sum to at most the capacity
    /// left after the idle EVSEs' reserve, and no limit is negative.
    #[test]
    fn planned_profiles_respect_capacity(
        request in arb_plan_request(),
        strategy in 0..strategies().len(),
        max_horizon in 1..80usize,
    ) {
        let planner = Planner::new(strategies().swap_remove(strategy), max_horizon);
        let plan = planner.plan(&request);
        prop_assert_eq!(plan.limits.len(), request.sessions.len());
        prop_assert_eq!(plan.schedule.validate(&plan.site, &request.sessions), Ok(()));

        let capacity = request.planning_capacity_w();
        let mut times: Vec<i64> = plan.limits.iter().flat_map(|l| l.profile.breakpoints()).collect();
        times.push(i64::MAX / 2);
        for t in times {
            let values: Vec<f64> = plan.limits.iter().map(|l| l.profile.value_at(t)).collect();
            prop_assert!(values.iter().all(|&v| v >= 0.0), "negative limit at {}", t);
            let total: f64 = values.iter().sum();
            prop_assert!(total <= capacity + 1e-6 * capacity + 1e-3, "{} W > {} W at {}", total, capacity, t);
        }
    }
}

const T0: i64 = 1_000_000;
const FALLBACK_W: f64 = 1_400.0;

fn evse(i: usize) -> EvseRef {
    EvseRef {
        station_id: "st".into(),
        evse_id: i as i32 + 1,
    }
}

fn arb_profile(start: std::ops::RangeInclusive<i64>) -> impl Strategy<Value = StepProfile> {
    let steps = prop::collection::vec((1..900i64, 0.0..30_000.0f64), 1..4);
    (start, steps).prop_map(|(start, steps)| {
        let mut offset = 0;
        let steps = steps.into_iter().map(|(gap, limit)| {
            let step = (offset, limit);
            offset += gap;
            step
        });
        StepProfile::from_steps(start, steps.collect::<Vec<_>>())
    })
}

/// `now` and every later breakpoint of the profiles, plus a time far in the future.
fn times<'a>(profiles: impl IntoIterator<Item = &'a StepProfile>) -> Vec<i64> {
    let mut t: Vec<i64> = profiles
        .into_iter()
        .flat_map(|p| p.breakpoints())
        .filter(|&t| t > T0)
        .collect();
    t.extend([T0, i64::MAX / 2]);
    t.sort_unstable();
    t.dedup();
    t
}

fn total_at(state: &[StepProfile], t: i64) -> f64 {
    state.iter().map(|p| p.value_at(t)).sum()
}

/// Scales `profiles` down so that, together with `fixed_w`, they never exceed `capacity_w`.
fn fit(profiles: &mut [StepProfile], fixed_w: f64, capacity_w: f64) {
    let peak = times(profiles.iter())
        .into_iter()
        .map(|t| total_at(profiles, t))
        .fold(0.0, f64::max);
    if peak > 0.0 && peak + fixed_w > capacity_w {
        let scale = ((capacity_w - fixed_w) / peak).max(0.0);
        for p in profiles.iter_mut() {
            p.periods
                .iter_mut()
                .for_each(|period| period.limit_w *= scale);
        }
    }
}

#[derive(Debug, Clone)]
struct ControlCase {
    capacity_w: f64,
    /// `None` means the EVSE is still on its fallback limit.
    effective: Vec<Option<StepProfile>>,
    targets: Vec<StepProfile>,
}

fn arb_control_case() -> impl Strategy<Value = ControlCase> {
    (1..=5usize, 10_000.0..100_000.0f64).prop_flat_map(|(n, capacity_w)| {
        let effective =
            prop::collection::vec(prop::option::weighted(0.8, arb_profile(T0 - 600..=T0)), n);
        let targets = prop::collection::vec(arb_profile(T0..=T0), n);
        (effective, targets).prop_map(move |(mut effective, mut targets)| {
            let on_fallback = effective.iter().filter(|e| e.is_none()).count() as f64 * FALLBACK_W;
            let mut known: Vec<StepProfile> = effective.iter().flatten().cloned().collect();
            fit(&mut known, on_fallback, capacity_w);
            let mut known = known.into_iter();
            for e in effective.iter_mut().filter(|e| e.is_some()) {
                *e = known.next();
            }
            fit(&mut targets, 0.0, capacity_w);
            ControlCase {
                capacity_w,
                effective,
                targets,
            }
        })
    })
}

proptest! {
    /// Whatever order the stations accept the updates in, the site limit is never exceeded:
    /// during phase 1 the total stays below the effective total, during phase 2 below the
    /// capacity. Every change above the threshold ends up applied.
    #[test]
    fn dispatch_is_safe_in_any_acceptance_order(case in arb_control_case()) {
        let n = case.targets.len();
        let tol = 1e-6 * case.capacity_w + 1e-3;
        let effective: BTreeMap<EvseRef, StepProfile> = case
            .effective
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.clone().map(|p| (evse(i), p)))
            .collect();
        let targets: Vec<EvseLimit> = case
            .targets
            .iter()
            .enumerate()
            .map(|(i, p)| EvseLimit { evse: evse(i), session: format!("s{i}").as_str().into(), profile: p.clone() })
            .collect();
        let config = ControlConfig::default();
        let d = dispatch(T0, case.capacity_w, &targets, &effective, FALLBACK_W, &config);

        let start: Vec<StepProfile> = (0..n)
            .map(|i| effective.get(&evse(i)).cloned().unwrap_or_else(|| StepProfile::constant(T0, FALLBACK_W)))
            .collect();
        let index = |e: &EvseRef| (e.evse_id - 1) as usize;
        let all_profiles: Vec<&StepProfile> = start
            .iter()
            .chain(&case.targets)
            .chain(d.phase1.iter().chain(&d.phase2).map(|u| &u.profile))
            .collect();
        let check_times = times(all_profiles.iter().copied());

        let mut after_phase1 = start.clone();
        for mask in 0u32..(1 << d.phase1.len()) {
            let mut state = start.clone();
            for (bit, u) in d.phase1.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    state[index(&u.evse)] = u.profile.clone();
                }
            }
            for &t in &check_times {
                prop_assert!(total_at(&state, t) <= total_at(&start, t) + tol, "phase 1 raised the total at {}", t);
            }
            after_phase1 = state;
        }

        let mut last = after_phase1.clone();
        for mask in 0u32..(1 << d.phase2.len()) {
            let mut state = after_phase1.clone();
            for (bit, u) in d.phase2.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    state[index(&u.evse)] = u.profile.clone();
                }
            }
            for &t in &check_times {
                prop_assert!(total_at(&state, t) <= case.capacity_w + tol, "phase 2 exceeds capacity at {}", t);
            }
            last = state;
        }

        for i in 0..n {
            let target = &case.targets[i];
            let big_change = check_times.iter().any(|&t| {
                let (new, eff) = (target.value_at(t), start[i].value_at(t));
                (new - eff).abs() > config.min_change_w.max(config.rel_change * eff)
            });
            if big_change {
                for &t in &check_times {
                    prop_assert!((last[i].value_at(t) - target.value_at(t)).abs() < 1e-6, "EVSE {} not at its target", i);
                }
            }
        }
    }
}
