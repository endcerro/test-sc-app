//! Contracts and the universe in, ranked routes out.
//!
//! Resolves contract places (through the caller's resolver), builds travel times
//! ([`travel`](crate::travel)), runs the selection and describes each route for the UI:
//! stops, cargo, money, warnings, how it differs, and why other contracts were left out.
//!
//! Each contract line (flow) becomes one order per drop-off: a line with one pickup and
//! several drop-offs knows the SCU of each; a line with several pickups and one drop-off
//! is split evenly between its pickups (the game doesn't say how much comes from each).
//! A line marked loaded is already on board: it has no pickups.

use std::collections::HashSet;

use sc_contracts::Contract;
use sc_contracts::snap::split_place;
use sc_universe::{LocationId, Universe};
use thiserror::Error;

use crate::output::{CargoMove, ContractOut, ContractStatus, Fit, PlanOutput, RouteOut, StepOut};
use crate::problem::{Job, Order, Problem};
use crate::route::{Move, Route, best_route};
use crate::select::{Candidate, Config, Goal, select};
use crate::settings::{Settings, Ship};
use crate::travel::{Leg, LegKind, leg};

/// Offers leaving the board within this many minutes get an "accept soon" warning.
const ACCEPT_SOON_MINUTES: f64 = 15.0;
/// Contracts a player can hold at once.
pub const MAX_HELD: usize = 10;
/// Job sets are 128-bit masks.
const MAX_BOARD: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    #[error("set where you are first")]
    NoStart,
    #[error("unknown start place \"{0}\"")]
    UnknownStart(String),
}

/// The places of the problem: index 0 is the start.
struct Places {
    ids: Vec<LocationId>,
    /// Name to show: as first written in a contract ("Grim HEX" rather than the data's
    /// "GrimHEX"), without its "on ..."/"above ..." part.
    shown: Vec<String>,
}

impl Places {
    fn index(&mut self, id: LocationId, written: &str) -> usize {
        match self.ids.iter().position(|x| *x == id) {
            Some(i) => i,
            None => {
                self.ids.push(id);
                self.shown.push(split_place(written.trim()).0.to_owned());
                self.ids.len() - 1
            }
        }
    }
}

/// "1h 4m", "59m 17s" -> minutes. None for "N/A" or unreadable values.
pub fn availability_minutes(text: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut any = false;
    for tok in text.split_whitespace() {
        let (num, unit) = tok.split_at(tok.find(|c: char| !c.is_ascii_digit())?);
        let n: f64 = num.parse().ok()?;
        total += match unit {
            "h" => n * 60.0,
            "m" => n,
            "s" => n / 60.0,
            _ => return None,
        };
        any = true;
    }
    any.then_some(total)
}

fn goal_name(g: Goal) -> String {
    match g {
        Goal::RatePerHour { min_minutes } => {
            format!("Best reward per hour (sessions of at least {min_minutes:.0} min)")
        }
        Goal::MaxReward { budget_minutes } => {
            format!("Highest reward within {budget_minutes:.0} min")
        }
        Goal::CompleteAll => "Shortest time to complete every contract".into(),
    }
}

/// Resolved places of one contract line: (pickups, drop-offs).
type Resolved = (Vec<LocationId>, Vec<LocationId>);

/// Why a contract can't be planned, or its resolved places.
fn check(
    c: &Contract,
    resolve: &impl Fn(&str) -> Option<LocationId>,
) -> Result<Vec<Resolved>, String> {
    if c.reward.is_none() {
        return Err("no readable reward".into());
    }
    if c.flows.is_empty() {
        return Err("no cargo".into());
    }
    let place = |name: &str| {
        if name.trim().is_empty() {
            Err("a place is missing".to_owned())
        } else {
            resolve(name).ok_or_else(|| format!("unknown or ambiguous place \"{}\"", name.trim()))
        }
    };
    let mut out = Vec::new();
    for f in &c.flows {
        let what = f.display_commodity();
        if !f.is_valid() {
            return Err(format!(
                "{what} has several pickups and several drop-offs: the amount per place is unknown"
            ));
        }
        if f.dropoffs.is_empty() {
            return Err(format!("no drop-off for {what}"));
        }
        if f.pickups.is_empty() && !f.loaded {
            return Err(format!("no pickup for {what}"));
        }
        if f.dropoffs.iter().any(|d| d.scu == 0) {
            return Err(format!("cargo amount missing for {what}"));
        }
        let pickups = if f.loaded {
            Vec::new()
        } else {
            f.pickups
                .iter()
                .map(|p| place(&p.name))
                .collect::<Result<_, _>>()?
        };
        let drops = f
            .dropoffs
            .iter()
            .map(|d| place(&d.place.name))
            .collect::<Result<_, _>>()?;
        out.push((pickups, drops));
    }
    Ok(out)
}

/// Plans `contracts` (numbered 1.. in slice order) from `s.start`. `resolve` turns a
/// written place name into a location, or None when unknown or ambiguous.
pub fn plan(
    contracts: &[Contract],
    u: &Universe,
    resolve: impl Fn(&str) -> Option<LocationId>,
    ship: &Ship,
    s: &Settings,
) -> Result<PlanOutput, PlanError> {
    if s.start.trim().is_empty() {
        return Err(PlanError::NoStart);
    }
    let start =
        resolve(&s.start).ok_or_else(|| PlanError::UnknownStart(s.start.trim().to_owned()))?;
    let mut places = Places {
        ids: vec![start],
        shown: vec![u.get(start).name.clone()],
    };
    let mut issues = Vec::new();

    // Contracts -> jobs. `job_contract[j]` is the contract index of job j, and
    // `job_lines[j][k]` the contract line behind the job's order k.
    let mut jobs: Vec<Job> = Vec::new();
    let mut job_contract: Vec<usize> = Vec::new();
    let mut job_lines: Vec<Vec<usize>> = Vec::new();
    let mut status: Vec<Option<ContractStatus>> = vec![None; contracts.len()];
    for (ci, c) in contracts.iter().enumerate() {
        let resolved = match check(c, &resolve) {
            Ok(r) => r,
            Err(why) => {
                status[ci] = Some(ContractStatus::LeftOut(why));
                continue;
            }
        };
        let mut orders = Vec::new();
        let mut line_of = Vec::new();
        for (fi, (f, (picks, drops))) in c.flows.iter().zip(&resolved).enumerate() {
            let pickups: Vec<usize> = picks
                .iter()
                .zip(&f.pickups)
                .map(|(&id, p)| places.index(id, &p.name))
                .collect();
            for (d, &id) in f.dropoffs.iter().zip(drops) {
                orders.push(Order {
                    scu: f64::from(d.scu),
                    pickups: pickups.clone(),
                    drop: places.index(id, &d.place.name),
                });
                line_of.push(fi);
            }
        }
        jobs.push(Job {
            reward: c.reward.unwrap_or(0) as f64,
            orders,
        });
        job_contract.push(ci);
        job_lines.push(line_of);
    }

    let n = places.ids.len();
    let legs: Vec<Vec<Leg>> = (0..n)
        .map(|a| {
            (0..n)
                .map(|b| leg(u, ship, s, places.ids[a], places.ids[b]))
                .collect()
        })
        .collect();
    let travel = legs
        .iter()
        .map(|row| row.iter().map(|x| x.minutes).collect())
        .collect();
    let p = Problem {
        travel,
        start: 0,
        stop_minutes: s.stop_minutes,
        jobs,
        hard_max: ship.hard_max,
    };

    // Contracts that can't be done alone are left out up front.
    let mut eligible = Vec::new();
    for (j, &ci) in job_contract.iter().enumerate() {
        if best_route(&p, &[j]).is_none() {
            let total: f64 = p.jobs[j].orders.iter().map(|o| o.scu).sum();
            status[ci] = Some(ContractStatus::LeftOut(match ship.hard_max {
                Some(m) if total > m => {
                    format!("needs multiple trips ({total:.0} SCU, hard maximum {m:.0})")
                }
                _ => "no route keeps cargo under the hard maximum".into(),
            }));
        } else if eligible.len() == MAX_BOARD {
            status[ci] = Some(ContractStatus::LeftOut(format!(
                "board too large (over {MAX_BOARD} contracts)"
            )));
        } else {
            eligible.push(j);
        }
    }

    let goal = s.goal;
    let max_jobs = MAX_HELD.saturating_sub(s.accepted as usize);
    if goal == Goal::CompleteAll && eligible.len() > max_jobs {
        issues.push(format!(
            "{} contracts entered, but only {max_jobs} more can be held at once",
            eligible.len()
        ));
    }
    let mut cfg = Config::new(goal, max_jobs);
    cfg.alternatives = s.alternatives;
    cfg.beam = s.beam;
    cfg.expand = s.expand;
    let candidates = if max_jobs == 0 {
        issues.push(format!(
            "Already holding {MAX_HELD} contracts: nothing more can be accepted"
        ));
        Vec::new()
    } else {
        select(&p, &eligible, &cfg)
    };
    if candidates.is_empty() && !eligible.is_empty() {
        issues.push("No route fits the goal; try a longer time budget".into());
    }

    let ctx = Describe {
        contracts,
        job_contract: &job_contract,
        job_lines: &job_lines,
        places: &places,
        legs: &legs,
        p: &p,
        ship,
        goal,
        hop_minutes: s.hop_minutes,
        u,
    };
    let mut routes: Vec<RouteOut> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| ctx.route(i + 1, c))
        .collect();
    label(&mut routes, goal);

    // Why each other contract isn't in the top route.
    if let Some(top) = candidates.first() {
        for &j in &eligible {
            let ci = job_contract[j];
            if top.jobs.contains(&j) {
                status[ci] = Some(ContractStatus::TopRoute);
                continue;
            }
            if let Some(k) = candidates.iter().position(|c| c.jobs.contains(&j)) {
                status[ci] = Some(ContractStatus::Alternative(k + 1));
                continue;
            }
            let Some(solo) = best_route(&p, &[j]) else {
                continue;
            };
            let solo_rate = p.jobs[j].reward / (solo.total() / 60.0);
            let why = if top.jobs.len() >= max_jobs {
                format!("the route already holds the maximum of {max_jobs} contracts")
            } else {
                match goal {
                    Goal::MaxReward { budget_minutes } if solo.total() > budget_minutes => format!(
                        "takes {:.0} min alone, over the {budget_minutes:.0}-minute budget",
                        solo.total()
                    ),
                    Goal::MaxReward { .. } => {
                        "doesn't fit the time budget with the chosen contracts".into()
                    }
                    _ if solo_rate < top.rate() => {
                        format!("poor pay for the time ({} /h alone)", thousands(solo_rate))
                    }
                    _ => "too far from the other stops".into(),
                }
            };
            status[ci] = Some(ContractStatus::LeftOut(why));
        }
    }

    let contracts_out = contracts
        .iter()
        .enumerate()
        .map(|(ci, c)| ContractOut {
            contract: ci + 1,
            title: c.title.clone(),
            reward: c.reward,
            scu: c.total_scu(),
            status: status[ci].clone().unwrap_or(ContractStatus::NotInAnyRoute),
        })
        .collect();

    Ok(PlanOutput {
        goal: goal_name(goal),
        start: places.shown[0].clone(),
        stop_minutes: s.stop_minutes,
        hop_minutes: s.hop_minutes,
        max_contracts: max_jobs,
        routes,
        contracts: contracts_out,
        issues,
        data_version: u.meta().game_version.clone(),
    })
}

fn thousands(v: f64) -> String {
    sc_contracts::truth::thousands(v.max(0.0).round() as u64)
}

/// What describing a route needs from the planning.
struct Describe<'a> {
    contracts: &'a [Contract],
    job_contract: &'a [usize],
    job_lines: &'a [Vec<usize>],
    places: &'a Places,
    legs: &'a [Vec<Leg>],
    p: &'a Problem,
    ship: &'a Ship,
    goal: Goal,
    hop_minutes: f64,
    u: &'a Universe,
}

impl Describe<'_> {
    fn cargo(&self, m: &Move) -> CargoMove {
        let ci = self.job_contract[m.job];
        let line = self.job_lines[m.job][m.order];
        let f = &self.contracts[ci].flows[line];
        CargoMove {
            contract: ci + 1,
            line: line + 1,
            commodity: f.commodity.clone(),
            scu: (m.scu * 100.0).round() / 100.0,
            restricted: f.restricted,
            estimated_split: f.pickups.len() > 1,
        }
    }

    fn route(&self, rank: usize, c: &Candidate) -> RouteOut {
        let r: &Route = &c.route;
        let mv = |m: &Move| self.cargo(m);
        let mut prev = self.p.start;
        let mut clock = 0.0;
        let mut steps = Vec::new();
        for (i, st) in r.steps.iter().enumerate() {
            let travel = self.legs[prev][st.loc].clone();
            clock += travel.minutes;
            let id = self.places.ids[st.loc];
            steps.push(StepOut {
                n: i + 1,
                location: self.places.shown[st.loc].clone(),
                system: self.u.get(self.u.get(id).system).name.clone(),
                drop_only: st.drop_only,
                travel,
                arrive_minutes: clock,
                delivered: merged(st.delivered.iter().map(mv)),
                picked: merged(st.picked.iter().map(mv)),
                cargo_after: (st.load_after * 100.0).round() / 100.0,
            });
            clock += self.p.stop_minutes;
            prev = st.loc;
        }

        let in_route: Vec<usize> = c.jobs.iter().map(|&j| self.job_contract[j]).collect();
        let warnings = self.warnings(rank, r, &in_route, &steps);
        let fit = match self.ship.capacity_estimate {
            Some(cap) if r.peak <= 0.8 * cap => Fit::Comfortable,
            Some(cap) if r.peak <= cap => Fit::Tight,
            Some(_) => Fit::OverEstimate,
            None => Fit::Unknown,
        };
        let on_board = c.jobs.iter().flat_map(|&j| {
            self.p.jobs[j]
                .orders
                .iter()
                .enumerate()
                .filter(|(_, o)| o.pickups.is_empty())
                .map(move |(k, o)| Move {
                    job: j,
                    order: k,
                    scu: o.scu,
                })
        });
        RouteOut {
            rank,
            label: String::new(),
            contracts: in_route.iter().map(|ci| ci + 1).collect(),
            reward: c.reward.round() as u64,
            reward_per_hour: c.rate().round() as u64,
            total_minutes: r.total(),
            quantum_minutes: steps.iter().map(|st| st.travel.quantum_minutes).sum(),
            stop_minutes: r.stop_minutes,
            stops: r.steps.len(),
            peak_scu: (r.peak * 100.0).round() / 100.0,
            on_board_at_start: merged(on_board.map(|m| self.cargo(&m))),
            fit,
            exact_order: r.exact,
            warnings,
            steps,
        }
    }

    fn warnings(
        &self,
        rank: usize,
        r: &Route,
        in_route: &[usize],
        steps: &[StepOut],
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        if let Goal::RatePerHour { min_minutes } = self.goal
            && r.total() < min_minutes
        {
            warnings.push(if rank == 1 {
                format!(
                    "Shorter than your {min_minutes:.0}-minute minimum: nothing longer was found"
                )
            } else {
                format!("Shorter than your {min_minutes:.0}-minute minimum")
            });
        }
        let restricted: Vec<String> = in_route
            .iter()
            .filter(|&&ci| self.contracts[ci].flows.iter().any(|f| f.restricted))
            .map(|ci| (ci + 1).to_string())
            .collect();
        if !restricted.is_empty() {
            warnings.push(format!(
                "Carries restricted cargo (contracts {})",
                restricted.join(", ")
            ));
        }
        for &ci in in_route {
            let ct = &self.contracts[ci];
            let n = ci + 1;
            if let (Some(cb), Some(sb)) = (ct.max_box, self.ship.max_box)
                && cb > sb
            {
                warnings.push(format!(
                    "Contract {n}: containers up to {cb} SCU, your ship takes {sb} SCU"
                ));
            }
            if let Some(m) = ct.availability.as_deref().and_then(availability_minutes)
                && m <= ACCEPT_SOON_MINUTES
            {
                warnings.push(format!(
                    "Contract {n}: leaves the board in about {m:.0} min, accept it first"
                ));
            }
            for f in ct.flows.iter().filter(|f| f.pickups.len() > 1 && !f.loaded) {
                warnings.push(format!(
                    "Contract {n}: how {} is split between its {} pickups is unknown; assumed even",
                    f.commodity,
                    f.pickups.len()
                ));
            }
            let mut inferred: Vec<&str> = Vec::new();
            for f in &ct.flows {
                let pickups = f.pickups.iter().filter(|_| !f.loaded);
                for p in pickups.chain(f.dropoffs.iter().map(|d| &d.place)) {
                    if p.uncertain && !inferred.contains(&p.name.as_str()) {
                        inferred.push(&p.name);
                    }
                }
            }
            if !inferred.is_empty() {
                warnings.push(format!(
                    "Contract {n}: check {} (inferred, not read)",
                    inferred.join(", ")
                ));
            }
        }
        let count = |kind: LegKind| steps.iter().filter(|st| st.travel.kind == kind).count();
        let hops = count(LegKind::SameBody);
        if hops > 0 {
            warnings.push(format!(
                "{hops} same-planet hop(s) use your hop time of {:.0} min",
                self.hop_minutes
            ));
        }
        let missing = count(LegKind::Missing);
        if missing > 0 {
            warnings.push(format!(
                "{missing} leg(s) have no distance data and use a rough guess"
            ));
        }
        if !r.exact {
            warnings.push("Too many stops to prove the shortest order; this one is close".into());
        }
        warnings
    }
}

/// One move per contract line: the shares of a line loaded or delivered at the same
/// stop (one per drop-off) are shown together.
fn merged(moves: impl Iterator<Item = CargoMove>) -> Vec<CargoMove> {
    let mut out: Vec<CargoMove> = Vec::new();
    for m in moves {
        match out
            .iter_mut()
            .find(|x| (x.contract, x.line) == (m.contract, m.line))
        {
            Some(x) => x.scu = ((x.scu + m.scu) * 100.0).round() / 100.0,
            None => out.push(m),
        }
    }
    out
}

/// Names what each route is best at among those shown; each label is used once.
/// The goal's own measure comes first.
fn label(routes: &mut [RouteOut], goal: Goal) {
    type Metric = (&'static str, fn(&RouteOut) -> f64);
    let mut metrics: Vec<Metric> = vec![
        ("Best pay per hour", |r| r.reward_per_hour as f64),
        ("Highest total reward", |r| r.reward as f64),
        ("Shortest time", |r| -r.total_minutes),
        ("Fewest stops", |r| -(r.stops as f64)),
        ("Least cargo", |r| -r.peak_scu),
    ];
    let first = match goal {
        Goal::RatePerHour { .. } => 0,
        Goal::MaxReward { .. } => 1,
        Goal::CompleteAll => 2,
    };
    let m = metrics.remove(first);
    metrics.insert(0, m);
    let mut used: HashSet<&str> = HashSet::new();
    for i in 0..routes.len() {
        let mut chosen = "Another set of contracts".to_owned();
        for &(name, f) in &metrics {
            if used.contains(name) {
                continue;
            }
            let best = routes.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
            if (f(&routes[i]) - best).abs() < 1e-9 {
                used.insert(name);
                chosen = name.to_owned();
                break;
            }
        }
        routes[i].label = chosen;
    }
}

#[cfg(test)]
mod tests {
    use super::availability_minutes;

    #[test]
    fn parses_availability() {
        assert_eq!(availability_minutes("1h 4m"), Some(64.0));
        assert_eq!(availability_minutes("59m 30s"), Some(59.5));
        assert_eq!(availability_minutes("N/A"), None);
    }
}
