//! Linear program: the best schedule for the sessions known now, solved with HiGHS.

use good_lp::{
    Expression, ProblemVariables, ResolutionError, Solution, SolverModel, Variable, constraint,
    highs, variable,
};

use crate::EqualShare;
use crate::types::{Schedule, Scheduler, SessionRequest, SiteInput};

/// Maximizes delivered energy, preferring early delivery.
///
/// Variables `p[i][t]` for each session `i` and each slot `t` before its departure, with
/// `0 ≤ p ≤ max_power`. Constraints: per slot, `Σ_i p[i][t] ≤ capacity[t]`; per session,
/// `Σ_t p[i][t]·Δ ≤ remaining energy`. Objective: maximize `Σ w_t·p[i][t]` with
/// `w_t = 1 − ε·t/T`.
///
/// The weights make the optimum deliver as early as possible, which leaves capacity free
/// for EVs that arrive later. Because every `w_t ≥ 1 − ε`, the delivered energy is at least
/// `(1 − ε)` times the true maximum.
#[derive(Debug, Clone, Copy)]
pub struct Lp {
    /// The earliness preference ε. Must be small, so it only breaks ties in delivered energy.
    pub epsilon: f64,
}

impl Default for Lp {
    fn default() -> Self {
        Self { epsilon: 1e-4 }
    }
}

/// HiGHS failed to solve the LP.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("LP solve failed: {0}")]
pub struct LpError(String);

impl From<ResolutionError> for LpError {
    fn from(e: ResolutionError) -> Self {
        Self(e.to_string())
    }
}

impl Lp {
    /// Solves the LP. The all-zero schedule is always feasible, so an error means a solver
    /// failure rather than an impossible instance.
    pub fn solve(
        &self,
        site: &SiteInput,
        sessions: &[SessionRequest],
    ) -> Result<Schedule, LpError> {
        let hours = site.slot_hours();
        let horizon = site.horizon();

        let mut vars = ProblemVariables::new();
        let p: Vec<Vec<Variable>> = sessions
            .iter()
            .map(|s| {
                let slots = if s.remaining_wh > 0.0 && s.max_power_w > 0.0 {
                    s.active_slots(site)
                } else {
                    0
                };
                vars.add_vector(variable().min(0.0).max(s.max_power_w), slots)
            })
            .collect();

        let mut schedule = Schedule::zeros(site, sessions);
        if p.iter().all(Vec::is_empty) {
            return Ok(schedule);
        }

        let weight = |t: usize| 1.0 - self.epsilon * t as f64 / horizon as f64;
        let objective: Expression = p
            .iter()
            .flat_map(|slots| slots.iter().enumerate())
            .map(|(t, &var)| weight(t) * var)
            .sum();

        let mut problem = vars.maximise(objective).using(highs).set_threads(1);
        for (t, &capacity) in site.capacity_w.iter().enumerate() {
            let load: Expression = p.iter().filter_map(|slots| slots.get(t)).sum();
            problem = problem.with(constraint!(load <= capacity.max(0.0)));
        }
        for (s, slots) in sessions.iter().zip(&p) {
            if !slots.is_empty() {
                let energy: Expression = slots.iter().sum();
                problem = problem.with(constraint!(energy * hours <= s.remaining_wh));
            }
        }

        let solution = problem.solve()?;
        for (s, slots) in sessions.iter().zip(&p) {
            let power = schedule
                .power_w
                .get_mut(&s.id)
                .expect("zeros has every session");
            for (t, &var) in slots.iter().enumerate() {
                power[t] = solution.value(var);
            }
        }
        repair(&mut schedule, site, sessions);
        Ok(schedule)
    }
}

impl Scheduler for Lp {
    fn name(&self) -> &'static str {
        "lp"
    }

    /// Solves the LP, falling back to [`EqualShare`] if HiGHS fails, so the caller always
    /// gets a safe schedule.
    fn schedule(&self, site: &SiteInput, sessions: &[SessionRequest]) -> Schedule {
        self.solve(site, sessions)
            .unwrap_or_else(|_| EqualShare.schedule(site, sessions))
    }
}

/// Removes the solver's tolerance-level violations: clamp to the power bounds, then scale
/// down sessions that exceed their energy, then slots that exceed capacity. Each step only
/// lowers power, so it can't undo the previous ones.
fn repair(schedule: &mut Schedule, site: &SiteInput, sessions: &[SessionRequest]) {
    let hours = site.slot_hours();
    for s in sessions {
        let power = schedule
            .power_w
            .get_mut(&s.id)
            .expect("zeros has every session");
        for p in power.iter_mut() {
            *p = p.clamp(0.0, s.max_power_w.max(0.0));
        }
        let energy = power.iter().sum::<f64>() * hours;
        if energy > s.remaining_wh {
            let scale = (s.remaining_wh / energy).max(0.0);
            power.iter_mut().for_each(|p| *p *= scale);
        }
    }
    for (t, &capacity) in site.capacity_w.iter().enumerate() {
        let total = schedule.slot_total_w(t);
        if total > capacity.max(0.0) {
            let scale = capacity.max(0.0) / total;
            schedule.power_w.values_mut().for_each(|p| p[t] *= scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Edf;
    use crate::types::tests::{session, site};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn beats_both_heuristics_on_the_edf_counterexample() {
        let site = site(10.0, 3);
        let sessions = [session("a", 2, 10.0, 10.0), session("b", 3, 5.0, 15.0)];
        let lp = Lp::default().solve(&site, &sessions).unwrap();
        assert_eq!(lp.validate(&site, &sessions), Ok(()));
        assert!(close(lp.total_energy_wh(&site), 25.0));
        assert!(lp.total_energy_wh(&site) > Edf.schedule(&site, &sessions).total_energy_wh(&site));
    }

    #[test]
    fn delivers_as_early_as_possible() {
        let site = site(10.0, 3);
        let sessions = [session("a", 3, 10.0, 10.0)];
        let lp = Lp::default().solve(&site, &sessions).unwrap();
        let power = &lp.power_w[&"a".into()];
        assert!(
            close(power[0], 10.0) && close(power[1], 0.0) && close(power[2], 0.0),
            "{power:?}"
        );
    }

    #[test]
    fn plans_around_a_capacity_dip() {
        let mut site = site(10.0, 3);
        site.capacity_w[1] = 2.0;
        let sessions = [session("a", 3, 10.0, 25.0), session("b", 2, 10.0, 5.0)];
        let lp = Lp::default().solve(&site, &sessions).unwrap();
        assert_eq!(lp.validate(&site, &sessions), Ok(()));
        assert!(close(lp.total_energy_wh(&site), 22.0), "{:?}", lp.power_w);
    }

    #[test]
    fn handles_trivial_instances() {
        let site = site(10.0, 2);
        let none = Lp::default().solve(&site, &[]).unwrap();
        assert!(none.power_w.is_empty());

        let sessions = [
            session("full", 2, 10.0, 0.0),
            session("gone", 0, 10.0, 50.0),
        ];
        let lp = Lp::default().solve(&site, &sessions).unwrap();
        assert_eq!(lp.total_energy_wh(&site), 0.0);
        assert_eq!(lp.validate(&site, &sessions), Ok(()));
    }

    #[test]
    fn repair_scales_down_overshoots() {
        let site = site(10.0, 2);
        let sessions = [session("a", 2, 10.0, 12.0), session("b", 2, 10.0, 100.0)];
        let mut schedule = Schedule::zeros(&site, &sessions);
        schedule.power_w.insert("a".into(), vec![10.5, 6.0]);
        schedule.power_w.insert("b".into(), vec![5.0, -0.1]);
        repair(&mut schedule, &site, &sessions);
        assert_eq!(schedule.validate(&site, &sessions), Ok(()));
    }
}
