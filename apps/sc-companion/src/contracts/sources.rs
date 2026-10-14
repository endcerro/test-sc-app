//! Where on its screenshots a contract value was read: the box of the OCR line(s), so the
//! player can check a field against the pixels. Values are found back by content (the
//! reward's digits, the bullet whose commodity and place match a line...), so they still
//! point to the right place after a merge of scrolled shots, and edits that move lines.

use sc_contracts::ObjectiveItem;
use sc_contracts::parse::{objective_segments, offer_entry};
use sc_ocr::Region;
use sc_universe::normalize;
use strsim::normalized_levenshtein;

use super::fields::Field;
use super::{Entry, LineBox};

/// A box on one of the entry's screenshots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Source {
    /// Index into `Entry::shots`.
    pub shot: usize,
    /// Region the value was read in: [`Region::Offer`] when the reward box was unreadable
    /// and the offer list supplied it.
    pub region: Region,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Lowest engine confidence of the lines.
    pub conf: Option<f32>,
}

/// Lowest similarity accepted when finding a value back in the text.
const MIN_SCORE: f64 = 0.6;

/// The reward box, then the offer list entry that stands in for it when the game draws the
/// box garbled; on equal scores the box wins.
const BOX_THEN_LIST: &[Region] = &[Region::Reward, Region::Offer];

/// Where field `f` was read, if it came from a screenshot.
pub fn locate(entry: &Entry, f: Field) -> Option<Source> {
    let c = &entry.contract;
    let flow = f.line().and_then(|k| c.flows.get(k));
    match f {
        Field::Title => whole_region(entry, Region::Title),
        Field::Reward => {
            let reward = c.reward?;
            let digits = reward.to_string();
            best_line(entry, BOX_THEN_LIST, |region, line| {
                let found = match region {
                    // `55k`: rounded, so only an exact match of the parsed amount counts.
                    Region::Offer => offer_entry(line).reward == Some(reward),
                    _ => line.chars().filter(char::is_ascii_digit).eq(digits.chars()),
                };
                Some(if found { 1.0 } else { 0.0 })
            })
        }
        Field::Contractor => best_line(entry, BOX_THEN_LIST, |_, l| {
            similar(l, c.contractor.as_deref()?)
        }),
        Field::Availability => best_line(entry, BOX_THEN_LIST, |_, l| {
            similar(l, c.availability.as_deref()?)
        }),
        Field::MaxBox => {
            let want = format!("{} scu", c.max_box?);
            best_line(entry, &[Region::Details], |_, l| {
                Some(if normalize(l).contains(&want) {
                    1.0
                } else {
                    0.0
                })
            })
        }
        Field::Rep => {
            let want = c.rep?.to_string();
            best_line(entry, &[Region::Details], |_, l| {
                let n = normalize(l);
                Some(match (n.contains("reputation"), n.contains(&want)) {
                    (_, false) => 0.0,
                    (true, true) => 1.0,
                    (false, true) => 0.7,
                })
            })
        }
        Field::Scu(_) | Field::Commodity(_) => {
            let flow = flow?;
            let drop = flow
                .dropoffs
                .first()
                .map(|d| (d.place.name.as_str(), d.scu));
            best_item(entry, |item| deliver_score(item, &flow.commodity, drop))
        }
        Field::Dropoff(_, j) | Field::DropScu(_, j) => {
            let flow = flow?;
            let d = flow.dropoffs.get(j)?;
            best_item(entry, |item| {
                deliver_score(item, &flow.commodity, Some((&d.place.name, d.scu)))
            })
        }
        Field::Pickup(_, j) => {
            let flow = flow?;
            let p = flow.pickups.get(j)?;
            best_item(entry, |item| match item {
                ObjectiveItem::Collect { commodity, from } => {
                    Some((similar(commodity, &flow.commodity)? + similar(from, &p.name)?) / 2.0)
                }
                ObjectiveItem::Deliver { .. } => None,
            })
        }
    }
}

/// Similarity of two names: 1 when one contains the other (`Ruin Station` in `Ruin
/// Station above Pyro VI`), else the normalised edit similarity.
fn similar(a: &str, b: &str) -> Option<f64> {
    let (a, b) = (normalize(a), normalize(b));
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let s = if a.contains(&b) || b.contains(&a) {
        1.0
    } else {
        normalized_levenshtein(&a, &b)
    };
    Some(s)
}

/// How well a `Deliver` bullet matches a commodity and (optionally) a drop-off and amount.
fn deliver_score(item: &ObjectiveItem, commodity: &str, drop: Option<(&str, u32)>) -> Option<f64> {
    let ObjectiveItem::Deliver {
        scu,
        commodity: what,
        to,
        ..
    } = item
    else {
        return None;
    };
    let goods = similar(what, commodity)?;
    let Some((place, amount)) = drop else {
        return Some(goods);
    };
    let place = similar(to, place).unwrap_or(0.0);
    let amount = if *scu == Some(amount) { 1.0 } else { 0.0 };
    Some((2.0 * goods + 2.0 * place + amount) / 5.0)
}

/// Union of all the lines of a region, on the first shot that has them.
fn whole_region(entry: &Entry, region: Region) -> Option<Source> {
    entry.shots.iter().enumerate().find_map(|(shot, s)| {
        let boxes: Vec<LineBox> = s.lines(region).into_iter().filter_map(|l| l.1).collect();
        union(shot, region, &boxes)
    })
}

/// The best scoring line of some regions, over all shots; on equal scores the earlier
/// region wins.
fn best_line(
    entry: &Entry,
    regions: &[Region],
    score: impl Fn(Region, &str) -> Option<f64>,
) -> Option<Source> {
    let mut best: Option<(f64, Source)> = None;
    for &region in regions {
        for (shot, s) in entry.shots.iter().enumerate() {
            for (text, b) in s.lines(region) {
                let (Some(b), Some(sc)) = (b, score(region, text)) else {
                    continue;
                };
                if sc >= MIN_SCORE && best.is_none_or(|(have, _)| sc > have) {
                    best = union(shot, region, &[b]).map(|src| (sc, src));
                }
            }
        }
    }
    best.map(|(_, s)| s)
}

/// The best scoring objective bullet, over all shots, as the union of its lines.
fn best_item(entry: &Entry, score: impl Fn(&ObjectiveItem) -> Option<f64>) -> Option<Source> {
    let mut best: Option<(f64, Source)> = None;
    for (shot, s) in entry.shots.iter().enumerate() {
        let lines = s.lines(Region::Objectives);
        let texts: Vec<String> = lines.iter().map(|l| l.0.to_owned()).collect();
        for (item, range) in objective_segments(&texts) {
            let Some(sc) = score(&item).filter(|sc| *sc >= MIN_SCORE) else {
                continue;
            };
            let boxes: Vec<LineBox> = lines[range].iter().filter_map(|l| l.1).collect();
            if best.is_none_or(|(have, _)| sc > have)
                && let Some(src) = union(shot, Region::Objectives, &boxes)
            {
                best = Some((sc, src));
            }
        }
    }
    best.map(|(_, s)| s)
}

fn union(shot: usize, region: Region, boxes: &[LineBox]) -> Option<Source> {
    let x0 = boxes.iter().map(|b| b.x).min()?;
    let y0 = boxes.iter().map(|b| b.y).min()?;
    let x1 = boxes.iter().map(|b| b.x + b.w).max()?;
    let y1 = boxes.iter().map(|b| b.y + b.h).max()?;
    let conf = boxes.iter().filter_map(|b| b.conf).min_by(f32::total_cmp);
    Some(Source {
        shot,
        region,
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
        conf,
    })
}

#[cfg(test)]
mod tests {
    use sc_contracts::{Contract, DropOff, Flow, Place};

    use super::*;
    use crate::contracts::{ShotBoxes, ShotText};

    fn b(y: u32, conf: f32) -> Option<LineBox> {
        Some(LineBox {
            x: 10,
            y,
            w: 100,
            h: 10,
            conf: Some(conf),
        })
    }

    fn entry() -> Entry {
        let shot = ShotText {
            title: "Rookie | Haul".into(),
            reward: "49,750\n2h 1m\nCovalex Independent Contractors".into(),
            details: "Max size will be 4 SCU".into(),
            objectives: "Deliver 0/5 SCU of Corundum to Seraphim Station above\n\
                         Crusader.\n\
                         Collect Corundum from ARC-L1 Wide Forest Station.\n\
                         Deliver 0/6 SCU of Corundum to Baijini Point above ArcCorp.\n\
                         Collect Corundum from ARC-L1 Wide Forest Station."
                .into(),
            boxes: ShotBoxes {
                title: vec![b(5, 0.99)],
                reward: vec![b(20, 0.98), b(30, 0.97), b(40, 0.6)],
                details: vec![b(60, 0.99)],
                objectives: vec![
                    b(100, 0.99),
                    b(110, 0.95),
                    b(120, 0.99),
                    b(130, 0.99),
                    b(140, 0.9),
                ],
                ..ShotBoxes::default()
            },
            ..ShotText::default()
        };
        let drop = |name: &str, scu| DropOff {
            place: Place::new(name),
            scu,
        };
        Entry {
            id: 1,
            contract: Contract {
                title: Some("Rookie | Haul".into()),
                reward: Some(49_750),
                contractor: Some("Covalex Independent Contractors".into()),
                availability: Some("2h 1m".into()),
                max_box: Some(4),
                rep: None,
                flows: vec![Flow {
                    commodity: "Corundum".into(),
                    restricted: false,
                    pickups: vec![Place::new("ARC-L1 Wide Forest Station")],
                    dropoffs: vec![
                        drop("Seraphim Station above Crusader", 5),
                        drop("Baijini Point above ArcCorp", 6),
                    ],
                    loaded: false,
                }],
            },
            shots: vec![shot],
            ..Entry::default()
        }
    }

    fn y(src: Option<Source>) -> Option<(u32, u32)> {
        src.map(|s| (s.y, s.h))
    }

    #[test]
    fn finds_values_back() {
        let e = entry();
        assert_eq!(y(locate(&e, Field::Title)), Some((5, 10)));
        assert_eq!(y(locate(&e, Field::Reward)), Some((20, 10)));
        assert_eq!(y(locate(&e, Field::Availability)), Some((30, 10)));
        let contractor = locate(&e, Field::Contractor);
        assert_eq!(y(contractor), Some((40, 10)));
        assert_eq!(contractor.and_then(|s| s.conf), Some(0.6));
        assert_eq!(y(locate(&e, Field::MaxBox)), Some((60, 10)));
        assert_eq!(locate(&e, Field::Rep), None);
        // The first Deliver spans two lines; the second drop-off is its own bullet.
        assert_eq!(y(locate(&e, Field::Dropoff(0, 0))), Some((100, 20)));
        assert_eq!(
            locate(&e, Field::Dropoff(0, 0)).and_then(|s| s.conf),
            Some(0.95)
        );
        assert_eq!(y(locate(&e, Field::DropScu(0, 1))), Some((130, 10)));
        assert_eq!(y(locate(&e, Field::Commodity(0))), Some((100, 20)));
        assert_eq!(y(locate(&e, Field::Pickup(0, 0))), Some((120, 10)));
        assert_eq!(locate(&e, Field::Pickup(0, 3)), None);
    }

    #[test]
    fn nothing_without_shots() {
        let e = Entry {
            shots: Vec::new(),
            ..entry()
        };
        assert_eq!(locate(&e, Field::Reward), None);
    }
}
