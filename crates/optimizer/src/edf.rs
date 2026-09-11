//! Earliest deadline first: a greedy heuristic that serves the EVs leaving soonest.

use std::cmp::Ordering;

use crate::types::{Candidate, Schedule, Scheduler, SessionRequest, SiteInput, schedule_per_slot};

/// In each slot, serves sessions in order of departure (ties: more energy still needed first,
/// then by id) and gives each as much as it can take until the slot's capacity runs out.
///
/// Not always better than [`EqualShare`](crate::EqualShare): putting all of a slot into an
/// urgent EV can starve a slow one that can't catch up later. See the tests for an example.
#[derive(Debug, Clone, Copy, Default)]
pub struct Edf;

impl Scheduler for Edf {
    fn name(&self) -> &'static str {
        "edf"
    }

    fn schedule(&self, site: &SiteInput, sessions: &[SessionRequest]) -> Schedule {
        schedule_per_slot(site, sessions, earliest_first)
    }
}

fn urgency(a: &Candidate<'_>, b: &Candidate<'_>) -> Ordering {
    a.session
        .departure
        .cmp(&b.session.departure)
        .then(b.needed_wh.total_cmp(&a.needed_wh))
        .then_with(|| a.session.id.cmp(&b.session.id))
}

fn earliest_first(candidates: &[Candidate<'_>], capacity_w: f64) -> Vec<f64> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&a, &b| urgency(&candidates[a], &candidates[b]));

    let mut power = vec![0.0; candidates.len()];
    let mut left = capacity_w;
    for i in order {
        let p = candidates[i].limit_w.min(left);
        power[i] = p;
        left -= p;
    }
    power
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EqualShare;
    use crate::types::tests::{session, site};

    fn power(site: &SiteInput, sessions: &[SessionRequest]) -> Vec<Vec<f64>> {
        let schedule = Edf.schedule(site, sessions);
        assert_eq!(schedule.validate(site, sessions), Ok(()));
        sessions
            .iter()
            .map(|s| schedule.power_w[&s.id].clone())
            .collect()
    }

    #[test]
    fn serves_earliest_departure_first() {
        let sessions = [
            session("late", 3, 10.0, 100.0),
            session("soon", 1, 8.0, 100.0),
        ];
        assert_eq!(power(&site(10.0, 1), &sessions), vec![vec![2.0], vec![8.0]]);
    }

    #[test]
    fn ties_go_to_the_session_needing_more() {
        let sessions = [
            session("small", 2, 10.0, 5.0),
            session("big", 2, 10.0, 50.0),
        ];
        assert_eq!(
            power(&site(10.0, 1), &sessions),
            vec![vec![0.0], vec![10.0]]
        );
    }

    #[test]
    fn stops_when_energy_is_delivered() {
        let sessions = [session("a", 1, 10.0, 4.0), session("b", 2, 10.0, 100.0)];
        assert_eq!(
            power(&site(10.0, 2), &sessions),
            vec![vec![4.0, 0.0], vec![6.0, 10.0]]
        );
    }

    /// EDF front-loads the urgent EV "a" and strands the slow EV "b", which can't use the
    /// capacity freed after "a" is full. Equal share keeps both charging.
    #[test]
    fn edf_can_deliver_less_than_equal_share() {
        let site = site(10.0, 3);
        let sessions = [session("a", 2, 10.0, 10.0), session("b", 3, 5.0, 15.0)];

        let edf = Edf.schedule(&site, &sessions);
        let equal = EqualShare.schedule(&site, &sessions);
        assert_eq!(edf.validate(&site, &sessions), Ok(()));
        assert_eq!(equal.validate(&site, &sessions), Ok(()));

        assert_eq!(edf.power_w[&"a".into()], vec![10.0, 0.0, 0.0]);
        assert_eq!(edf.power_w[&"b".into()], vec![0.0, 5.0, 5.0]);
        assert_eq!(edf.total_energy_wh(&site), 20.0);
        assert_eq!(equal.total_energy_wh(&site), 25.0);
    }
}
