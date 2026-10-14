use std::collections::{HashMap, VecDeque};

use crate::lookup::NameIndex;
use crate::{
    Distance, DistanceError, GameUuid, Leg, Location, LocationId, LocationKind, LookupOptions,
    Match, Route,
};

/// Provenance of the loaded data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataMeta {
    /// Human readable source (e.g. `scunpacked-data`).
    pub source: String,
    /// Game version the data was extracted from (e.g. `4.10.2-LIVE.12881860`), if known.
    pub game_version: Option<String>,
    /// Source revision (git sha for scunpacked-data).
    pub revision: Option<String>,
    /// When the data was downloaded, seconds since the Unix epoch.
    pub fetched_at_unix: Option<u64>,
    /// True if the data came from the local cache because the source was unreachable.
    pub from_cache: bool,
}

/// A bidirectional jump tunnel between two jump points in different systems.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpLink {
    pub a: LocationId,
    pub b: LocationId,
    pub fuel_cost: Option<f64>,
}

/// The loaded universe: an arena of [`Location`]s plus indices.
///
/// Built by [`crate::UniverseBuilder`]; immutable afterwards.
#[derive(Debug, Clone)]
pub struct Universe {
    pub(crate) locations: Vec<Location>,
    pub(crate) systems: Vec<LocationId>,
    pub(crate) by_uuid: HashMap<GameUuid, LocationId>,
    pub(crate) names: NameIndex,
    pub(crate) jump_links: Vec<JumpLink>,
    pub(crate) meta: DataMeta,
}

impl Universe {
    pub fn meta(&self) -> &DataMeta {
        &self.meta
    }

    pub fn set_meta(&mut self, meta: DataMeta) {
        self.meta = meta;
    }

    pub fn len(&self) -> usize {
        self.locations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }

    /// All locations, indexed by [`LocationId::index`].
    pub fn locations(&self) -> &[Location] {
        &self.locations
    }

    /// Panics if `id` does not come from this universe.
    pub fn get(&self, id: LocationId) -> &Location {
        &self.locations[id.index()]
    }

    pub fn try_get(&self, id: LocationId) -> Option<&Location> {
        self.locations.get(id.index())
    }

    pub fn by_uuid(&self, uuid: GameUuid) -> Option<&Location> {
        self.by_uuid.get(&uuid).map(|id| self.get(*id))
    }

    /// Roots of the tree, one per star system.
    pub fn systems(&self) -> impl Iterator<Item = &Location> {
        self.systems.iter().map(|id| self.get(*id))
    }

    /// Finds a system root by name, case-insensitively (`"stanton"`).
    pub fn system(&self, name: &str) -> Option<&Location> {
        self.systems().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn children(&self, id: LocationId) -> impl Iterator<Item = &Location> {
        self.get(id).children.iter().map(|c| self.get(*c))
    }

    pub fn parent(&self, id: LocationId) -> Option<&Location> {
        self.get(id).parent.map(|p| self.get(p))
    }

    /// Ancestors from the direct parent up to the system root.
    pub fn ancestors(&self, id: LocationId) -> impl Iterator<Item = &Location> {
        std::iter::successors(self.parent(id), |l| self.parent(l.id))
    }

    /// True if `id` is `ancestor` or lies in its subtree.
    pub fn is_within(&self, id: LocationId, ancestor: LocationId) -> bool {
        id == ancestor || self.ancestors(id).any(|a| a.id == ancestor)
    }

    /// Depth-first iterator over a subtree, `root` included.
    pub fn descendants(&self, root: LocationId) -> impl Iterator<Item = &Location> {
        let mut stack = vec![root];
        std::iter::from_fn(move || {
            let id = stack.pop()?;
            let loc = self.get(id);
            stack.extend(loc.children.iter().rev());
            Some(loc)
        })
    }

    /// Closest planet or moon the location belongs to (itself included).
    ///
    /// `None` for locations in open space (L-point stations, gateways, the star...).
    pub fn nearest_body(&self, id: LocationId) -> Option<&Location> {
        let loc = self.get(id);
        if loc.kind.is_body() {
            return Some(loc);
        }
        self.ancestors(id).find(|a| a.kind.is_body())
    }

    /// `"Stanton › Hurston › Lorville"` (system root first, star omitted).
    pub fn path_string(&self, id: LocationId) -> String {
        let mut parts: Vec<&str> = self
            .ancestors(id)
            .filter(|a| a.kind != LocationKind::Star)
            .map(|a| a.name.as_str())
            .collect();
        parts.reverse();
        parts.push(&self.get(id).name);
        parts.join(" › ")
    }

    pub fn iter_kind(&self, kind: LocationKind) -> impl Iterator<Item = &Location> {
        self.locations.iter().filter(move |l| l.kind == kind)
    }

    /// Locations with a commodity trading terminal.
    pub fn trading_locations(&self) -> impl Iterator<Item = &Location> {
        self.locations.iter().filter(|l| l.has_commodity_trading())
    }

    pub fn jump_links(&self) -> &[JumpLink] {
        &self.jump_links
    }

    // ---- lookup -------------------------------------------------------------------------

    /// Locations whose normalised name equals the normalised `name`.
    pub fn find_exact(&self, name: &str) -> &[LocationId] {
        self.names.exact(name)
    }

    /// Ranked fuzzy matches, best first. See [`crate::normalize`] for the rules.
    pub fn find_fuzzy(&self, query: &str, opts: LookupOptions) -> Vec<Match> {
        self.names.fuzzy(query, opts, |id| match opts.scope {
            Some(scope) => self.is_within(id, scope),
            None => true,
        })
    }

    /// Best single match for `query`, if any passes `opts.min_score`.
    pub fn resolve(&self, query: &str, opts: LookupOptions) -> Option<&Location> {
        self.find_fuzzy(query, LookupOptions { limit: 1, ..opts })
            .first()
            .map(|m| self.get(m.id))
    }

    // ---- geometry -----------------------------------------------------------------------

    /// Straight-line distance between two locations of the same system.
    ///
    /// Surface locations use an unrotated offset from their body's centre, so distances
    /// involving them are approximate (accurate to roughly the body's radius).
    pub fn distance(&self, a: LocationId, b: LocationId) -> Result<Distance, DistanceError> {
        let (la, lb) = (self.get(a), self.get(b));
        if la.system != lb.system {
            return Err(DistanceError::DifferentSystems(a, b));
        }
        let pa = la.position.vec().ok_or(DistanceError::NoPosition(a))?;
        let pb = lb.position.vec().ok_or(DistanceError::NoPosition(b))?;
        Ok(Distance(pa.distance_to(pb)))
    }

    /// Route between two locations, crossing systems through jump points if needed.
    ///
    /// Picks the route with the fewest jumps. Jump legs add no distance.
    pub fn route(&self, a: LocationId, b: LocationId) -> Result<Route, DistanceError> {
        let (sa, sb) = (self.get(a).system, self.get(b).system);
        let hops = self
            .system_path(sa, sb)
            .ok_or(DistanceError::NoJumpRoute(a, b))?;

        let mut legs = Vec::with_capacity(hops.len() * 2 + 1);
        let mut total = Distance::ZERO;
        let mut cursor = a;
        for (entry, exit, fuel_cost) in hops {
            let d = self.distance(cursor, entry)?;
            total = total + d;
            legs.push(Leg::Travel {
                from: cursor,
                to: entry,
                distance: d,
            });
            legs.push(Leg::Jump {
                from: entry,
                to: exit,
                fuel_cost,
            });
            cursor = exit;
        }
        let d = self.distance(cursor, b)?;
        total = total + d;
        legs.push(Leg::Travel {
            from: cursor,
            to: b,
            distance: d,
        });
        Ok(Route { legs, total })
    }

    /// BFS over systems. Returns the jumps to take as `(entry jump point, exit jump point)`.
    fn system_path(
        &self,
        from: LocationId,
        to: LocationId,
    ) -> Option<Vec<(LocationId, LocationId, Option<f64>)>> {
        if from == to {
            return Some(Vec::new());
        }
        // For each reached system: the jump (entry, exit, fuel) used to reach it.
        let mut came_from: HashMap<LocationId, (LocationId, LocationId, Option<f64>)> =
            HashMap::new();
        let mut queue = VecDeque::from([from]);
        while let Some(sys) = queue.pop_front() {
            for link in &self.jump_links {
                let (sa, sb) = (self.get(link.a).system, self.get(link.b).system);
                let (entry, exit, next) = if sa == sys {
                    (link.a, link.b, sb)
                } else if sb == sys {
                    (link.b, link.a, sa)
                } else {
                    continue;
                };
                if next == from || came_from.contains_key(&next) {
                    continue;
                }
                came_from.insert(next, (entry, exit, link.fuel_cost));
                if next == to {
                    let mut path = Vec::new();
                    let mut cur = to;
                    while cur != from {
                        let hop = came_from[&cur];
                        path.push(hop);
                        cur = self.get(hop.0).system;
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(next);
            }
        }
        None
    }
}
