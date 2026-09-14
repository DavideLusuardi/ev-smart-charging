//! Property tests over random instances.

use optimizer::{Edf, EqualShare, EvseRef, Schedule, Scheduler, SessionRequest, SiteInput};
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
