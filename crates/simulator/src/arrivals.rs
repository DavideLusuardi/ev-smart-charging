//! Seeded EV arrivals: when EVs come, how long they stay, and what they want.
//!
//! Arrivals follow a non-homogeneous Poisson process whose rate depends on the hour of the
//! day and on weekends. It is sampled by thinning (Lewis and Shedler): draw candidate
//! times from a homogeneous process at the peak rate, then keep each one with probability
//! `rate(t) / peak`.

use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::weighted::WeightedIndex;
use rand_distr::{Distribution, Exp, LogNormal};

use crate::scenario::{Arrivals, Scenario, TruncatedLogNormal};

/// Unix time the simulation starts: Monday 2026-01-05 00:00 UTC.
pub const START_SECS: i64 = 1_767_571_200;

const DAY_SECS: i64 = 86_400;

/// One EV coming to the site.
#[derive(Debug, Clone, PartialEq)]
pub struct EvArrival {
    pub id: String,
    pub arrival_secs: i64,
    /// When the driver leaves, charged or not.
    pub departure_secs: i64,
    /// Energy the driver wants.
    pub energy_wh: f64,
    /// The EV's own max charging power (the EVSE may be lower).
    pub max_power_w: f64,
    /// Whether the EV reports its needs over ISO 15118.
    pub iso15118: bool,
}

/// Generates every EV arriving during `days` days from [`START_SECS`].
///
/// Times are rounded up to `step_secs`. The ISO 15118 flag is drawn as `u < share` from a
/// uniform `u` taken for every EV, so the EVs themselves don't depend on `iso15118_share`,
/// and a larger share only turns more of the same EVs into ISO ones.
pub fn generate(scenario: &Scenario, seed: u64, days: u32, iso15118_share: f64) -> Vec<EvArrival> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let end = START_SECS + i64::from(days) * DAY_SECS;
    let step = i64::from(scenario.control.step_secs);
    let ev = &scenario.ev;
    let powers = WeightedIndex::new(ev.max_power_w.iter().map(|w| w.weight))
        .expect("validated: positive weights");

    arrival_times(&scenario.arrivals, &mut rng, START_SECS, end)
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let arrival_secs = round_up(t, step);
            let dwell = round_up(sample(&ev.dwell_secs, &mut rng), step);
            let energy_wh = sample(&ev.energy_wh, &mut rng);
            let max_power_w = ev.max_power_w[powers.sample(&mut rng)].value;
            let iso15118 = rng.random::<f64>() < iso15118_share;
            EvArrival {
                id: format!("ev-{:05}", i + 1),
                arrival_secs,
                departure_secs: arrival_secs + dwell.max(step),
                energy_wh,
                max_power_w,
                iso15118,
            }
        })
        .collect()
}

/// Expected arrivals per second at unix time `t`.
pub fn rate_at(arrivals: &Arrivals, t: i64) -> f64 {
    let hour = t.rem_euclid(DAY_SECS) / 3600;
    // 1970-01-01 was a Thursday; 0 is Monday.
    let weekday = (t.div_euclid(DAY_SECS) + 3).rem_euclid(7);
    let factor = if weekday >= 5 {
        arrivals.weekend_factor
    } else {
        1.0
    };
    arrivals.rate_per_hour[hour as usize] * factor / 3600.0
}

fn arrival_times(arrivals: &Arrivals, rng: &mut ChaCha8Rng, start: i64, end: i64) -> Vec<f64> {
    let peak = arrivals.rate_per_hour.iter().copied().fold(0.0, f64::max)
        * arrivals.weekend_factor.max(1.0)
        / 3600.0;
    if peak <= 0.0 {
        return Vec::new();
    }
    let gaps = Exp::new(peak).expect("validated: finite positive rate");
    let mut times = Vec::new();
    let mut t = start as f64;
    loop {
        t += gaps.sample(rng);
        if t >= end as f64 {
            return times;
        }
        if rng.random::<f64>() * peak < rate_at(arrivals, t.floor() as i64) {
            times.push(t);
        }
    }
}

/// Draws from the lognormal until the value lands in `[min, max]`, clamping after too
/// many misses (only a badly placed range gets there).
fn sample(d: &TruncatedLogNormal, rng: &mut ChaCha8Rng) -> f64 {
    let dist = LogNormal::new(d.median.ln(), d.sigma).expect("validated: sigma >= 0");
    for _ in 0..1_000 {
        let x = dist.sample(rng);
        if (d.min..=d.max).contains(&x) {
            return x;
        }
    }
    dist.sample(rng).clamp(d.min, d.max)
}

fn round_up(t: f64, step: i64) -> i64 {
    (t / step as f64).ceil() as i64 * step
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workplace() -> Scenario {
        Scenario::load("workplace").unwrap()
    }

    #[test]
    fn start_is_a_monday_midnight() {
        assert_eq!(START_SECS % DAY_SECS, 0);
        let a = &workplace().arrivals;
        assert_eq!(rate_at(a, START_SECS + 8 * 3600), 9.0 / 3600.0);
        // Saturday 08:00.
        let saturday = START_SECS + 5 * DAY_SECS + 8 * 3600;
        assert!((rate_at(a, saturday) - 9.0 * 0.15 / 3600.0).abs() < 1e-12);
    }

    #[test]
    fn same_seed_same_evs() {
        let s = workplace();
        assert_eq!(generate(&s, 7, 3, 0.3), generate(&s, 7, 3, 0.3));
        assert_ne!(generate(&s, 7, 3, 0.3), generate(&s, 8, 3, 0.3));
    }

    #[test]
    fn samples_respect_the_scenario() {
        let s = workplace();
        let evs = generate(&s, 1, 14, 0.3);
        assert!(!evs.is_empty());
        let end = START_SECS + 14 * DAY_SECS;
        for ev in &evs {
            assert!((START_SECS..=end).contains(&ev.arrival_secs));
            assert_eq!(ev.arrival_secs % 10, 0);
            let dwell = (ev.departure_secs - ev.arrival_secs) as f64;
            assert!((s.ev.dwell_secs.min..=s.ev.dwell_secs.max + 10.0).contains(&dwell));
            assert!((s.ev.energy_wh.min..=s.ev.energy_wh.max).contains(&ev.energy_wh));
            assert!(s.ev.max_power_w.iter().any(|w| w.value == ev.max_power_w));
            // Nobody arrives when the rate is zero.
            assert!(rate_at(&s.arrivals, ev.arrival_secs - 1) > 0.0);
        }
        assert!(
            evs.windows(2)
                .all(|w| w[0].arrival_secs <= w[1].arrival_secs)
        );
    }

    #[test]
    fn arrival_count_matches_the_rate() {
        let s = workplace();
        let weeks = 20;
        let n = generate(&s, 3, weeks * 7, 0.3).len() as f64;
        let per_weekday: f64 = s.arrivals.rate_per_hour.iter().sum();
        let expected = f64::from(weeks) * per_weekday * (5.0 + 2.0 * s.arrivals.weekend_factor);
        // Poisson: the standard deviation is the square root of the mean.
        assert!(
            (n - expected).abs() < 4.0 * expected.sqrt(),
            "{n} arrivals, expected {expected}"
        );
    }

    #[test]
    fn a_larger_iso_share_only_adds_iso_evs() {
        let s = workplace();
        let low = generate(&s, 5, 7, 0.3);
        let high = generate(&s, 5, 7, 0.6);
        assert_eq!(low.len(), high.len());
        for (a, b) in low.iter().zip(&high) {
            assert_eq!((a.arrival_secs, a.energy_wh), (b.arrival_secs, b.energy_wh));
            assert!(!a.iso15118 || b.iso15118);
        }
        let share = high.iter().filter(|e| e.iso15118).count() as f64 / high.len() as f64;
        assert!((share - 0.6).abs() < 0.1, "{share}");
    }

    #[test]
    fn truncation_keeps_samples_in_range() {
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let d = TruncatedLogNormal {
            median: 100.0,
            sigma: 1.0,
            min: 80.0,
            max: 90.0,
        };
        assert!((0..1_000).all(|_| (80.0..=90.0).contains(&sample(&d, &mut rng))));
        let far = TruncatedLogNormal {
            median: 1.0,
            sigma: 0.01,
            min: 1e6,
            max: 2e6,
        };
        assert_eq!(sample(&far, &mut rng), 1e6);
    }
}
