//! Choosing which jobs to take: a beam search over job sets, each set scored by its
//! best route (from `route`), then a pick of distinct alternatives.

use crate::problem::Problem;
use crate::route::{Route, best_route};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Goal {
    /// Best reward per hour among routes of at least `min_minutes`; shorter routes
    /// only when nothing longer exists.
    RatePerHour { min_minutes: f64 },
    /// Highest total reward within `budget_minutes`.
    MaxReward { budget_minutes: f64 },
    /// Every eligible job, shortest route.
    CompleteAll,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub goal: Goal,
    /// Most jobs in one route (10 minus contracts already accepted).
    pub max_jobs: usize,
    /// Routes to return, best first.
    pub alternatives: usize,
    /// Sets kept per size in the beam search.
    pub beam: usize,
    /// Additions evaluated exactly per kept set.
    pub expand: usize,
}

impl Config {
    pub fn new(goal: Goal, max_jobs: usize) -> Self {
        Self {
            goal,
            max_jobs,
            alternatives: 5,
            beam: 24,
            expand: 8,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    /// Job indexes, ascending.
    pub jobs: Vec<usize>,
    pub reward: f64,
    pub route: Route,
}

impl Candidate {
    pub fn rate(&self) -> f64 {
        self.reward / (self.route.total().max(1e-6) / 60.0)
    }
}

/// Sort key, larger is better. Values within 1% count as a tie, broken by fewer
/// stops, then lower peak cargo.
fn key(c: &Candidate, goal: Goal) -> Option<(i64, i64, i64, i64, i64)> {
    let bucket = |v: f64| {
        if v > 0.0 {
            (v.ln() / 1.01f64.ln()).floor() as i64
        } else {
            i64::MIN / 2
        }
    };
    let stops = -(c.route.steps.len() as i64);
    let peak = -(c.route.peak.round() as i64);
    match goal {
        Goal::RatePerHour { min_minutes } => {
            let long_enough = i64::from(c.route.total() + 1e-9 >= min_minutes);
            Some((
                long_enough,
                bucket(c.rate()),
                stops,
                peak,
                c.rate().round() as i64,
            ))
        }
        Goal::MaxReward { budget_minutes } => {
            (c.route.total() <= budget_minutes + 1e-9).then(|| {
                (
                    1,
                    bucket(c.reward),
                    stops,
                    peak,
                    -(c.route.total() * 60.0).round() as i64,
                )
            })
        }
        Goal::CompleteAll => Some((1, 0, stops, peak, -(c.route.total() * 60.0).round() as i64)),
    }
}

fn mask(jobs: &[usize]) -> u128 {
    jobs.iter().fold(0u128, |m, &j| m | (1u128 << j))
}

struct Search<'a> {
    p: &'a Problem,
    cache: HashMap<u128, Option<Candidate>>,
}

impl<'a> Search<'a> {
    fn eval(&mut self, jobs: &[usize]) -> Option<Candidate> {
        let m = mask(jobs);
        if let Some(c) = self.cache.get(&m) {
            return c.clone();
        }
        let c = best_route(self.p, jobs).map(|route| Candidate {
            jobs: jobs.to_vec(),
            reward: jobs.iter().map(|&j| self.p.jobs[j].reward).sum(),
            route,
        });
        self.cache.insert(m, c.clone());
        c
    }

    /// Cheap guess of the route time after adding `job`: each new location is
    /// inserted where it lengthens the current route least, plus one stop.
    fn estimate(&self, c: &Candidate, job: usize) -> f64 {
        let p = self.p;
        let path: Vec<usize> = std::iter::once(p.start)
            .chain(c.route.steps.iter().map(|s| s.loc))
            .collect();
        let mut extra = 0.0;
        let mut seen: Vec<usize> = path.clone();
        for o in &p.jobs[job].orders {
            for &l in o.pickups.iter().chain([&o.drop]) {
                if seen.contains(&l) {
                    continue;
                }
                seen.push(l);
                let mut cheapest = p.t(*path.last().unwrap_or(&p.start), l);
                for w in path.windows(2) {
                    cheapest = cheapest.min(p.t(w[0], l) + p.t(l, w[1]) - p.t(w[0], w[1]));
                }
                extra += cheapest + p.stop_minutes;
            }
        }
        c.route.total() + extra
    }
}

/// Ranked, distinct candidates for the goal. Job sets are bit masks, so only jobs
/// 0..128 are considered.
pub fn select(p: &Problem, eligible: &[usize], cfg: &Config) -> Vec<Candidate> {
    let eligible: Vec<usize> = eligible.iter().copied().filter(|&j| j < 128).collect();
    let eligible = eligible.as_slice();
    let mut s = Search {
        p,
        cache: HashMap::new(),
    };
    let goal = cfg.goal;

    if goal == Goal::CompleteAll {
        let mut all = eligible.to_vec();
        all.sort_unstable();
        return s.eval(&all).into_iter().collect();
    }

    let mut pool: Vec<Candidate> = Vec::new();
    let mut beam: Vec<Candidate> = Vec::new();
    for &j in eligible {
        if let Some(c) = s.eval(&[j]) {
            if key(&c, goal).is_some() {
                beam.push(c.clone());
            }
            pool.push(c);
        }
    }
    let rank = |v: &mut Vec<Candidate>| {
        v.sort_by_key(|c| std::cmp::Reverse(key(c, goal)));
    };
    rank(&mut beam);
    beam.truncate(cfg.beam);

    for _size in 2..=cfg.max_jobs {
        let mut next: Vec<Candidate> = Vec::new();
        let mut seen: Vec<u128> = Vec::new();
        for c in &beam {
            let mut options: Vec<(f64, usize)> = eligible
                .iter()
                .filter(|j| !c.jobs.contains(j))
                .map(|&j| {
                    let time = s.estimate(c, j);
                    let reward = c.reward + p.jobs[j].reward;
                    let score = match goal {
                        Goal::RatePerHour { min_minutes } => {
                            reward / (time / 60.0) + if time >= min_minutes { 1e12 } else { 0.0 }
                        }
                        Goal::MaxReward { budget_minutes } if time <= budget_minutes * 1.2 => {
                            reward
                        }
                        _ => f64::NEG_INFINITY,
                    };
                    (score, j)
                })
                .filter(|(score, _)| score.is_finite())
                .collect();
            options.sort_by(|a, b| b.0.total_cmp(&a.0));
            for &(_, j) in options.iter().take(cfg.expand) {
                let mut jobs = c.jobs.clone();
                jobs.push(j);
                jobs.sort_unstable();
                let m = mask(&jobs);
                if seen.contains(&m) {
                    continue;
                }
                seen.push(m);
                if let Some(nc) = s.eval(&jobs) {
                    if key(&nc, goal).is_some() {
                        next.push(nc.clone());
                    }
                    pool.push(nc);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        rank(&mut next);
        next.truncate(cfg.beam);
        beam = next;
    }

    pool.retain(|c| key(c, goal).is_some());
    rank(&mut pool);
    distinct(pool, cfg.alternatives)
}

/// Best first, skipping candidates whose job set mostly overlaps one already chosen.
fn distinct(ranked: Vec<Candidate>, n: usize) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for c in ranked {
        let similar = out.iter().any(|o| {
            let shared = c.jobs.iter().filter(|j| o.jobs.contains(j)).count() as f64;
            let union = (c.jobs.len() + o.jobs.len()) as f64 - shared;
            shared / union >= 0.7
        });
        if !similar {
            out.push(c);
        }
        if out.len() == n {
            break;
        }
    }
    out
}

/// Every subset up to `max_jobs`, for tests: the best candidate by the same key.
#[cfg(test)]
pub fn brute_force(
    p: &Problem,
    eligible: &[usize],
    goal: Goal,
    max_jobs: usize,
) -> Option<Candidate> {
    let n = eligible.len();
    assert!(n <= 16);
    let mut best: Option<Candidate> = None;
    for bits in 1u32..(1 << n) {
        if bits.count_ones() as usize > max_jobs {
            continue;
        }
        let jobs: Vec<usize> = (0..n)
            .filter(|i| bits & (1 << i) != 0)
            .map(|i| eligible[i])
            .collect();
        let Some(route) = best_route(p, &jobs) else {
            continue;
        };
        let c = Candidate {
            reward: jobs.iter().map(|&j| p.jobs[j].reward).sum(),
            jobs,
            route,
        };
        if key(&c, goal).is_none() {
            continue;
        }
        if best.as_ref().is_none_or(|b| key(&c, goal) > key(b, goal)) {
            best = Some(c);
        }
    }
    best
}

#[cfg(test)]
pub fn rank_key(c: &Candidate, goal: Goal) -> Option<(i64, i64, i64, i64, i64)> {
    key(c, goal)
}
