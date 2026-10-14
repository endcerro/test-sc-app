//! Hauling route planning: which contracts to take, and in what order to visit their
//! places.
//!
//! - [`problem`]: the plain-number model (travel minutes, jobs, orders). No game data.
//! - [`route`]: shortest order of stops for a fixed set of jobs (exact A*, heuristic
//!   fallback for large sets).
//! - [`select`]: which jobs to take for a [`Goal`], with distinct alternatives.
//! - [`travel`]: minutes between two universe locations for a ship.
//! - [`plan`]: contracts and the universe in, ranked and explained routes out
//!   ([`PlanOutput`]).
//!
//! Pure: no IO. Place names are resolved by a function the caller passes, so the app's
//! taught names apply.

pub mod output;
pub mod plan;
pub mod problem;
pub mod route;
pub mod select;
pub mod settings;
pub mod travel;

pub use output::{CargoMove, ContractOut, ContractStatus, Fit, PlanOutput, RouteOut, StepOut};
pub use plan::{MAX_HELD, PlanError, availability_minutes, plan};
pub use select::Goal;
pub use settings::{Settings, Ship};
pub use travel::{Leg, LegKind, leg};

#[cfg(test)]
mod tests {
    use crate::problem::{Job, Order, Problem};
    use crate::route::{Visit, best_route, simulate};
    use crate::select::{Config, Goal, brute_force, rank_key, select};

    /// Small deterministic generator (no external crates).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn float(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (self.next() % 10_000) as f64 / 10_000.0 * (hi - lo)
        }
    }

    /// Random world: `n` places on a plane (index 0 is the start), random jobs.
    fn world(
        rng: &mut Rng,
        n: usize,
        jobs: usize,
        max_orders: usize,
        hard_max: Option<f64>,
    ) -> Problem {
        let pts: Vec<(f64, f64)> = (0..n)
            .map(|_| (rng.float(0.0, 40.0), rng.float(0.0, 40.0)))
            .collect();
        let travel = pts
            .iter()
            .map(|a| {
                pts.iter()
                    .map(|b| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt())
                    .collect()
            })
            .collect();
        let jobs = (0..jobs)
            .map(|_| {
                let orders = (0..1 + rng.below(max_orders))
                    .map(|_| {
                        let drop = 1 + rng.below(n - 1);
                        let mut pickups = vec![];
                        for _ in 0..1 + rng.below(2) {
                            let pk = 1 + rng.below(n - 1);
                            if pk != drop && !pickups.contains(&pk) {
                                pickups.push(pk);
                            }
                        }
                        if pickups.is_empty() {
                            pickups.push(if drop == 1 { 2 } else { 1 });
                        }
                        Order {
                            scu: rng.float(1.0, 60.0).round(),
                            pickups,
                            drop,
                        }
                    })
                    .collect();
                Job {
                    reward: rng.float(10_000.0, 300_000.0).round(),
                    orders,
                }
            })
            .collect();
        Problem {
            travel,
            start: 0,
            stop_minutes: 5.0,
            jobs,
            hard_max,
        }
    }

    /// Independent check: depth-first search over every visit sequence up to a
    /// length, with its own copy of the visit rules.
    fn brute_route(p: &Problem, jobs: &[usize], max_len: usize) -> Option<f64> {
        #[derive(Clone)]
        struct S {
            picked: Vec<Vec<bool>>,
            delivered: Vec<bool>,
            load: f64,
        }
        let orders: Vec<&Order> = jobs.iter().flat_map(|&j| p.jobs[j].orders.iter()).collect();
        let mut locs: Vec<usize> = orders
            .iter()
            .flat_map(|o| o.pickups.iter().copied().chain([o.drop]))
            .collect();
        locs.sort_unstable();
        locs.dedup();
        #[allow(clippy::too_many_arguments)]
        fn go(
            p: &Problem,
            orders: &[&Order],
            locs: &[usize],
            s: S,
            cur: usize,
            cost: f64,
            left: usize,
            best: &mut Option<f64>,
        ) {
            if s.delivered.iter().all(|&d| d) {
                if best.is_none_or(|b| cost < b) {
                    *best = Some(cost);
                }
                return;
            }
            if left == 0 || best.is_some_and(|b| cost >= b) {
                return;
            }
            for &l in locs {
                for drop_only in [false, true] {
                    let mut n = s.clone();
                    let mut did = false;
                    for (i, o) in orders.iter().enumerate() {
                        if !n.delivered[i] && o.drop == l && n.picked[i].iter().all(|&x| x) {
                            n.delivered[i] = true;
                            n.load -= o.scu;
                            did = true;
                        }
                    }
                    if !drop_only {
                        for (i, o) in orders.iter().enumerate() {
                            for (k, &pk) in o.pickups.iter().enumerate() {
                                if !n.delivered[i] && pk == l && !n.picked[i][k] {
                                    n.picked[i][k] = true;
                                    n.load += o.scu / o.pickups.len() as f64;
                                    did = true;
                                }
                            }
                        }
                    }
                    if did && p.fits(n.load) {
                        go(
                            p,
                            orders,
                            locs,
                            n,
                            l,
                            cost + p.t(cur, l) + p.stop_minutes,
                            left - 1,
                            best,
                        );
                    }
                }
            }
        }
        let s = S {
            picked: orders
                .iter()
                .map(|o| vec![false; o.pickups.len()])
                .collect(),
            delivered: vec![false; orders.len()],
            load: 0.0,
        };
        let mut best = None;
        go(p, &orders, &locs, s, p.start, 0.0, max_len, &mut best);
        best
    }

    /// Checks a route against the rules directly from its steps.
    fn assert_valid(p: &Problem, jobs: &[usize], visits: &[Visit]) {
        let r = simulate(p, jobs, visits).unwrap_or_else(|| panic!("route replays"));
        let mut picked = std::collections::HashMap::new();
        for s in &r.steps {
            for d in &s.delivered {
                let o = &p.jobs[d.job].orders[d.order];
                let got: f64 = *picked.get(&(d.job, d.order)).unwrap_or(&0.0);
                assert!((got - o.scu).abs() < 1e-6, "delivered before every pickup");
            }
            for m in &s.picked {
                *picked.entry((m.job, m.order)).or_insert(0.0) += m.scu;
            }
            assert!(p.fits(s.load_after), "over hard maximum");
        }
    }

    #[test]
    fn exact_ordering_matches_brute_force() {
        let mut rng = Rng(7);
        let mut checked = 0;
        for case in 0..120 {
            let hard_max = if case % 3 == 0 { Some(70.0) } else { None };
            let p = world(&mut rng, 6, 3, 2, hard_max);
            let jobs: Vec<usize> = (0..p.jobs.len()).collect();
            let brute = brute_route(&p, &jobs, 7);
            match best_route(&p, &jobs) {
                Some(r) => {
                    assert!(r.exact);
                    assert_valid(&p, &jobs, &r.visits());
                    let b = brute.unwrap_or_else(|| panic!("brute force finds a route too"));
                    assert!(
                        (r.total() - b).abs() < 1e-6,
                        "case {case}: solver {} vs brute force {b}",
                        r.total()
                    );
                    checked += 1;
                }
                None => assert!(
                    brute.is_none(),
                    "case {case}: solver found no route, brute force did"
                ),
            }
        }
        assert!(checked > 80, "most cases should be feasible, got {checked}");
    }

    #[test]
    fn selection_matches_brute_force() {
        let mut rng = Rng(42);
        for case in 0..25 {
            let p = world(
                &mut rng,
                7,
                8,
                2,
                if case % 2 == 0 { Some(90.0) } else { None },
            );
            let eligible: Vec<usize> = (0..p.jobs.len())
                .filter(|&j| best_route(&p, &[j]).is_some())
                .collect();
            for goal in [
                Goal::RatePerHour { min_minutes: 40.0 },
                Goal::MaxReward {
                    budget_minutes: 90.0,
                },
            ] {
                let picked = select(&p, &eligible, &Config::new(goal, 10));
                let brute = brute_force(&p, &eligible, goal, 10);
                match (picked.first(), brute) {
                    (Some(a), Some(b)) => {
                        assert_eq!(
                            rank_key(a, goal),
                            rank_key(&b, goal),
                            "case {case} {goal:?}: {:?} vs {:?}",
                            a.jobs,
                            b.jobs
                        );
                        assert_valid(&p, &a.jobs, &a.route.visits());
                    }
                    (None, None) => {}
                    (a, b) => panic!(
                        "case {case} {goal:?}: solver {:?} vs brute {:?}",
                        a.map(|c| &c.jobs),
                        b.map(|c| c.jobs)
                    ),
                }
            }
        }
    }

    #[test]
    fn respects_job_cap_and_returns_distinct_sets() {
        let mut rng = Rng(3);
        let p = world(&mut rng, 8, 20, 2, None);
        let eligible: Vec<usize> = (0..20).collect();
        let picked = select(
            &p,
            &eligible,
            &Config::new(Goal::RatePerHour { min_minutes: 600.0 }, 4),
        );
        assert!(!picked.is_empty());
        for (i, a) in picked.iter().enumerate() {
            assert!(a.jobs.len() <= 4);
            for b in &picked[i + 1..] {
                assert_ne!(a.jobs, b.jobs);
            }
        }
    }

    #[test]
    fn cargo_already_on_board_needs_no_pickup() {
        let travel = vec![vec![0.0, 10.0], vec![10.0, 0.0]];
        let p = Problem {
            travel,
            start: 0,
            stop_minutes: 5.0,
            jobs: vec![Job {
                reward: 1.0,
                orders: vec![Order {
                    scu: 8.0,
                    pickups: vec![],
                    drop: 1,
                }],
            }],
            hard_max: Some(10.0),
        };
        let r = best_route(&p, &[0]).unwrap_or_else(|| panic!("feasible"));
        assert_eq!(r.steps.len(), 1);
        assert_eq!(r.start_load, 8.0);
        assert!((r.total() - 15.0).abs() < 1e-9);
    }

    #[test]
    fn hard_maximum_forces_delivery_between_pickups() {
        // Two 60 SCU jobs from 1 to 2 and 3 to 4, hard maximum 80: can't carry both.
        let n = 5;
        let travel = (0..n)
            .map(|a| (0..n).map(|b| if a == b { 0.0 } else { 1.0 }).collect())
            .collect();
        let jobs = vec![
            Job {
                reward: 1.0,
                orders: vec![Order {
                    scu: 60.0,
                    pickups: vec![1],
                    drop: 2,
                }],
            },
            Job {
                reward: 1.0,
                orders: vec![Order {
                    scu: 60.0,
                    pickups: vec![3],
                    drop: 4,
                }],
            },
        ];
        let p = Problem {
            travel,
            start: 0,
            stop_minutes: 1.0,
            jobs,
            hard_max: Some(80.0),
        };
        let r = best_route(&p, &[0, 1]).unwrap_or_else(|| panic!("feasible"));
        assert!(r.peak <= 80.0);
        assert_eq!(r.steps.len(), 4);
    }
}
