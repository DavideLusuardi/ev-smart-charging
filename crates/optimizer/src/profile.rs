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
}
