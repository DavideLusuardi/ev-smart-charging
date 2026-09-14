//! Equal share: the baseline, a stand-in for static load management.

use crate::types::{Candidate, Schedule, Scheduler, SessionRequest, SiteInput, schedule_per_slot};

/// Splits each slot's capacity equally among the sessions present, capped at what each can
/// take. Capacity a capped session leaves unused goes to the others (water-filling).
///
/// It ignores departure times, so it never trades a session's energy now for another's later.
#[derive(Debug, Clone, Copy, Default)]
pub struct EqualShare;

impl Scheduler for EqualShare {
    fn name(&self) -> &'static str {
        "equal"
    }

    fn schedule(&self, site: &SiteInput, sessions: &[SessionRequest]) -> Schedule {
        schedule_per_slot(site, sessions, water_fill)
    }
}

/// Water-filling: raise a common level until capacity runs out, capping each session at its
/// limit. Visiting sessions from the smallest limit up means each capped session's leftover
/// is shared by everyone after it.
fn water_fill(candidates: &[Candidate<'_>], capacity_w: f64) -> Vec<f64> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&a, &b| candidates[a].limit_w.total_cmp(&candidates[b].limit_w));

    let mut power = vec![0.0; candidates.len()];
    let mut left = capacity_w;
    for (served, &i) in order.iter().enumerate() {
        let share = left / (order.len() - served) as f64;
        let p = candidates[i].limit_w.min(share);
        power[i] = p;
        left -= p;
    }
    power
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::tests::{session, site};

    fn power(site: &SiteInput, sessions: &[SessionRequest]) -> Vec<Vec<f64>> {
        let schedule = EqualShare.schedule(site, sessions);
        assert_eq!(schedule.validate(site, sessions), Ok(()));
        sessions
            .iter()
            .map(|s| schedule.power_w[&s.id].clone())
            .collect()
    }

    #[test]
    fn splits_capacity_equally() {
        let sessions = [session("a", 1, 10.0, 100.0), session("b", 1, 10.0, 100.0)];
        assert_eq!(power(&site(10.0, 1), &sessions), vec![vec![5.0], vec![5.0]]);
    }

    #[test]
    fn redistributes_what_a_capped_session_leaves() {
        let sessions = [
            session("a", 1, 2.0, 100.0),
            session("b", 1, 10.0, 100.0),
            session("c", 1, 10.0, 100.0),
        ];
        assert_eq!(
            power(&site(12.0, 1), &sessions),
            vec![vec![2.0], vec![5.0], vec![5.0]]
        );
    }

    #[test]
    fn stops_when_energy_is_delivered() {
        // One-hour slots: "a" needs only 3 Wh, so it takes 3 W and then nothing.
        let sessions = [session("a", 2, 10.0, 3.0), session("b", 2, 10.0, 100.0)];
        assert_eq!(
            power(&site(10.0, 2), &sessions),
            vec![vec![3.0, 0.0], vec![7.0, 10.0]]
        );
    }

    #[test]
    fn leaves_capacity_unused_when_demand_is_low() {
        let sessions = [session("a", 1, 4.0, 100.0)];
        assert_eq!(power(&site(10.0, 1), &sessions), vec![vec![4.0]]);
    }

    #[test]
    fn departed_sessions_get_nothing() {
        let sessions = [session("a", 1, 10.0, 100.0), session("b", 3, 10.0, 100.0)];
        assert_eq!(
            power(&site(10.0, 3), &sessions),
            vec![vec![5.0, 0.0, 0.0], vec![5.0, 10.0, 10.0]]
        );
    }

    #[test]
    fn negative_capacity_means_zero() {
        let sessions = [session("a", 1, 10.0, 100.0)];
        assert_eq!(power(&site(-5.0, 1), &sessions), vec![vec![0.0]]);
    }
}
