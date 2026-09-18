//! Power limits over time, in the shape of an OCPP absolute charging schedule.

/// One step of a profile: `limit_w` holds from `start_offset_secs` after the profile start
/// until the next period.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Period {
    pub start_offset_secs: i64,
    pub limit_w: f64,
}

/// A step function of power over time: the first period starts at offset 0 and the last
/// one holds forever, as in an OCPP charging schedule without a duration.
#[derive(Debug, Clone, PartialEq)]
pub struct StepProfile {
    /// Unix time of offset 0 (the schedule's `startSchedule`).
    pub start_secs: i64,
    /// Non-empty, with strictly increasing offsets starting at 0.
    pub periods: Vec<Period>,
}

impl StepProfile {
    /// A single limit that holds from `start_secs` on.
    pub fn constant(start_secs: i64, limit_w: f64) -> Self {
        Self {
            start_secs,
            periods: vec![Period {
                start_offset_secs: 0,
                limit_w,
            }],
        }
    }

    /// Builds a profile from `(offset, limit)` steps in increasing offset order, merging
    /// consecutive steps with the same limit. The first step is moved to offset 0.
    pub fn from_steps(start_secs: i64, steps: impl IntoIterator<Item = (i64, f64)>) -> Self {
        let mut periods: Vec<Period> = Vec::new();
        for (offset, limit_w) in steps {
            match periods.last() {
                None => periods.push(Period {
                    start_offset_secs: 0,
                    limit_w,
                }),
                Some(last) if last.limit_w == limit_w => {}
                Some(last) => {
                    debug_assert!(offset > last.start_offset_secs, "offsets must increase");
                    periods.push(Period {
                        start_offset_secs: offset,
                        limit_w,
                    });
                }
            }
        }
        assert!(!periods.is_empty(), "a profile needs at least one step");
        Self {
            start_secs,
            periods,
        }
    }

    /// The limit in force at unix time `t_secs`. Before the start the first period applies.
    pub fn value_at(&self, t_secs: i64) -> f64 {
        let offset = t_secs - self.start_secs;
        let idx = self
            .periods
            .partition_point(|p| p.start_offset_secs <= offset)
            .saturating_sub(1);
        self.periods[idx].limit_w
    }

    /// The limit of the first period.
    pub fn first_limit_w(&self) -> f64 {
        self.periods[0].limit_w
    }

    /// The lower of the two profiles at every time from `from_secs` on, as a profile that
    /// starts at `from_secs`.
    pub fn pointwise_min(&self, other: &StepProfile, from_secs: i64) -> StepProfile {
        self.combine(other, from_secs, f64::min)
    }

    /// The higher of the two profiles at every time from `from_secs` on, as a profile that
    /// starts at `from_secs`.
    pub fn pointwise_max(&self, other: &StepProfile, from_secs: i64) -> StepProfile {
        self.combine(other, from_secs, f64::max)
    }

    /// The highest limit at any time from `from_secs` on.
    pub fn peak_from(&self, from_secs: i64) -> f64 {
        self.breakpoints()
            .filter(|&t| t > from_secs)
            .map(|t| self.value_at(t))
            .fold(self.value_at(from_secs), f64::max)
    }

    fn combine(
        &self,
        other: &StepProfile,
        from_secs: i64,
        op: impl Fn(f64, f64) -> f64,
    ) -> StepProfile {
        let mut times: Vec<i64> = self
            .breakpoints()
            .chain(other.breakpoints())
            .filter(|&t| t > from_secs)
            .collect();
        times.push(from_secs);
        times.sort_unstable();
        times.dedup();
        StepProfile::from_steps(
            from_secs,
            times
                .into_iter()
                .map(|t| (t - from_secs, op(self.value_at(t), other.value_at(t)))),
        )
    }

    /// Unix times at which the limit may change: the start and every later period start.
    pub fn breakpoints(&self) -> impl Iterator<Item = i64> + '_ {
        self.periods
            .iter()
            .map(|p| self.start_secs + p.start_offset_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_equal_steps_and_starts_at_zero() {
        let p =
            StepProfile::from_steps(100, [(5, 3.0), (10, 3.0), (20, 1.0), (30, 1.0), (40, 0.0)]);
        let offsets: Vec<_> = p
            .periods
            .iter()
            .map(|p| (p.start_offset_secs, p.limit_w))
            .collect();
        assert_eq!(offsets, vec![(0, 3.0), (20, 1.0), (40, 0.0)]);
    }

    #[test]
    fn value_at_follows_the_steps() {
        let p = StepProfile::from_steps(100, [(0, 3.0), (20, 1.0), (40, 0.0)]);
        assert_eq!(p.value_at(50), 3.0);
        assert_eq!(p.value_at(100), 3.0);
        assert_eq!(p.value_at(119), 3.0);
        assert_eq!(p.value_at(120), 1.0);
        assert_eq!(p.value_at(140), 0.0);
        assert_eq!(p.value_at(1_000_000), 0.0);
        assert_eq!(p.breakpoints().collect::<Vec<_>>(), vec![100, 120, 140]);
    }

    #[test]
    fn pointwise_min_takes_the_lower_step() {
        let a = StepProfile::from_steps(100, [(0, 5.0), (60, 10.0)]);
        let b = StepProfile::from_steps(130, [(0, 8.0), (60, 2.0)]);
        let m = a.pointwise_min(&b, 110);
        let steps: Vec<_> = m
            .periods
            .iter()
            .map(|p| (p.start_offset_secs, p.limit_w))
            .collect();
        // From 110: a=5,b=8 → 5; at 160: a=10,b=8 → 8; at 190: b=2 → 2.
        assert_eq!(m.start_secs, 110);
        assert_eq!(steps, vec![(0, 5.0), (50, 8.0), (80, 2.0)]);
    }

    #[test]
    fn pointwise_max_takes_the_higher_step() {
        let a = StepProfile::from_steps(100, [(0, 5.0), (60, 10.0)]);
        let b = StepProfile::from_steps(130, [(0, 8.0), (60, 2.0)]);
        let m = a.pointwise_max(&b, 110);
        let steps: Vec<_> = m
            .periods
            .iter()
            .map(|p| (p.start_offset_secs, p.limit_w))
            .collect();
        // From 110: a=5,b=8 → 8; at 160: a=10 → 10.
        assert_eq!(steps, vec![(0, 8.0), (50, 10.0)]);
    }

    #[test]
    fn peak_from_ignores_the_past() {
        let p = StepProfile::from_steps(100, [(0, 9.0), (20, 1.0), (40, 4.0), (60, 0.0)]);
        assert_eq!(p.peak_from(100), 9.0);
        assert_eq!(p.peak_from(125), 4.0);
        assert_eq!(p.peak_from(1_000), 0.0);
    }
}
