use std::collections::{HashMap, VecDeque};
use std::fmt;

use crate::lookup::NameIndex;
use crate::{
    Amenity, DataMeta, GameUuid, JumpLink, Location, LocationId, LocationKind, Position, Universe,
    Vec3,
};

/// A location as handed over by a data source, before the tree is resolved.
#[derive(Debug, Clone)]
pub struct NodeSpec {
    pub uuid: GameUuid,
    pub parent: Option<GameUuid>,
    /// System code (`"stanton"`) if the source knows it. Only needed on tree roots (stars);
    /// everything else inherits the system of its root.
    pub system: Option<String>,
    pub name: String,
    pub kind: LocationKind,
    pub raw_type: String,
    pub position: Option<Vec3>,
    pub radius_m: Option<f64>,
    pub qt_valid: bool,
    pub hidden: bool,
    pub hierarchy_tag: Option<String>,
    pub jurisdiction: Option<String>,
    pub amenities: Vec<Amenity>,
}

impl NodeSpec {
    /// Minimal spec; set the remaining public fields as needed.
    pub fn new(uuid: GameUuid, name: impl Into<String>, kind: LocationKind) -> Self {
        Self {
            uuid,
            parent: None,
            system: None,
            name: name.into(),
            kind,
            raw_type: String::new(),
            position: None,
            radius_m: None,
            qt_valid: false,
            hidden: false,
            hierarchy_tag: None,
            jurisdiction: None,
            amenities: Vec::new(),
        }
    }
}

/// A jump tunnel between two jump point objects, by game uuid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpSpec {
    pub a: GameUuid,
    pub b: GameUuid,
    pub fuel_cost: Option<f64>,
}

/// Non-fatal data problems found while building. The universe is still usable.
#[derive(Debug, Clone, PartialEq)]
pub enum BuildWarning {
    /// A second node with the same uuid was ignored.
    DuplicateUuid { uuid: GameUuid, name: String },
    /// The parent uuid is unknown; the node was attached to its system root.
    DanglingParent {
        uuid: GameUuid,
        name: String,
        parent: GameUuid,
    },
    /// The node was part of a parent cycle; its parent link was cut.
    Cycle { uuid: GameUuid, name: String },
    /// No system could be determined; the node (and its subtree) was dropped.
    NoSystem { uuid: GameUuid, name: String },
    /// A jump link references an unknown node and was dropped.
    JumpEndpointMissing { uuid: GameUuid },
}

impl fmt::Display for BuildWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateUuid { uuid, name } => write!(f, "duplicate uuid {uuid} ({name})"),
            Self::DanglingParent { uuid, name, parent } => {
                write!(f, "{name} ({uuid}) has unknown parent {parent}")
            }
            Self::Cycle { uuid, name } => write!(f, "{name} ({uuid}) is in a parent cycle"),
            Self::NoSystem { uuid, name } => write!(f, "{name} ({uuid}) has no system, dropped"),
            Self::JumpEndpointMissing { uuid } => write!(f, "jump endpoint {uuid} unknown"),
        }
    }
}

/// Assembles a [`Universe`] from flat [`NodeSpec`]s.
///
/// - inserts one synthetic [`LocationKind::System`] root per system code,
/// - resolves parents, cutting cycles and re-attaching dangling nodes,
/// - inherits missing positions from the closest positioned ancestor,
/// - sorts children by kind then name for a stable order,
/// - builds the uuid and name indices.
#[derive(Debug, Default)]
pub struct UniverseBuilder {
    systems: Vec<(String, String)>,
    nodes: Vec<NodeSpec>,
    seen: HashMap<GameUuid, usize>,
    jumps: Vec<JumpSpec>,
    warnings: Vec<BuildWarning>,
}

impl UniverseBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a system with a display name. Systems referenced by nodes but never
    /// declared get a capitalised version of their code.
    pub fn add_system(&mut self, code: impl Into<String>, name: impl Into<String>) -> &mut Self {
        let code = code.into().to_lowercase();
        if !self.systems.iter().any(|(c, _)| *c == code) {
            self.systems.push((code, name.into()));
        }
        self
    }

    /// Adds a node. The first node wins on duplicate uuids.
    pub fn add_node(&mut self, spec: NodeSpec) -> &mut Self {
        if self.seen.contains_key(&spec.uuid) {
            self.warnings.push(BuildWarning::DuplicateUuid {
                uuid: spec.uuid,
                name: spec.name,
            });
            return self;
        }
        self.seen.insert(spec.uuid, self.nodes.len());
        self.nodes.push(spec);
        self
    }

    pub fn contains(&self, uuid: GameUuid) -> bool {
        self.seen.contains_key(&uuid)
    }

    pub fn add_jump(&mut self, jump: JumpSpec) -> &mut Self {
        self.jumps.push(jump);
        self
    }

    pub fn build(mut self, meta: DataMeta) -> (Universe, Vec<BuildWarning>) {
        let n = self.nodes.len();

        // 1. Resolve parent indices.
        let mut parent: Vec<Option<usize>> = Vec::with_capacity(n);
        for spec in &self.nodes {
            let p = match spec.parent {
                None => None,
                Some(pu) => match self.seen.get(&pu) {
                    Some(&pi) => Some(pi),
                    None => {
                        self.warnings.push(BuildWarning::DanglingParent {
                            uuid: spec.uuid,
                            name: spec.name.clone(),
                            parent: pu,
                        });
                        None
                    }
                },
            };
            parent.push(p);
        }

        // 2. Cut cycles (iterative colouring: 0 = new, 1 = on current path, 2 = done).
        let mut state = vec![0u8; n];
        for start in 0..n {
            let mut path = Vec::new();
            let mut cur = Some(start);
            while let Some(i) = cur {
                match state[i] {
                    2 => break,
                    1 => {
                        self.warnings.push(BuildWarning::Cycle {
                            uuid: self.nodes[i].uuid,
                            name: self.nodes[i].name.clone(),
                        });
                        parent[i] = None;
                        break;
                    }
                    _ => {
                        state[i] = 1;
                        path.push(i);
                        cur = parent[i];
                    }
                }
            }
            for i in path {
                state[i] = 2;
            }
        }

        // 3. System of each node: the root's code, else the code closest to the root.
        let mut system_code: Vec<Option<String>> = vec![None; n];
        for i in 0..n {
            let mut chain = vec![i];
            let mut cur = parent[i];
            while let Some(p) = cur {
                chain.push(p);
                cur = parent[p];
            }
            let code = chain
                .iter()
                .rev()
                .find_map(|&j| self.nodes[j].system.as_deref())
                .map(str::to_lowercase);
            system_code[i] = code;
        }
        for (i, spec) in self.nodes.iter().enumerate() {
            if system_code[i].is_none() {
                self.warnings.push(BuildWarning::NoSystem {
                    uuid: spec.uuid,
                    name: spec.name.clone(),
                });
            }
        }
        for code in system_code.iter().flatten() {
            if !self.systems.iter().any(|(c, _)| c == code) {
                self.systems.push((code.clone(), capitalise(code)));
            }
        }

        // 4. Arena: systems first, then kept nodes in insertion order.
        let mut locations: Vec<Location> = Vec::with_capacity(self.systems.len() + n);
        let mut system_ids: HashMap<String, LocationId> = HashMap::new();
        for (code, name) in &self.systems {
            let id = LocationId::from_index(locations.len());
            system_ids.insert(code.clone(), id);
            locations.push(Location {
                id,
                uuid: None,
                name: name.clone(),
                kind: LocationKind::System,
                raw_type: "System".to_owned(),
                parent: None,
                children: Vec::new(),
                system: id,
                position: Position::Unknown,
                radius_m: None,
                qt_valid: false,
                hidden: false,
                hierarchy_tag: None,
                jurisdiction: None,
                amenities: Vec::new(),
            });
        }
        let mut node_id: Vec<Option<LocationId>> = vec![None; n];
        let nodes = std::mem::take(&mut self.nodes);
        let mut own_pos: Vec<Option<Vec3>> = Vec::new();
        for (i, spec) in nodes.into_iter().enumerate() {
            let Some(sys) = system_code[i].as_ref().map(|c| system_ids[c]) else {
                continue;
            };
            let id = LocationId::from_index(locations.len());
            node_id[i] = Some(id);
            own_pos.push(spec.position);
            locations.push(Location {
                id,
                uuid: Some(spec.uuid),
                name: spec.name,
                kind: spec.kind,
                raw_type: spec.raw_type,
                parent: None,
                children: Vec::new(),
                system: sys,
                position: Position::Unknown,
                radius_m: spec.radius_m,
                qt_valid: spec.qt_valid,
                hidden: spec.hidden,
                hierarchy_tag: spec.hierarchy_tag,
                jurisdiction: spec.jurisdiction,
                amenities: spec.amenities,
            });
        }

        // 5. Parent / children links.
        for i in 0..n {
            let Some(id) = node_id[i] else { continue };
            let p = parent[i]
                .and_then(|pi| node_id[pi])
                .unwrap_or(locations[id.index()].system);
            locations[id.index()].parent = Some(p);
            locations[p.index()].children.push(id);
        }
        let order: Vec<(u8, String)> = locations
            .iter()
            .map(|l| (l.kind.rank(), l.name.to_lowercase()))
            .collect();
        for loc in &mut locations {
            loc.children
                .sort_by(|a, b| order[a.index()].cmp(&order[b.index()]));
        }

        // 6. Positions, top-down so parents are resolved before children.
        let first_node = self.systems.len();
        let roots: Vec<LocationId> = system_ids.values().copied().collect();
        let mut queue: VecDeque<LocationId> = roots.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            let idx = id.index();
            if idx >= first_node {
                let own = own_pos[idx - first_node];
                let inherited = locations[idx].parent.map(|p| {
                    let pl = &locations[p.index()];
                    match pl.position {
                        Position::Exact(pos) => Position::Inherited { from: p, pos },
                        other => other,
                    }
                });
                locations[idx].position = match own {
                    Some(v) => Position::Exact(v),
                    None => inherited.unwrap_or(Position::Unknown),
                };
            }
            queue.extend(locations[idx].children.iter().copied());
        }

        // 7. Indices.
        let mut by_uuid = HashMap::with_capacity(locations.len());
        let mut names = NameIndex::default();
        for loc in &locations {
            if let Some(u) = loc.uuid {
                by_uuid.insert(u, loc.id);
            }
            names.insert(&loc.name, loc.id);
        }

        let mut jump_links = Vec::new();
        for j in &self.jumps {
            match (by_uuid.get(&j.a), by_uuid.get(&j.b)) {
                (Some(&a), Some(&b)) => jump_links.push(JumpLink {
                    a,
                    b,
                    fuel_cost: j.fuel_cost,
                }),
                (None, _) => self
                    .warnings
                    .push(BuildWarning::JumpEndpointMissing { uuid: j.a }),
                (_, None) => self
                    .warnings
                    .push(BuildWarning::JumpEndpointMissing { uuid: j.b }),
            }
        }

        let mut systems: Vec<LocationId> = roots;
        systems.sort();
        let universe = Universe {
            locations,
            systems,
            by_uuid,
            names,
            jump_links,
            meta,
        };
        (universe, self.warnings)
    }
}

fn capitalise(code: &str) -> String {
    let mut chars = code.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
