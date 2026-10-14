//! What the player should look at: a [`Mark`] per field (red when it blocks planning,
//! amber when it is a doubtful read), problems per contract, "Use X for Y" fixes for unknown
//! places, the fields left to check in reading order, and duplicate contracts.
//!
//! A doubtful read is an inferred place (`?`), a name snapping changed, an ambiguous place,
//! a value read from an OCR line the engine was not sure of, or a reward taken from the
//! offer list (rounded) because the reward box was garbled. Typing a value, or pressing
//! Enter on it, confirms it.

use sc_ocr::Region;
use sc_universe::{Universe, normalize};

use super::fields::{self, Field};
use super::places::{PlaceTree, Places};
use super::{Entry, ReadKind, sources};

/// Engine confidence below which a value counts as a doubtful read. On the samples every
/// line PaddleOCR read was right and the lowest confidence was 0.859 (reward lines sit
/// around 0.90 because of the currency glyph), so only clearly shaky reads are flagged.
pub const LOW_CONFIDENCE: f32 = 0.85;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    None,
    /// Doubtful read: check it.
    Unsure,
    /// Missing or unknown: blocks planning.
    Bad,
}

/// What reviewing needs besides the entry.
#[derive(Debug, Clone, Copy)]
pub struct Lookup<'a> {
    pub universe: &'a Universe,
    pub places: &'a Places,
}

/// Replace `written` with `suggestion` everywhere in the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub written: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Review {
    /// Block planning (red).
    pub problems: Vec<String>,
    /// Doubtful reads, by field (`reward`, `Lorville pickup`), for one "Check:" line.
    pub unsure: Vec<String>,
    /// Other things worth a look (amber).
    pub notes: Vec<String>,
    /// Cargo types snapping changed: `Gold cargo, read "Gald"` (amber).
    pub cargo: Vec<String>,
    pub fixes: Vec<Fix>,
}

/// Whether the player confirmed this field's current value (typed it or pressed Enter).
pub fn confirmed(e: &Entry, f: Field) -> bool {
    let text = fields::text(&e.contract, f);
    e.confirmed
        .iter()
        .any(|(label, value)| label == f.label() && *value == text)
}

pub fn mark(e: &Entry, f: Field, lk: Lookup) -> Mark {
    let c = &e.contract;
    let flow = f.line().and_then(|k| c.flows.get(k));
    let text = fields::text(c, f);
    let tree = || lk.places.tree(lk.universe, &text);
    let bad = match f {
        Field::Reward => c.reward.is_none(),
        Field::Scu(_) => flow.is_some_and(|f| match f.dropoffs.as_slice() {
            [] => true,
            [d] => d.scu == 0,
            _ => false,
        }),
        Field::DropScu(_, j) => flow
            .and_then(|f| f.dropoffs.get(j))
            .is_some_and(|d| d.scu == 0),
        // Every line needs a pickup and a drop-off: the empty first field is red.
        Field::Pickup(_, j) => match flow {
            Some(f) if f.pickups.is_empty() => j == 0,
            Some(_) => tree() == PlaceTree::Unknown,
            None => false,
        },
        Field::Dropoff(_, j) => match flow {
            Some(f) if f.dropoffs.is_empty() => j == 0,
            Some(_) => tree() == PlaceTree::Unknown,
            None => false,
        },
        _ => false,
    };
    if bad {
        return Mark::Bad;
    }
    // The cargo type never needs the player: it doesn't change the route.
    if matches!(f, Field::Commodity(_)) || text.is_empty() || confirmed(e, f) {
        return Mark::None;
    }
    let inferred = match f {
        Field::Pickup(_, j) => flow
            .and_then(|f| f.pickups.get(j))
            .is_some_and(|p| p.uncertain),
        Field::Dropoff(_, j) => flow
            .and_then(|f| f.dropoffs.get(j))
            .is_some_and(|d| d.place.uncertain),
        _ => false,
    };
    let snapped = f.is_place()
        && e.reads
            .iter()
            .any(|r| r.kind == ReadKind::Place && r.known == text);
    let ambiguous = f.is_place() && matches!(tree(), PlaceTree::Ambiguous(_));
    let source = sources::locate(e, f);
    let low = source
        .and_then(|s| s.conf)
        .is_some_and(|c| c < LOW_CONFIDENCE);
    let rounded = matches!(f, Field::Reward) && source.is_some_and(|s| s.region == Region::Offer);
    if inferred || snapped || ambiguous || low || rounded {
        Mark::Unsure
    } else {
        Mark::None
    }
}

pub fn review(e: &Entry, lk: Lookup) -> Review {
    let c = &e.contract;
    let mut r = Review::default();
    if c.reward.is_none() {
        r.problems.push("Reward not read".to_owned());
    }
    if c.flows.is_empty() {
        r.problems.push("No cargo lines".to_owned());
    }
    for (k, f) in c.flows.iter().enumerate() {
        if f.commodity.trim().is_empty() {
            r.problems.push(format!("Line {}: no cargo type", k + 1));
        }
        if f.pickups.len() > 1 && f.dropoffs.len() > 1 {
            r.problems.push(format!(
                "Line {}: several pickups and several drop-offs (a line has one pickup or one \
                 drop-off)",
                k + 1
            ));
        }
        let places = f.pickups.iter().chain(f.dropoffs.iter().map(|d| &d.place));
        for p in places {
            if let Some(suggestion) = lk.places.fix(lk.universe, &p.name) {
                let fix = Fix {
                    written: p.name.clone(),
                    suggestion,
                };
                if !r.fixes.contains(&fix) {
                    r.fixes.push(fix);
                }
            }
        }
    }
    let rounded = !confirmed(e, Field::Reward)
        && sources::locate(e, Field::Reward).is_some_and(|s| s.region == Region::Offer);
    if rounded {
        r.notes.push(
            "reward read from the offer list, rounded (the reward box was unreadable)".to_owned(),
        );
    }
    for f in fields::all(c) {
        if rounded && matches!(f, Field::Reward) {
            continue;
        }
        if mark(e, f, lk) == Mark::Unsure {
            let what = if f.is_place() {
                format!("{} {}", fields::text(c, f), f.label())
            } else {
                f.label().to_owned()
            };
            if !r.unsure.contains(&what) {
                r.unsure.push(what);
            }
        }
    }
    for read in e.reads.iter().filter(|r| r.kind == ReadKind::Commodity) {
        let shown = c.flows.iter().any(|f| f.commodity == read.known);
        let ok = c
            .flows
            .iter()
            .enumerate()
            .any(|(k, f)| f.commodity == read.known && confirmed(e, Field::Commodity(k)));
        if shown && !ok {
            r.cargo.push(format!(
                "{} cargo, read \u{201c}{}\u{201d}",
                read.known, read.read
            ));
        }
    }
    if e.stale {
        r.notes.push(
            "A screenshot was merged into this edited contract: re-parse to include it \
             (discards edits)"
                .to_owned(),
        );
    }
    r
}

/// Where a contract stands, for its badge on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing to check.
    Ready,
    /// This many doubtful reads (amber), nothing blocking.
    ToCheck(usize),
    /// This many problems or missing/unknown values (red): planning skips or misplans it.
    Blocked(usize),
}

pub fn status(e: &Entry, lk: Lookup) -> Status {
    let bad = fields::all(&e.contract)
        .into_iter()
        .filter(|&f| mark(e, f, lk) == Mark::Bad)
        .count()
        + review(e, lk).problems.len();
    let check = unsure(e, lk).len() + usize::from(e.stale);
    match (bad, check) {
        (0, 0) => Status::Ready,
        (0, n) => Status::ToCheck(n),
        (n, _) => Status::Blocked(n),
    }
}

/// The doubtful reads of a contract, in reading order: amber fields and cargo types that
/// snapping changed. Confirming them all ("Done") leaves nothing to check but red problems
/// and a stale merge.
pub fn unsure(e: &Entry, lk: Lookup) -> Vec<Field> {
    fields::all(&e.contract)
        .into_iter()
        .filter(|&f| match f {
            Field::Commodity(k) => snapped_cargo(e, k),
            _ => mark(e, f, lk) == Mark::Unsure,
        })
        .collect()
}

/// Whether the cargo type of line `k` was changed by snapping and not confirmed since.
fn snapped_cargo(e: &Entry, k: usize) -> bool {
    let Some(flow) = e.contract.flows.get(k) else {
        return false;
    };
    e.reads
        .iter()
        .any(|r| r.kind == ReadKind::Commodity && r.known == flow.commodity)
        && !confirmed(e, Field::Commodity(k))
}

/// Every field still needing the player, in board order then reading order.
pub fn to_check(entries: &[Entry], lk: Lookup) -> Vec<(u64, Field)> {
    entries
        .iter()
        .flat_map(|e| {
            fields::all(&e.contract)
                .into_iter()
                .filter(move |&f| mark(e, f, lk) != Mark::None)
                .map(move |f| (e.id, f))
        })
        .collect()
}

/// Pairs of entries that look like the same contract (same title and reward).
pub fn duplicates(entries: &[Entry]) -> Vec<(u64, u64)> {
    let key = |e: &Entry| -> Option<(String, u64)> {
        Some((normalize(e.contract.title.as_deref()?), e.contract.reward?))
    };
    let mut out = Vec::new();
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[i + 1..] {
            if key(a).is_some() && key(a) == key(b) {
                out.push((a.id, b.id));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sc_contracts::{Contract, DropOff, Flow, Place};
    use sc_universe::{DataMeta, UniverseBuilder};

    use super::*;
    use crate::contracts::{LineBox, Read, ShotBoxes, ShotText};

    /// Universe with no places: every non-empty place name is unknown.
    fn empty() -> Universe {
        UniverseBuilder::new().build(DataMeta::default()).0
    }

    fn entry(pickup: Place, drop: &str, scu: u32) -> Entry {
        Entry {
            id: 7,
            contract: Contract {
                title: Some("Haul".into()),
                reward: Some(1000),
                flows: vec![Flow {
                    commodity: "Gold".into(),
                    pickups: vec![pickup],
                    dropoffs: vec![DropOff {
                        place: Place::new(drop),
                        scu,
                    }],
                    ..Flow::default()
                }],
                ..Contract::default()
            },
            ..Entry::default()
        }
    }

    #[test]
    fn marks() {
        let u = empty();
        let places = Places::new(&u, &BTreeMap::new());
        let lk = Lookup {
            universe: &u,
            places: &places,
        };
        let e = entry(Place::uncertain("A"), "", 0);
        assert_eq!(mark(&e, Field::Title, lk), Mark::None);
        assert_eq!(mark(&e, Field::Scu(0), lk), Mark::Bad);
        assert_eq!(
            mark(&e, Field::Pickup(0, 0), lk),
            Mark::Bad,
            "unknown place"
        );
        assert_eq!(
            mark(&e, Field::Dropoff(0, 0), lk),
            Mark::None,
            "empty, not missing"
        );
        assert_eq!(mark(&e, Field::Commodity(0), lk), Mark::None);

        let mut e = entry(Place::new("A"), "B", 4);
        e.contract.flows[0].dropoffs.clear();
        assert_eq!(
            mark(&e, Field::Dropoff(0, 0), lk),
            Mark::Bad,
            "a line needs a drop-off"
        );
        assert_eq!(mark(&e, Field::Scu(0), lk), Mark::Bad);

        let mut e = entry(Place::new("A"), "B", 4);
        e.contract.reward = None;
        assert_eq!(mark(&e, Field::Reward, lk), Mark::Bad);
        let r = review(&e, lk);
        assert_eq!(r.problems, ["Reward not read"]);
    }

    #[test]
    fn doubtful_reads_until_confirmed() {
        // "Area18" known, via a one-location universe built from the fixture would be
        // heavier; an alias map teaches the empty universe nothing, so use the low
        // confidence path and the inferred flag on a known-free field instead.
        let u = empty();
        let places = Places::new(&u, &BTreeMap::new());
        let lk = Lookup {
            universe: &u,
            places: &places,
        };
        let mut e = entry(Place::new("A"), "B", 4);
        e.shots = vec![ShotText {
            title: "Haul".into(),
            boxes: ShotBoxes {
                title: vec![Some(LineBox {
                    x: 0,
                    y: 0,
                    w: 10,
                    h: 10,
                    conf: Some(0.5),
                })],
                ..ShotBoxes::default()
            },
            ..ShotText::default()
        }];
        assert_eq!(mark(&e, Field::Title, lk), Mark::Unsure);
        assert_eq!(review(&e, lk).unsure, ["title"]);
        assert_eq!(
            to_check(std::slice::from_ref(&e), lk).first(),
            Some(&(7, Field::Title))
        );
        e.confirmed.push(("title".into(), "Haul".into()));
        assert_eq!(mark(&e, Field::Title, lk), Mark::None);
        // A different value is not confirmed.
        e.contract.title = Some("Haul 2".into());
        assert_eq!(mark(&e, Field::Title, lk), Mark::Unsure);

        e.reads.push(Read {
            kind: ReadKind::Commodity,
            read: "Gald".into(),
            known: "Gold".into(),
        });
        assert_eq!(
            review(&e, lk).cargo,
            ["Gold cargo, read \u{201c}Gald\u{201d}"]
        );
    }

    #[test]
    fn reward_from_the_offer_list_is_doubtful() {
        let u = empty();
        let places = Places::new(&u, &BTreeMap::new());
        let lk = Lookup {
            universe: &u,
            places: &places,
        };
        let line = |y| {
            Some(LineBox {
                x: 0,
                y,
                w: 10,
                h: 10,
                conf: Some(0.99),
            })
        };
        let mut e = entry(Place::new("A"), "B", 4);
        e.contract.reward = Some(55_000);
        e.shots = vec![ShotText {
            reward: "1h 56m\nCovalex Independent Contrect9Ea".into(),
            offer: "JUNIOR | HAUL\n55k\n1h 56m\nCOVALEX INDEPENDENT CONTRACTORS".into(),
            boxes: ShotBoxes {
                reward: vec![line(0), line(10)],
                offer: vec![line(100), line(110), line(120), line(130)],
                ..ShotBoxes::default()
            },
            ..ShotText::default()
        }];
        let src = sources::locate(&e, Field::Reward);
        assert_eq!(src.map(|s| (s.region, s.y)), Some((Region::Offer, 110)));
        assert_eq!(mark(&e, Field::Reward, lk), Mark::Unsure);
        let r = review(&e, lk);
        assert!(r.unsure.is_empty(), "{:?}", r.unsure);
        assert_eq!(
            r.notes,
            ["reward read from the offer list, rounded (the reward box was unreadable)"]
        );
        e.confirmed
            .push(("reward".into(), fields::text(&e.contract, Field::Reward)));
        assert_eq!(mark(&e, Field::Reward, lk), Mark::None);
        assert!(review(&e, lk).notes.is_empty());

        // An exact reward read in the box points there, even when the list agrees.
        e.confirmed.clear();
        e.contract.reward = Some(55_250);
        e.shots[0].reward = "¤ 55,250\n1h 56m".into();
        let src = sources::locate(&e, Field::Reward);
        assert_eq!(src.map(|s| (s.region, s.y)), Some((Region::Reward, 0)));
        assert_eq!(mark(&e, Field::Reward, lk), Mark::None);
    }

    #[test]
    fn statuses() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/sc-data/tests/fixtures/scunpacked");
        use sc_data::UniverseSource;
        let u = sc_data::ScunpackedSource::from_dir(dir)
            .load()
            .unwrap_or_else(|e| panic!("{e}"))
            .universe;
        let places = Places::new(&u, &BTreeMap::new());
        let lk = Lookup {
            universe: &u,
            places: &places,
        };
        let mut e = entry(Place::new("Lorville"), "Area18", 4);
        assert_eq!(status(&e, lk), Status::Ready);

        e.reads.push(Read {
            kind: ReadKind::Commodity,
            read: "Gald".into(),
            known: "Gold".into(),
        });
        e.stale = true;
        assert_eq!(unsure(&e, lk), [Field::Commodity(0)]);
        assert_eq!(status(&e, lk), Status::ToCheck(2));
        e.confirmed.push(("cargo".into(), "Gold".into()));
        e.stale = false;
        assert_eq!(status(&e, lk), Status::Ready);

        e.contract.reward = None;
        e.contract.flows[0].pickups[0] = Place::new("Nowhere Yard");
        assert_eq!(
            status(&e, lk),
            Status::Blocked(3),
            "unknown place, reward + problem"
        );
    }

    #[test]
    fn unsure_fields_until_confirmed() {
        let u = empty();
        let places = Places::new(&u, &BTreeMap::new());
        let lk = Lookup {
            universe: &u,
            places: &places,
        };
        // Places are unknown (red) in an empty universe: never "unsure", never confirmed away.
        let mut e = entry(Place::uncertain("A"), "B", 4);
        e.shots = vec![ShotText {
            title: "Haul".into(),
            boxes: ShotBoxes {
                title: vec![Some(LineBox {
                    x: 0,
                    y: 0,
                    w: 10,
                    h: 10,
                    conf: Some(0.5),
                })],
                ..ShotBoxes::default()
            },
            ..ShotText::default()
        }];
        assert_eq!(unsure(&e, lk), [Field::Title]);
        e.confirmed.push(("title".into(), "Haul".into()));
        e.confirmed.push(("pickup".into(), "A".into()));
        assert!(unsure(&e, lk).is_empty());
        assert_eq!(mark(&e, Field::Pickup(0, 0), lk), Mark::Bad);
    }

    #[test]
    fn duplicate_pairs() {
        let a = entry(Place::new("A"), "B", 4);
        let b = Entry { id: 8, ..a.clone() };
        let mut c = Entry { id: 9, ..a.clone() };
        c.contract.reward = Some(5);
        assert_eq!(duplicates(&[a, b, c]), [(7, 8)]);
    }
}
