//! Property tests over random instances.

use optimizer::{
    Edf, EqualShare, EvseRef, PlanRequest, Planner, Schedule, Scheduler, SessionRequest, SiteInput,
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
