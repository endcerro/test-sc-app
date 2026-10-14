//! The hand-written `#HAULING v1` truth format.
//!
//! ```text
//! #HAULING v1
//!
//! # comment
//! CONTRACT: Rookie | DIRECT Interstellar Bulk Haul | Orbituary > Teasa Spaceport
//! REWARD: 308,250 aUEC
//! CONTRACTOR: Red Wind Linehaul
//! AVAILABILITY: N/A
//! MAXBOX: 8
//! REP: 200
//! CARGO: 32 SCU | Distilled Spirits | Orbituary above Pyro III | Teasa Spaceport in Lorville
//! CARGO: 3 SCU | [!] Neon | Place A + Place B | Drop
//! ```
//!
//! Column padding inside `CARGO` lines is ignored. Several pickups are joined with ` + `.
//! A trailing `?` marks a place that was not read but inferred.

use thiserror::Error;

use crate::{Cargo, HaulingContract, Place};

pub const HEADER: &str = "#HAULING v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TruthError {
    #[error("missing `{HEADER}` header")]
    MissingHeader,
    #[error("line {line}: {message}")]
    Line { line: usize, message: String },
}

pub fn parse(text: &str) -> Result<HaulingContract, TruthError> {
    let mut lines = text.lines().enumerate();
    let header = lines
        .by_ref()
        .map(|(_, l)| l.trim())
        .find(|l| !l.is_empty());
    if header != Some(HEADER) {
        return Err(TruthError::MissingHeader);
    }

    let mut c = HaulingContract::default();
    for (i, raw) in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |message: String| TruthError::Line {
            line: i + 1,
            message,
        };
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| err(format!("expected `KEY: value`, got `{line}`")))?;
        let value = value.trim();
        let number = |v: &str| -> Result<u64, TruthError> {
            let digits: String = v.chars().filter(char::is_ascii_digit).collect();
            digits
                .parse()
                .map_err(|_| err(format!("expected a number, got `{v}`")))
        };
        let small = |v: &str| -> Result<u32, TruthError> {
            u32::try_from(number(v)?).map_err(|_| err(format!("number too large: `{v}`")))
        };
        match key.trim() {
            "CONTRACT" => c.title = Some(value.to_owned()),
            "REWARD" => c.reward = Some(number(value)?),
            "CONTRACTOR" => c.contractor = Some(value.to_owned()),
            "AVAILABILITY" => c.availability = Some(value.to_owned()),
            "MAXBOX" => c.max_box = Some(small(value)?),
            "REP" => c.rep = Some(small(value)?),
            "CARGO" => {
                let cols: Vec<&str> = value.split('|').map(str::trim).collect();
                let [scu, commodity, pickups, dropoff] = cols[..] else {
                    return Err(err(format!(
                        "CARGO needs 4 `|` columns, got {}",
                        cols.len()
                    )));
                };
                let (restricted, commodity) = match commodity.strip_prefix("[!]") {
                    Some(rest) => (true, rest.trim()),
                    None => (false, commodity),
                };
                c.cargo.push(Cargo {
                    scu: small(scu)?,
                    commodity: commodity.to_owned(),
                    restricted,
                    pickups: pickups.split(" + ").map(parse_place).collect(),
                    dropoff: parse_place(dropoff),
                });
            }
            other => return Err(err(format!("unknown key `{other}`"))),
        }
    }
    Ok(c)
}

fn parse_place(s: &str) -> Place {
    let s = s.trim();
    match s.strip_suffix('?') {
        Some(name) => Place::uncertain(name.trim()),
        None => Place::new(s),
    }
}

pub fn serialize(c: &HaulingContract) -> String {
    let mut out = format!("{HEADER}\n\n");
    let mut kv = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            out.push_str(&format!("{k}: {v}\n"));
        }
    };
    kv("CONTRACT", c.title.clone());
    kv("REWARD", c.reward.map(|r| format!("{} aUEC", thousands(r))));
    kv("CONTRACTOR", c.contractor.clone());
    kv("AVAILABILITY", c.availability.clone());
    kv("MAXBOX", c.max_box.map(|v| v.to_string()));
    kv("REP", c.rep.map(|v| v.to_string()));
    for cargo in &c.cargo {
        let pickups: Vec<String> = cargo.pickups.iter().map(ToString::to_string).collect();
        out.push_str(&format!(
            "CARGO: {} SCU | {} | {} | {}\n",
            cargo.scu,
            cargo.display_commodity(),
            pickups.join(" + "),
            cargo.dropoff
        ));
    }
    out
}

/// `535000` -> `535,000`.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "#HAULING v1\n\n# note\nCONTRACT: A | B\nREWARD: 535,000 aUEC\n\
        CONTRACTOR: Dead Saints\nAVAILABILITY: N/A\nMAXBOX: 2\nREP: 100\n\
        CARGO: 4 SCU | [!] E'tam | Samson & Son's Salvage Center on Wala | Riker\n\
        CARGO: 3 SCU  | Aluminum (Ore) | X on Lyria + Y on Lyria | Baijini Point\n\
        CARGO: 20 SCU | Medical Supplies | Yela facility? | Chawla's Beach\n";

    #[test]
    fn parses_all_keys() {
        let c = parse(SAMPLE).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.title.as_deref(), Some("A | B"));
        assert_eq!(c.reward, Some(535_000));
        assert_eq!(c.max_box, Some(2));
        assert_eq!(c.rep, Some(100));
        assert_eq!(c.cargo.len(), 3);
        assert!(c.cargo[0].restricted);
        assert_eq!(c.cargo[0].commodity, "E'tam");
        assert_eq!(c.cargo[1].pickups.len(), 2);
        assert_eq!(c.cargo[2].pickups[0], Place::uncertain("Yela facility"));
    }

    #[test]
    fn round_trips() {
        let c = parse(SAMPLE).unwrap_or_else(|e| panic!("{e}"));
        let again = parse(&serialize(&c)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c, again);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(parse("CONTRACT: x"), Err(TruthError::MissingHeader));
        assert!(parse("#HAULING v1\nCARGO: 1 SCU | x").is_err());
        assert!(parse("#HAULING v1\nFOO: 1").is_err());
    }

    #[test]
    fn formats_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(49_750), "49,750");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
