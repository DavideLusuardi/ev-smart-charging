//! Turns a plan into the profile updates to send, ordered so the site limit holds at every
//! moment, whichever order the stations accept them in.
//!
//! The *effective* profile of an EVSE is the one the station is following: the last
//! accepted TxProfile, or the TxDefaultProfile fallback limit before the first one.
//!
//! Updates go out in two phases:
//!
//! 1. Each changed EVSE gets `min(new, effective)`. These only lower limits, so while they
//!    are being accepted, in any order, the total stays below the total of the effective
//!    profiles.
//! 2. Once every phase-1 update is acknowledged, each EVSE whose new profile is higher
//!    somewhere gets `new`. Every EVSE is then at `min(new, effective)` or `new`, both at most
//!    `new`, so the total stays below the plan's total, which fits the capacity.
//!
//! An EVSE whose plan is lower everywhere needs only phase 1, one that is higher everywhere
//! only phase 2; only a mixed change costs two messages.

use std::collections::{BTreeMap, BTreeSet};

use crate::planner::EvseLimit;
use crate::profile::StepProfile;
use crate::types::{EvseRef, SessionId, tolerance};

/// When a planned change is big enough to send.
///
/// Starting or stopping (a change from or to 0 W) is always sent, however small: an EV held
/// at 0 W by a small change that is never sent would never charge again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlConfig {
    /// Changes of at most this many watts are not sent…
    pub min_change_w: f64,
    /// …nor changes of at most this fraction of the effective limit, whichever is larger.
    pub rel_change: f64,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            min_change_w: 1_000.0,
            rel_change: 0.05,
        }
    }
}

/// One SetChargingProfile to send.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileUpdate {
    pub evse: EvseRef,
    pub session: SessionId,
    pub profile: StepProfile,
}

/// The updates for one control tick, in the order they must be sent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dispatch {
    /// Pure decreases; send these first and wait until all are acknowledged.
    pub phase1: Vec<ProfileUpdate>,
    /// Send these after phase 1 completes.
    pub phase2: Vec<ProfileUpdate>,
}

impl Dispatch {
    /// Whether nothing needs to be sent.
    pub fn is_empty(&self) -> bool {
        self.phase1.is_empty() && self.phase2.is_empty()
    }
}

/// Decides which planned profiles to send, and in which phase.
///
/// `effective` holds the profiles in force; an EVSE in `targets` without an entry is on its
/// fallback limit. EVSEs in `effective` without a target keep their profile and still count
/// towards the total.
///
/// A target is sent when it differs from the effective profile by more than the threshold at
/// some time from `now_secs` on, or starts or stops charging at some time (one is 0 W and
/// the other isn't). If holding back the small changes would push the total over
/// `capacity_w` at any time, every target that differs at all is sent.
pub fn dispatch(
    now_secs: i64,
    capacity_w: f64,
    targets: &[EvseLimit],
    effective: &BTreeMap<EvseRef, StepProfile>,
    fallback_limit_w: f64,
    config: &ControlConfig,
) -> Dispatch {
    let fallback = StepProfile::constant(now_secs, fallback_limit_w);
    let current: Vec<&StepProfile> = targets
        .iter()
        .map(|t| effective.get(&t.evse).unwrap_or(&fallback))
        .collect();

    let above_threshold = |new: &StepProfile, eff: &StepProfile| {
        sample_times(now_secs, [new, eff]).any(|t| {
            let (n, e) = (new.value_at(t), eff.value_at(t));
            (n - e).abs() > config.min_change_w.max(config.rel_change * e)
                || (n > ZERO_W) != (e > ZERO_W)
        })
    };
    let mut changed: Vec<bool> = targets
        .iter()
        .zip(&current)
        .map(|(t, eff)| above_threshold(&t.profile, eff))
        .collect();

    let untargeted: Vec<&StepProfile> = effective
        .iter()
        .filter(|(evse, _)| !targets.iter().any(|t| &t.evse == *evse))
        .map(|(_, p)| p)
        .collect();
    let fits = |changed: &[bool]| {
        let state: Vec<&StepProfile> = targets
            .iter()
            .zip(&current)
            .zip(changed)
            .map(|((t, eff), &c)| if c { &t.profile } else { *eff })
            .chain(untargeted.iter().copied())
            .collect();
        within_capacity(now_secs, capacity_w, state)
    };
    if !fits(&changed) {
        changed = targets
            .iter()
            .zip(&current)
            .map(|(t, eff)| differs(&t.profile, eff, now_secs))
            .collect();
    }

    let mut out = Dispatch::default();
    for ((target, eff), _) in targets
        .iter()
        .zip(&current)
        .zip(&changed)
        .filter(|(_, c)| **c)
    {
        let lowered = target.profile.pointwise_min(eff, now_secs);
        let update = |profile: &StepProfile| ProfileUpdate {
            evse: target.evse.clone(),
            session: target.session.clone(),
            profile: profile.clone(),
        };
        if differs(&lowered, eff, now_secs) {
            out.phase1.push(update(&lowered));
        }
        if differs(&target.profile, &lowered, now_secs) {
            out.phase2.push(update(&target.profile));
        }
    }
    out
}

/// Whether `profiles` together stay within `capacity_w` at every time from `now_secs` on.
pub fn within_capacity<'a>(
    now_secs: i64,
    capacity_w: f64,
    profiles: impl IntoIterator<Item = &'a StepProfile>,
) -> bool {
    let profiles: Vec<&StepProfile> = profiles.into_iter().collect();
    sample_times(now_secs, profiles.iter().copied()).all(|t| {
        let total: f64 = profiles.iter().map(|p| p.value_at(t)).sum();
        total <= capacity_w + tolerance(capacity_w)
    })
}

/// Limits up to this are "not charging".
const ZERO_W: f64 = 1e-6;

/// `now` and every later time at which one of the profiles can change. Step functions are
/// constant between these, so checking them covers all future times.
fn sample_times<'a>(
    now_secs: i64,
    profiles: impl IntoIterator<Item = &'a StepProfile>,
) -> impl Iterator<Item = i64> {
    let mut times: BTreeSet<i64> = profiles
        .into_iter()
        .flat_map(StepProfile::breakpoints)
        .filter(|&t| t > now_secs)
        .collect();
    times.insert(now_secs);
    times.into_iter()
}

fn differs(a: &StepProfile, b: &StepProfile, now_secs: i64) -> bool {
    sample_times(now_secs, [a, b]).any(|t| (a.value_at(t) - b.value_at(t)).abs() > ZERO_W)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000;

    fn evse(id: i32) -> EvseRef {
        EvseRef {
            station_id: "st".into(),
            evse_id: id,
        }
    }

    fn target(id: i32, steps: &[(i64, f64)]) -> EvseLimit {
        EvseLimit {
            evse: evse(id),
            session: format!("s{id}").as_str().into(),
            profile: StepProfile::from_steps(NOW, steps.iter().copied()),
        }
    }

    fn effective(entries: &[(i32, f64)]) -> BTreeMap<EvseRef, StepProfile> {
        entries
            .iter()
            .map(|&(id, w)| (evse(id), StepProfile::constant(NOW - 60, w)))
            .collect()
    }

    fn run(
        capacity_w: f64,
        targets: &[EvseLimit],
        eff: &BTreeMap<EvseRef, StepProfile>,
    ) -> Dispatch {
        dispatch(
            NOW,
            capacity_w,
            targets,
            eff,
            1_400.0,
            &ControlConfig::default(),
        )
    }

    fn ids(updates: &[ProfileUpdate]) -> Vec<i32> {
        updates.iter().map(|u| u.evse.evse_id).collect()
    }

    #[test]
    fn small_changes_are_not_sent() {
        let d = run(
            100_000.0,
            &[target(1, &[(0, 10_400.0)])],
            &effective(&[(1, 10_000.0)]),
        );
        assert!(d.is_empty());
    }

    #[test]
    fn increases_go_in_phase_two_and_decreases_in_phase_one() {
        let eff = effective(&[(1, 10_000.0), (2, 10_000.0)]);
        let d = run(
            100_000.0,
            &[target(1, &[(0, 20_000.0)]), target(2, &[(0, 4_000.0)])],
            &eff,
        );
        assert_eq!(ids(&d.phase1), vec![2]);
        assert_eq!(d.phase1[0].profile.first_limit_w(), 4_000.0);
        assert_eq!(ids(&d.phase2), vec![1]);
        assert_eq!(d.phase2[0].profile.first_limit_w(), 20_000.0);
    }

    #[test]
    fn mixed_changes_are_split_into_both_phases() {
        // The EVSE moves its 10 kW from later to now: lower later, higher now.
        let eff = BTreeMap::from([(
            evse(1),
            StepProfile::from_steps(NOW, [(0, 5_000.0), (600, 10_000.0)]),
        )]);
        let new = target(1, &[(0, 10_000.0), (600, 5_000.0)]);
        let d = run(100_000.0, &[new], &eff);
        assert_eq!(ids(&d.phase1), vec![1]);
        assert_eq!(d.phase1[0].profile.value_at(NOW), 5_000.0);
        assert_eq!(d.phase1[0].profile.value_at(NOW + 600), 5_000.0);
        assert_eq!(ids(&d.phase2), vec![1]);
        assert_eq!(d.phase2[0].profile.value_at(NOW), 10_000.0);
    }

    /// EVSE 1 drops by 2 kW (under 5 % of 50 kW, so normally held back) while EVSE 2 rises
    /// by 2 kW (over 1 kW, so sent). Sending only the rise would total 62 kW on a 60 kW site,
    /// so the drop is sent too, and first.
    #[test]
    fn held_back_changes_are_sent_when_the_total_would_overshoot() {
        let eff = effective(&[(1, 50_000.0), (2, 10_000.0)]);
        let targets = [target(1, &[(0, 48_000.0)]), target(2, &[(0, 12_000.0)])];

        let roomy = run(100_000.0, &targets, &eff);
        assert!(roomy.phase1.is_empty());
        assert_eq!(ids(&roomy.phase2), vec![2]);

        let tight = run(60_000.0, &targets, &eff);
        assert_eq!(ids(&tight.phase1), vec![1]);
        assert_eq!(ids(&tight.phase2), vec![2]);
    }

    /// A remainder of a few Wh plans a few hundred W. Held back by the threshold, the EV
    /// would sit at 0 W for good.
    #[test]
    fn starting_or_stopping_is_always_sent() {
        let d = run(
            100_000.0,
            &[target(1, &[(0, 172.0)])],
            &effective(&[(1, 0.0)]),
        );
        assert_eq!(ids(&d.phase2), vec![1]);

        let d = run(
            100_000.0,
            &[target(1, &[(0, 0.0)])],
            &effective(&[(1, 500.0)]),
        );
        assert_eq!(ids(&d.phase1), vec![1]);

        // Stopping later counts too.
        let d = run(
            100_000.0,
            &[target(1, &[(0, 500.0), (300, 0.0)])],
            &effective(&[(1, 500.0)]),
        );
        assert_eq!(ids(&d.phase1), vec![1]);
    }

    #[test]
    fn sessions_without_a_profile_start_from_the_fallback() {
        let none = BTreeMap::new();
        let d = run(100_000.0, &[target(1, &[(0, 7_000.0)])], &none);
        assert!(d.phase1.is_empty());
        assert_eq!(ids(&d.phase2), vec![1]);

        // Below the fallback: a decrease, even though the change is under the threshold.
        let d = run(1_000.0, &[target(1, &[(0, 1_000.0)])], &none);
        assert_eq!(ids(&d.phase1), vec![1]);
        assert!(d.phase2.is_empty());
    }

    /// EVSE 9 isn't in the plan but keeps drawing 45 kW. EVSE 1's small drop (800 W) is
    /// normally held back, but with EVSE 9 counted, holding it back would exceed 54.5 kW.
    #[test]
    fn untargeted_profiles_count_towards_the_total() {
        let eff = effective(&[(1, 10_000.0), (9, 45_000.0)]);
        let targets = [target(1, &[(0, 9_200.0)])];
        assert!(run(100_000.0, &targets, &eff).is_empty());
        assert_eq!(ids(&run(54_500.0, &targets, &eff).phase1), vec![1]);
        // Over the limit whatever is sent: the drop still goes out, as close as it gets.
        assert_eq!(ids(&run(50_000.0, &targets, &eff).phase1), vec![1]);
    }

    #[test]
    fn capacity_is_checked_at_every_step() {
        let a = StepProfile::from_steps(NOW, [(0, 6_000.0), (600, 9_000.0)]);
        let b = StepProfile::constant(NOW - 60, 5_000.0);
        assert!(within_capacity(NOW, 14_000.0, [&a, &b]));
        // Fits now, but not once `a` steps up.
        assert!(!within_capacity(NOW, 12_000.0, [&a, &b]));
        assert!(within_capacity(NOW, 0.0, []));
    }
}
