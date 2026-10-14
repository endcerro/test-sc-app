//! Every editable value of a contract, addressed as a [`Field`]: the text it shows, and
//! setting it from what the player typed. Flow fields carry the line index `k`; place
//! fields the index `j` in their list.

use sc_contracts::{Contract, DropOff, Flow, Place, truth};

/// One editable value of a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Title,
    Reward,
    Contractor,
    Availability,
    MaxBox,
    Rep,
    /// SCU of line `k`: the drop-off's amount when it has one drop-off. With several, the
    /// line shows their total and each has a [`Field::DropScu`].
    Scu(usize),
    /// Commodity of line `k`, `[!]` prefix for restricted cargo.
    Commodity(usize),
    /// Pickup `j` of line `k`. Past the end adds one; emptied removes it.
    Pickup(usize, usize),
    /// Drop-off `j` of line `k`, likewise.
    Dropoff(usize, usize),
    /// Amount of drop-off `j` of line `k`.
    DropScu(usize, usize),
}

impl Field {
    /// The cargo line it belongs to.
    pub fn line(self) -> Option<usize> {
        match self {
            Self::Scu(k)
            | Self::Commodity(k)
            | Self::Pickup(k, _)
            | Self::Dropoff(k, _)
            | Self::DropScu(k, _) => Some(k),
            _ => None,
        }
    }

    /// What the value is, in messages ("read as the drop-off").
    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Reward => "reward",
            Self::Contractor => "contractor",
            Self::Availability => "availability",
            Self::MaxBox => "largest box",
            Self::Rep => "reputation",
            Self::Scu(_) | Self::DropScu(..) => "SCU",
            Self::Commodity(_) => "cargo",
            Self::Pickup(..) => "pickup",
            Self::Dropoff(..) => "drop-off",
        }
    }

    pub fn is_place(self) -> bool {
        matches!(self, Self::Pickup(..) | Self::Dropoff(..))
    }
}

/// The fields of a contract in reading order (an empty place list still has its first
/// field, to type into).
pub fn all(c: &Contract) -> Vec<Field> {
    let mut out = vec![
        Field::Title,
        Field::Reward,
        Field::Contractor,
        Field::Availability,
        Field::MaxBox,
        Field::Rep,
    ];
    for (k, f) in c.flows.iter().enumerate() {
        out.extend([Field::Scu(k), Field::Commodity(k)]);
        out.extend((0..f.pickups.len().max(1)).map(|j| Field::Pickup(k, j)));
        out.extend((0..f.dropoffs.len().max(1)).map(|j| Field::Dropoff(k, j)));
        if f.dropoffs.len() > 1 {
            out.extend((0..f.dropoffs.len()).map(|j| Field::DropScu(k, j)));
        }
    }
    out
}

/// The text a field shows.
pub fn text(c: &Contract, f: Field) -> String {
    let flow = f.line().and_then(|k| c.flows.get(k));
    let number = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
    match f {
        Field::Title => c.title.clone().unwrap_or_default(),
        Field::Reward => c.reward.map(truth::thousands).unwrap_or_default(),
        Field::Contractor => c.contractor.clone().unwrap_or_default(),
        Field::Availability => c.availability.clone().unwrap_or_default(),
        Field::MaxBox => number(c.max_box),
        Field::Rep => number(c.rep),
        Field::Scu(_) => flow
            .map(|f| match f.dropoffs.as_slice() {
                [] => String::new(),
                [d] => d.scu.to_string(),
                _ => f.scu().to_string(),
            })
            .unwrap_or_default(),
        Field::Commodity(_) => flow.map(Flow::display_commodity).unwrap_or_default(),
        Field::Pickup(_, j) => flow
            .and_then(|f| f.pickups.get(j))
            .map(|p| p.name.clone())
            .unwrap_or_default(),
        Field::Dropoff(_, j) => flow
            .and_then(|f| f.dropoffs.get(j))
            .map(|d| d.place.name.clone())
            .unwrap_or_default(),
        Field::DropScu(_, j) => flow
            .and_then(|f| f.dropoffs.get(j))
            .map(|d| d.scu.to_string())
            .unwrap_or_default(),
    }
}

/// Digits of a typed number (`525,500 aUEC` -> 525500); `None` when there are none.
pub fn parse_number<T: std::str::FromStr>(text: &str) -> Option<T> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Sets a field from what the player typed. A typed place is no longer "inferred".
pub fn set(c: &mut Contract, f: Field, value: &str) {
    let text = value.trim();
    let opt = || (!text.is_empty()).then(|| text.to_owned());
    match f {
        Field::Title => c.title = opt(),
        Field::Reward => c.reward = parse_number(text),
        Field::Contractor => c.contractor = opt(),
        Field::Availability => c.availability = opt(),
        Field::MaxBox => c.max_box = parse_number(text),
        Field::Rep => c.rep = parse_number(text),
        Field::Scu(k)
        | Field::Commodity(k)
        | Field::Pickup(k, _)
        | Field::Dropoff(k, _)
        | Field::DropScu(k, _) => {
            if let Some(flow) = c.flows.get_mut(k) {
                set_line(flow, f, text);
            }
        }
    }
}

fn set_line(flow: &mut Flow, f: Field, text: &str) {
    let scu = || parse_number(text).unwrap_or(0);
    match f {
        Field::Scu(_) => match flow.dropoffs.as_mut_slice() {
            [] => flow.dropoffs.push(DropOff {
                place: Place::new(""),
                scu: scu(),
            }),
            [d] => d.scu = scu(),
            // Several drop-offs: the total is not editable, each amount is.
            _ => {}
        },
        Field::DropScu(_, j) => {
            if let Some(d) = flow.dropoffs.get_mut(j) {
                d.scu = scu();
            }
        }
        Field::Commodity(_) => {
            let (restricted, name) = match text.strip_prefix("[!]") {
                Some(rest) => (true, rest.trim()),
                None => (false, text),
            };
            flow.commodity = name.to_owned();
            flow.restricted = restricted;
        }
        Field::Pickup(_, j) => match (text.is_empty(), j < flow.pickups.len()) {
            (true, true) => {
                flow.pickups.remove(j);
            }
            (false, true) => flow.pickups[j] = Place::new(text),
            (false, false) => flow.pickups.push(Place::new(text)),
            (true, false) => {}
        },
        Field::Dropoff(_, j) => match (text.is_empty(), j < flow.dropoffs.len()) {
            (true, true) => {
                flow.dropoffs.remove(j);
            }
            (false, true) => flow.dropoffs[j].place = Place::new(text),
            (false, false) => flow.dropoffs.push(DropOff {
                place: Place::new(text),
                scu: 0,
            }),
            (true, false) => {}
        },
        _ => {}
    }
}

/// A new cargo line with the previous line's places (most contracts share them), and no
/// amounts.
pub fn add_line(c: &mut Contract) {
    let mut flow = c.flows.last().cloned().unwrap_or_default();
    flow.commodity.clear();
    flow.restricted = false;
    flow.loaded = false;
    for d in &mut flow.dropoffs {
        d.scu = 0;
    }
    c.flows.push(flow);
}

/// Replaces every place written `written` with `suggestion`, on every line.
pub fn replace_place(c: &mut Contract, written: &str, suggestion: &str) {
    let places = c.flows.iter_mut().flat_map(|flow| {
        flow.pickups
            .iter_mut()
            .chain(flow.dropoffs.iter_mut().map(|d| &mut d.place))
    });
    for p in places.filter(|p| p.name == written) {
        *p = Place::new(suggestion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> Contract {
        Contract {
            title: Some("Haul".into()),
            reward: Some(525_500),
            flows: vec![Flow {
                commodity: "Neon".into(),
                restricted: true,
                pickups: vec![Place::uncertain("A")],
                dropoffs: vec![DropOff {
                    place: Place::new("B"),
                    scu: 5,
                }],
                loaded: true,
            }],
            ..Contract::default()
        }
    }

    #[test]
    fn texts() {
        let c = contract();
        assert_eq!(text(&c, Field::Reward), "525,500");
        assert_eq!(text(&c, Field::Scu(0)), "5");
        assert_eq!(text(&c, Field::Commodity(0)), "[!] Neon");
        assert_eq!(text(&c, Field::Pickup(0, 0)), "A");
        assert_eq!(text(&c, Field::Pickup(0, 1)), "");
        assert_eq!(text(&c, Field::MaxBox), "");
    }

    #[test]
    fn set_header_and_numbers() {
        let mut c = contract();
        set(&mut c, Field::Reward, "1 000 aUEC");
        set(&mut c, Field::Title, "  ");
        set(&mut c, Field::MaxBox, "x");
        set(&mut c, Field::Commodity(0), "Gold");
        assert_eq!(c.reward, Some(1000));
        assert_eq!(c.title, None);
        assert_eq!(c.max_box, None);
        assert_eq!(
            (c.flows[0].commodity.as_str(), c.flows[0].restricted),
            ("Gold", false)
        );
        set(&mut c, Field::Commodity(0), "[!] Widow");
        assert_eq!(
            (c.flows[0].commodity.as_str(), c.flows[0].restricted),
            ("Widow", true)
        );
    }

    #[test]
    fn places_add_edit_remove() {
        let mut c = contract();
        set(&mut c, Field::Pickup(0, 0), "A");
        assert!(!c.flows[0].pickups[0].uncertain, "typed = confirmed");
        set(&mut c, Field::Dropoff(0, 1), "C");
        let f = &c.flows[0];
        assert_eq!(f.dropoffs.len(), 2);
        assert_eq!(text(&c, Field::Scu(0)), "5", "total of 5 + 0");
        set(&mut c, Field::DropScu(0, 1), "6");
        set(&mut c, Field::Scu(0), "99");
        assert_eq!(text(&c, Field::Scu(0)), "11", "total is not editable");
        set(&mut c, Field::Dropoff(0, 0), "");
        assert_eq!(c.flows[0].dropoffs[0].place.name, "C");
        assert_eq!(text(&c, Field::Scu(0)), "6");
        set(&mut c, Field::Pickup(0, 5), "");
        assert_eq!(c.flows[0].pickups.len(), 1);
    }

    #[test]
    fn lines() {
        let mut c = contract();
        add_line(&mut c);
        let f = &c.flows[1];
        assert_eq!((f.commodity.as_str(), f.scu(), f.loaded), ("", 0, false));
        assert_eq!(f.pickups[0].name, "A");
        assert_eq!(
            all(&c).iter().filter(|f| f.line() == Some(1)).count(),
            4,
            "SCU, cargo, one pickup, one drop-off"
        );
        replace_place(&mut c, "A", "Area18");
        assert_eq!(c.flows[1].pickups[0].name, "Area18");
        assert_eq!(c.flows[0].pickups[0].name, "Area18");
    }
}
