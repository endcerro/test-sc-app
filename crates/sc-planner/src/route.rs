//! Ordering the stops for a chosen set of jobs.
//!
//! A route is a sequence of visits. At each visit the ship first delivers every order
//! whose pickups are all done and whose drop-off is here, then loads everything
//! waiting here. A "drop-off only" visit skips the loading, for when loading would
//! push cargo over the hard maximum. Loading early and delivering early never costs
//! time, so with that rule the only real decision is the order of visits.
//!
//! The order is found exactly with an A* search over (pickups done per location,
//! orders delivered, current location). Sets too large for that fall back to a
//! greedy route improved by local moves.

use crate::problem::Problem;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// States the exact search may expand before giving up and using the heuristic.
const STATE_CAP: usize = 150_000;
const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visit {
    pub loc: usize,
    pub drop_only: bool,
}

/// Cargo moved for one order at one stop.
#[derive(Debug, Clone, PartialEq)]
pub struct Move {
    pub job: usize,
    pub order: usize,
    pub scu: f64,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub loc: usize,
    pub drop_only: bool,
    /// Minutes from the previous stop (or the start).
    pub travel: f64,
    pub delivered: Vec<Move>,
    pub picked: Vec<Move>,
    pub load_after: f64,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub steps: Vec<Step>,
    pub travel_minutes: f64,
    pub stop_minutes: f64,
    pub start_load: f64,
    pub peak: f64,
    /// True when the order is proven shortest; false for the heuristic fallback.
    pub exact: bool,
}

impl Route {
    pub fn total(&self) -> f64 {
        self.travel_minutes + self.stop_minutes
    }

    pub fn visits(&self) -> Vec<Visit> {
        self.steps
            .iter()
            .map(|s| Visit {
                loc: s.loc,
                drop_only: s.drop_only,
            })
            .collect()
    }
}

/// Plays a visit sequence and returns the route, or None when it is invalid: a visit
/// does nothing, cargo exceeds the hard maximum, or an order is left undelivered.
pub fn simulate(p: &Problem, jobs: &[usize], visits: &[Visit]) -> Option<Route> {
    struct O {
        job: usize,
        order: usize,
        scu: f64,
        pickups: Vec<usize>,
        picked: Vec<bool>,
        drop: usize,
        delivered: bool,
    }
    let mut os: Vec<O> = jobs
        .iter()
        .flat_map(|&j| {
            p.jobs[j].orders.iter().enumerate().map(move |(k, o)| O {
                job: j,
                order: k,
                scu: o.scu,
                pickups: o.pickups.clone(),
                picked: vec![false; o.pickups.len()],
                drop: o.drop,
                delivered: false,
            })
        })
        .collect();
    let start_load: f64 = os
        .iter()
        .filter(|o| o.pickups.is_empty())
        .map(|o| o.scu)
        .sum();
    if !p.fits(start_load) {
        return None;
    }
    let (mut load, mut peak, mut cur) = (start_load, start_load, p.start);
    let mut route = Route {
        steps: Vec::new(),
        travel_minutes: 0.0,
        stop_minutes: 0.0,
        start_load,
        peak,
        exact: false,
    };
    for v in visits {
        let mut step = Step {
            loc: v.loc,
            drop_only: v.drop_only,
            travel: p.t(cur, v.loc),
            delivered: vec![],
            picked: vec![],
            load_after: 0.0,
        };
        for o in os
            .iter_mut()
            .filter(|o| !o.delivered && o.drop == v.loc && o.picked.iter().all(|&x| x))
        {
            o.delivered = true;
            load -= o.scu;
            step.delivered.push(Move {
                job: o.job,
                order: o.order,
                scu: o.scu,
            });
        }
        if !v.drop_only {
            for o in os.iter_mut().filter(|o| !o.delivered) {
                let share = o.scu / o.pickups.len().max(1) as f64;
                let mut taken = 0.0;
                for (k, &pk) in o.pickups.iter().enumerate() {
                    if pk == v.loc && !o.picked[k] {
                        o.picked[k] = true;
                        taken += share;
                    }
                }
                if taken > 0.0 {
                    load += taken;
                    step.picked.push(Move {
                        job: o.job,
                        order: o.order,
                        scu: taken,
                    });
                }
            }
        }
        if (step.delivered.is_empty() && step.picked.is_empty()) || !p.fits(load) {
            return None;
        }
        load = load.max(0.0);
        peak = peak.max(load);
        step.load_after = load;
        route.travel_minutes += step.travel;
        route.stop_minutes += p.stop_minutes;
        cur = v.loc;
        route.steps.push(step);
    }
    if os.iter().any(|o| !o.delivered) {
        return None;
    }
    route.peak = peak;
    Some(route)
}

/// Compact view of a job set for the searches: locations renumbered 0..m.
struct Model {
    /// Local index -> problem location.
    locs: Vec<usize>,
    /// Per order: local pickup locations as a bit mask, and SCU.
    need: Vec<u32>,
    scu: Vec<f64>,
    /// Per local location: orders dropped here (bit per order), SCU loaded here.
    drop_mask: Vec<u64>,
    pick_load: Vec<f64>,
    /// Locations with at least one pickup.
    pick_locs: u32,
    all_delivered: u64,
    start_load: f64,
}

impl Model {
    fn new(p: &Problem, jobs: &[usize]) -> Option<Self> {
        let orders: Vec<&crate::problem::Order> =
            jobs.iter().flat_map(|&j| p.jobs[j].orders.iter()).collect();
        let mut locs: Vec<usize> = orders
            .iter()
            .flat_map(|o| o.pickups.iter().copied().chain([o.drop]))
            .collect();
        locs.sort_unstable();
        locs.dedup();
        if locs.len() > 32 || orders.len() > 64 {
            return None;
        }
        // Every pickup and drop-off is in `locs`, so the lookups below always succeed.
        let local = |g: usize| locs.binary_search(&g).ok();
        let mut m = Model {
            need: Vec::new(),
            scu: Vec::new(),
            drop_mask: vec![0; locs.len()],
            pick_load: vec![0.0; locs.len()],
            pick_locs: 0,
            all_delivered: if orders.len() == 64 {
                u64::MAX
            } else {
                (1u64 << orders.len()) - 1
            },
            start_load: 0.0,
            locs: Vec::new(),
        };
        for (i, o) in orders.iter().enumerate() {
            let mut need = 0u32;
            for &pk in &o.pickups {
                let l = local(pk)?;
                let share = o.scu / o.pickups.len() as f64;
                need |= 1 << l;
                m.pick_load[l] += share;
                m.pick_locs |= 1 << l;
            }
            if o.pickups.is_empty() {
                m.start_load += o.scu;
            }
            m.need.push(need);
            m.scu.push(o.scu);
            m.drop_mask[local(o.drop)?] |= 1 << i;
        }
        m.locs = locs;
        Some(m)
    }

    /// Orders deliverable at `l` given what's done.
    fn deliverable(&self, l: usize, done: u32, delivered: u64) -> (u64, f64) {
        let mut candidates = self.drop_mask[l] & !delivered;
        let (mut bits, mut scu) = (0u64, 0.0);
        while candidates != 0 {
            let o = candidates.trailing_zeros() as usize;
            candidates &= candidates - 1;
            if self.need[o] & !done == 0 {
                bits |= 1 << o;
                scu += self.scu[o];
            }
        }
        (bits, scu)
    }

    /// Locations that still need a visit: pickups not done, or undelivered drop-offs.
    fn pending(&self, done: u32, delivered: u64) -> u32 {
        let mut mask = self.pick_locs & !done;
        for (l, &dm) in self.drop_mask.iter().enumerate() {
            if dm & !delivered != 0 {
                mask |= 1 << l;
            }
        }
        mask
    }

    /// Possible visits from a state, written into `out`:
    /// (local location, drop_only, new done, new delivered, new load).
    fn moves(
        &self,
        p: &Problem,
        done: u32,
        delivered: u64,
        load: f64,
        out: &mut Vec<(usize, bool, u32, u64, f64)>,
    ) {
        out.clear();
        for l in 0..self.locs.len() {
            let (bits, unload) = self.deliverable(l, done, delivered);
            let pick_pending = self.pick_locs & !done & (1 << l) != 0;
            if pick_pending {
                let after = load - unload + self.pick_load[l];
                if p.fits(after) {
                    out.push((l, false, done | (1 << l), delivered | bits, after));
                }
                if bits != 0 {
                    out.push((l, true, done, delivered | bits, load - unload));
                }
            } else if bits != 0 {
                out.push((l, false, done, delivered | bits, load - unload));
            }
        }
    }
}

/// Small fast hasher for the search's state table (std's default is built for
/// untrusted input and is several times slower here).
#[derive(Default)]
struct FxHasher(u64);
impl std::hash::Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.add(u64::from(b));
        }
    }
    fn write_u8(&mut self, v: u8) {
        self.add(u64::from(v));
    }
    fn write_u32(&mut self, v: u32) {
        self.add(u64::from(v));
    }
    fn write_u64(&mut self, v: u64) {
        self.add(v);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}
impl FxHasher {
    fn add(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}
type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FxHasher>>;

enum Exact {
    Found(Vec<Visit>),
    Infeasible,
    TooBig,
}

#[derive(PartialEq)]
struct Entry {
    f: f64,
    g: f64,
    node: usize,
}
impl Eq for Entry {}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f)
            .then(o.g.total_cmp(&self.g))
            .then(o.node.cmp(&self.node))
    }
}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// A* over (pickups done, orders delivered, location). The estimate of what's left
/// is one stop per location that still needs work, plus the trip to the nearest of
/// them: never more than the real remainder, so the result is still the shortest.
fn exact(p: &Problem, m: &Model) -> Exact {
    const AT_START: u8 = u8::MAX;
    struct Node {
        done: u32,
        delivered: u64,
        cur: u8,
        load: f64,
        parent: usize,
        visit: Option<Visit>,
    }
    if !p.fits(m.start_load) {
        return Exact::Infeasible;
    }
    // Every location still needing work is first reached from the current position
    // or from another such location, so the cheapest such trip into each one is a
    // lower bound on the remaining travel.
    let k = m.locs.len();
    let local_t: Vec<f64> = (0..k * k)
        .map(|i| p.t(m.locs[i / k], m.locs[i % k]))
        .collect();
    let h = |done: u32, delivered: u64, from: usize| {
        let pend = m.pending(done, delivered);
        let mut total = f64::from(pend.count_ones()) * p.stop_minutes;
        let mut targets = pend;
        while targets != 0 {
            let l = targets.trailing_zeros() as usize;
            targets &= targets - 1;
            let mut cheapest = p.t(from, m.locs[l]);
            let mut others = pend & !(1 << l);
            while others != 0 {
                let x = others.trailing_zeros() as usize;
                others &= others - 1;
                cheapest = cheapest.min(local_t[x * k + l]);
            }
            total += cheapest;
        }
        total
    };
    let mut nodes = vec![Node {
        done: 0,
        delivered: 0,
        cur: AT_START,
        load: m.start_load,
        parent: usize::MAX,
        visit: None,
    }];
    let mut best: FastMap<(u32, u64, u8), f64> = FastMap::default();
    best.insert((0, 0, AT_START), 0.0);
    let mut heap = BinaryHeap::new();
    heap.push(Entry {
        f: h(0, 0, p.start),
        g: 0.0,
        node: 0,
    });
    let mut moves = Vec::with_capacity(2 * m.locs.len());

    while let Some(Entry { g, node, .. }) = heap.pop() {
        let (done, delivered, cur, load) = (
            nodes[node].done,
            nodes[node].delivered,
            nodes[node].cur,
            nodes[node].load,
        );
        if best
            .get(&(done, delivered, cur))
            .is_some_and(|&b| g > b + EPS)
        {
            continue;
        }
        if delivered == m.all_delivered {
            let mut visits = Vec::new();
            let mut n = node;
            while let Some(v) = nodes[n].visit {
                visits.push(v);
                n = nodes[n].parent;
            }
            visits.reverse();
            return Exact::Found(visits);
        }
        if nodes.len() > STATE_CAP {
            return Exact::TooBig;
        }
        let from = if cur == AT_START {
            p.start
        } else {
            m.locs[cur as usize]
        };
        m.moves(p, done, delivered, load, &mut moves);
        for &(l, drop_only, nd, ndel, nload) in &moves {
            if l as u8 == cur {
                continue;
            }
            let ng = g + p.t(from, m.locs[l]) + p.stop_minutes;
            let key = (nd, ndel, l as u8);
            if best.get(&key).is_some_and(|&b| b <= ng + EPS) {
                continue;
            }
            best.insert(key, ng);
            nodes.push(Node {
                done: nd,
                delivered: ndel,
                cur: l as u8,
                load: nload,
                parent: node,
                visit: Some(Visit {
                    loc: m.locs[l],
                    drop_only,
                }),
            });
            heap.push(Entry {
                f: ng + h(nd, ndel, m.locs[l]),
                g: ng,
                node: nodes.len() - 1,
            });
        }
    }
    Exact::Infeasible
}

/// Greedy nearest useful visit, then improvement by moving or removing visits.
fn heuristic(p: &Problem, jobs: &[usize], m: &Model) -> Option<Vec<Visit>> {
    let (mut done, mut delivered, mut load, mut cur) = (0u32, 0u64, m.start_load, p.start);
    let mut visits = Vec::new();
    let mut moves = Vec::new();
    while delivered != m.all_delivered {
        m.moves(p, done, delivered, load, &mut moves);
        let next = *moves
            .iter()
            .filter(|mv| m.locs[mv.0] != cur)
            .min_by(|a, b| {
                p.t(cur, m.locs[a.0])
                    .total_cmp(&p.t(cur, m.locs[b.0]))
                    .then(a.1.cmp(&b.1))
            })?;
        visits.push(Visit {
            loc: m.locs[next.0],
            drop_only: next.1,
        });
        (done, delivered, load, cur) = (next.2, next.3, next.4, m.locs[next.0]);
    }
    improve(p, jobs, visits)
}

fn improve(p: &Problem, jobs: &[usize], mut visits: Vec<Visit>) -> Option<Vec<Visit>> {
    let mut best = simulate(p, jobs, &visits)?.total();
    for _ in 0..60 {
        let mut improved = false;
        let n = visits.len();
        'search: for i in 0..n {
            let mut without = visits.clone();
            let v = without.remove(i);
            if let Some(r) = simulate(p, jobs, &without)
                && r.total() < best - EPS
            {
                (visits, best, improved) = (without, r.total(), true);
                break 'search;
            }
            for j in 0..n {
                if j == i {
                    continue;
                }
                let mut moved = without.clone();
                moved.insert(j.min(moved.len()), v);
                if let Some(r) = simulate(p, jobs, &moved)
                    && r.total() < best - EPS
                {
                    (visits, best, improved) = (moved, r.total(), true);
                    break 'search;
                }
            }
        }
        if !improved {
            break;
        }
    }
    Some(visits)
}

/// Shortest route completing every order of `jobs`, or None when no route keeps
/// cargo under the hard maximum.
pub fn best_route(p: &Problem, jobs: &[usize]) -> Option<Route> {
    let m = Model::new(p, jobs)?;
    match exact(p, &m) {
        Exact::Found(v) => simulate(p, jobs, &v).map(|mut r| {
            r.exact = true;
            r
        }),
        Exact::Infeasible => None,
        Exact::TooBig => heuristic(p, jobs, &m).and_then(|v| simulate(p, jobs, &v)),
    }
}
