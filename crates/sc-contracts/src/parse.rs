//! OCR'd region text -> [`HaulingContract`].

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use sc_universe::normalize;

use crate::repair::{self, clean_text, leading_keyword};
use crate::{Cargo, HaulingContract, Place, merge};

/// Text read from each region of one screenshot. Lines are separated by `\n`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionText {
    pub title: Option<String>,
    /// Values column of the reward box: reward, availability, contractor.
    pub reward: Option<String>,
    pub details: Option<String>,
    /// Primary objectives: one entry per OCR line, or per objective when the caller already
    /// segmented them (one crop per bullet). Entries that don't start with
    /// `Deliver`/`Collect` continue the previous one.
    pub objectives: Vec<String>,
    /// The highlighted entry of the offer list: title, rounded reward, availability,
    /// contractor. Fallback for the reward box, see [`offer_entry`].
    pub offer: Option<String>,
}

/// One bullet of the Primary Objectives list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectiveItem {
    Deliver {
        scu: Option<u32>,
        commodity: String,
        restricted: bool,
        to: String,
    },
    Collect {
        commodity: String,
        from: String,
    },
}

impl ObjectiveItem {
    /// Normalised text used to compare items across screenshots.
    pub fn key(&self) -> String {
        match self {
            Self::Deliver {
                scu, commodity, to, ..
            } => normalize(&format!(
                "deliver {} {commodity} {to}",
                scu.map_or(String::new(), |s| s.to_string())
            )),
            Self::Collect { commodity, from } => normalize(&format!("collect {commodity} {from}")),
        }
    }
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => panic!("invalid built-in regex {pattern}: {e}"),
    })
}

/// Parses one screenshot.
pub fn contract(regions: &RegionText) -> HaulingContract {
    contract_from_shots(std::slice::from_ref(regions))
}

/// Parses several screenshots of the same contract (the objective list scrolled between
/// them). Scalar fields come from the first shot that has them; objective lists are merged
/// by overlap, see [`merge::merge_items`]. The reward box is checked against the offer list
/// entry, see [`with_offer`].
pub fn contract_from_shots(shots: &[RegionText]) -> HaulingContract {
    let first = |f: fn(&RegionText) -> Option<&String>| shots.iter().find_map(f).cloned();
    let title = first(|r| r.title.as_ref()).and_then(|t| title_text(&t));
    let boxed = first(|r| r.reward.as_ref())
        .map(|t| reward_box(&t))
        .unwrap_or_default();
    let offer = first(|r| r.offer.as_ref())
        .map(|t| offer_entry(&t))
        .filter(|o| same_title(title.as_deref(), o.title.as_deref()));
    let (reward, availability, contractor) = with_offer(boxed, offer.as_ref());
    let details = first(|r| r.details.as_ref()).unwrap_or_default();
    let info = details_info(&details);
    let lists: Vec<Vec<ObjectiveItem>> = shots
        .iter()
        .map(|s| objective_items(&s.objectives))
        .collect();
    let items = merge::merge_items(&lists);
    HaulingContract {
        title,
        reward,
        contractor,
        availability,
        max_box: info.max_box,
        rep: info.rep,
        cargo: build_cargo(&items, &details),
    }
}

/// Title lines joined; a lone `I`/`l`/`1` between words is the `|` separator.
pub fn title_text(text: &str) -> Option<String> {
    let joined = clean_text(text);
    if joined.is_empty() {
        return None;
    }
    let words: Vec<&str> = joined
        .split(' ')
        .map(|w| match w {
            "I" | "l" | "1" | "!" | "]" | "[" => "|",
            w => w,
        })
        .collect();
    Some(words.join(" "))
}

/// Values column of the reward box -> (reward, availability, contractor).
/// Tolerates the labels being included (`Reward`, `Contract Availability`, `Contracted By`).
pub fn reward_box(text: &str) -> (Option<u64>, Option<String>, Option<String>) {
    static MONEY: OnceLock<Regex> = OnceLock::new();
    static NA: OnceLock<Regex> = OnceLock::new();
    static TIME: OnceLock<Regex> = OnceLock::new();
    let money = re(&MONEY, r"\d[\dOoBlI]{0,2}(?:[,.][\dOoBlI]{3})+|\d{4,}");
    let na = re(&NA, r"(?i)\bN\s*[/I|l1]\s*[A4]\b");
    let time = re(&TIME, r"(?i)\b([\dOolI]{1,3})\s*([hms])\b");

    let (mut reward, mut availability, mut contractor) = (None, None, None);
    for raw in text.lines() {
        let mut line = clean_text(raw);
        for label in ["Contract Availability", "Contracted By", "Reward"] {
            if let Some(rest) = strip_prefix_ci(&line, label) {
                line = rest.trim().to_owned();
            }
        }
        if line.is_empty() {
            continue;
        }
        if reward.is_none()
            && let Some(m) = money.find(&line)
        {
            reward = repair::number(m.as_str());
            if reward.is_some() {
                continue;
            }
        }
        if availability.is_none() {
            if na.is_match(&line) {
                availability = Some("N/A".to_owned());
                continue;
            }
            let parts: Vec<String> = time
                .captures_iter(&line)
                .filter_map(|c| {
                    let n = repair::number(&c[1])?;
                    Some(format!("{n}{}", c[2].to_lowercase()))
                })
                .collect();
            if !parts.is_empty() {
                availability = Some(parts.join(" "));
                continue;
            }
        }
        let letters = line.chars().filter(|c| c.is_alphabetic()).count();
        if contractor.is_none() && letters >= 3 {
            contractor = Some(line);
        }
    }
    (reward, availability, contractor)
}

/// What the highlighted entry of the offer list shows:
///
/// ```text
/// JUNIOR | DIRECT MEDIUM HAUL | HUR-           55k
/// L2 FAITHFUL DREAM STATION -> PORT         1h 56m
/// TRESSLER [BP]*
/// COVALEX INDEPENDENT CONTRACTORS
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OfferEntry {
    /// Title lines joined (upper case on screen).
    pub title: Option<String>,
    /// Reward as shown, rounded: `55k` -> 55,000.
    pub reward: Option<u64>,
    /// What `reward` is rounded to: 1,000 for `55k`, 100,000 for `1.2M`.
    pub reward_step: u64,
    /// Countdown (`1h 56m`); the list shows none for `N/A`.
    pub availability: Option<String>,
    /// The bottom line, title-cased: `Covalex Independent Contractors`.
    pub contractor: Option<String>,
}

/// Parses the text of the highlighted offer list entry (lines in reading order; the
/// reward and countdown, right-aligned, can come anywhere among the title lines).
pub fn offer_entry(text: &str) -> OfferEntry {
    static AMOUNT: OnceLock<Regex> = OnceLock::new();
    static NA: OnceLock<Regex> = OnceLock::new();
    static TIME: OnceLock<Regex> = OnceLock::new();
    // `M` upper case only: `56m` is a countdown.
    let amount = re(
        &AMOUNT,
        r"(?:^|\s)([\dOolI]{1,4})(?:[.,]([\dOolI]{1,2}))?\s*([kK]|M)$",
    );
    let na = re(&NA, r"(?i)^N\s*[/I|l1]\s*[A4]$");
    let time = re(&TIME, r"\b([\dOolI]{1,3})\s*([hms])\b");

    let mut out = OfferEntry::default();
    let mut texts: Vec<String> = Vec::new();
    for raw in text.lines() {
        let mut line = clean_text(raw);
        if out.availability.is_none() {
            if na.is_match(&line) {
                out.availability = Some("N/A".to_owned());
                continue;
            }
            let rest = time.replace_all(&line, "");
            if rest.trim().is_empty() && !line.is_empty() {
                let parts: Vec<String> = time
                    .captures_iter(&line)
                    .filter_map(|c| Some(format!("{}{}", repair::number(&c[1])?, &c[2])))
                    .collect();
                out.availability = Some(parts.join(" "));
                continue;
            }
        }
        if out.reward.is_none()
            && let Some(c) = amount.captures(&line)
            && let Some(whole) = repair::number(&c[1])
        {
            let decimals = c.get(2).map_or("", |m| m.as_str());
            let fraction = repair::number(decimals).unwrap_or(0);
            let unit: u64 = if &c[3] == "M" { 1_000_000 } else { 1_000 };
            let step = unit / 10u64.pow(decimals.len() as u32);
            out.reward = Some(whole * unit + fraction * step);
            out.reward_step = step;
            let start = c.get(0).map_or(line.len(), |m| m.start());
            line.truncate(start);
        }
        if !line.trim().is_empty() {
            texts.push(line.trim().to_owned());
        }
    }
    // The contractor is the last line, under the title; it has no title punctuation.
    let is_contractor = |l: &str| {
        l.chars().filter(|c| c.is_alphabetic()).count() >= 3
            && !l.contains(['[', ']', '|', '>', '\u{2192}'])
    };
    if texts.len() >= 2 && texts.last().is_some_and(|l| is_contractor(l)) {
        out.contractor = texts.pop().map(|l| title_case(&l));
    }
    if !texts.is_empty() {
        out.title = Some(texts.join(" "));
    }
    out
}

/// `COVALEX INDEPENDENT CONTRACTORS` -> `Covalex Independent Contractors`.
fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the offer list entry is the contract of the details panel (titles alike).
/// Unknown titles don't veto.
fn same_title(panel: Option<&str>, offer: Option<&str>) -> bool {
    let (Some(panel), Some(offer)) = (panel, offer) else {
        return true;
    };
    let (panel, offer) = (normalize(panel), normalize(offer));
    panel.contains(&offer)
        || offer.contains(&panel)
        || strsim::normalized_levenshtein(&panel, &offer) >= 0.6
}

/// The reward box values (reward, availability, contractor), checked against the offer
/// list entry. The game sometimes draws the reward box with its lines on top of each other:
/// the reward is then lost or misread and the contractor garbled (`Covalex Independent
/// Contrect9Ea`). The list's values are used then: a reward the list disagrees with (beyond
/// its rounding) or missing, and the contractor of a box that looks broken.
pub fn with_offer(
    boxed: (Option<u64>, Option<String>, Option<String>),
    offer: Option<&OfferEntry>,
) -> (Option<u64>, Option<String>, Option<String>) {
    let Some(o) = offer else {
        return boxed;
    };
    let (reward, availability, contractor) = boxed;
    let agrees = |r: u64| o.reward.is_none_or(|l| r.abs_diff(l) < o.reward_step);
    let reward_ok = reward.is_some_and(agrees);
    let broken = !reward_ok
        || contractor
            .as_deref()
            .is_some_and(|c| c.chars().any(|ch| ch.is_ascii_digit()));
    let reward = if reward_ok {
        reward
    } else {
        o.reward.or(reward)
    };
    let contractor = match (contractor, &o.contractor) {
        (Some(c), Some(listed)) if broken && normalize(&c) != normalize(listed) => {
            Some(listed.clone())
        }
        (None, listed) => listed.clone(),
        (c, _) => c,
    };
    let availability = availability.or_else(|| o.availability.clone());
    (reward, availability, contractor)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// What the details text tells beyond the objectives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailsInfo {
    /// First `N SCU` mentioned: the container size (`16 SCU cargo containers`,
    /// `Max size will be 8 SCU`, `4 SCU or smaller`, `largest container used is 2 SCU`).
    pub max_box: Option<u32>,
    /// `Reputation Awarded: N`.
    pub rep: Option<u32>,
}

pub fn details_info(text: &str) -> DetailsInfo {
    static SCU: OnceLock<Regex> = OnceLock::new();
    static REP: OnceLock<Regex> = OnceLock::new();
    let scu = re(&SCU, r"\b([\dOoBlI]{1,3})\s*(?i:SCU)\b");
    let rep = re(
        &REP,
        r"(?i)Reputation\s+Awarded\s*[:;.]?\s*([\dOoBlIS]{1,5})",
    );
    let text = clean_text(text);
    DetailsInfo {
        max_box: scu
            .captures_iter(&text)
            .find_map(|c| repair::small_number(&c[1])),
        rep: rep
            .captures(&text)
            .and_then(|c| repair::small_number(&c[1])),
    }
}

/// Groups OCR lines into bullets and parses them. Lines before the first bullet and bullets
/// that don't parse (cut off at a scroll edge) are dropped.
pub fn objective_items(lines: &[String]) -> Vec<ObjectiveItem> {
    objective_segments(lines)
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

/// Like [`objective_items`], with the indices of the input lines each bullet was read from
/// (to find a value back on the screenshot).
pub fn objective_segments(lines: &[String]) -> Vec<(ObjectiveItem, std::ops::Range<usize>)> {
    let mut segments: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let line = clean_text(raw);
        if line.is_empty() {
            continue;
        }
        let started =
            leading_keyword(&line, "Deliver").or_else(|| leading_keyword(&line, "Collect"));
        match (started, segments.last_mut()) {
            (Some(item), _) => segments.push((item, i..i + 1)),
            (None, Some((last, range))) => {
                last.push(' ');
                last.push_str(&line);
                range.end = i + 1;
            }
            (None, None) => {}
        }
    }
    segments
        .into_iter()
        .filter_map(|(s, range)| objective_item(&s).map(|item| (item, range)))
        .collect()
}

/// Parses one bullet: `Deliver 0/4 SCU of [!] E'tam to Riker Memorial Spaceport in Area18.`
/// or `Collect E'tam from Samson & Son's Salvage Center.`
pub fn objective_item(segment: &str) -> Option<ObjectiveItem> {
    static DELIVER: OnceLock<Regex> = OnceLock::new();
    static COLLECT: OnceLock<Regex> = OnceLock::new();
    let deliver = re(
        &DELIVER,
        r"(?i)^Deliver\s*(?P<qty>\S*?)\s*(?:SCU|5CU|SCV|SGU)\s+(?:of|af|oF|ot)\s+(?P<what>.+?)\s+to\s+(?P<to>.+?)[\s.,]*$",
    );
    let collect = re(
        &COLLECT,
        r"(?i)^Collect\s+(?P<what>.+?)\s+(?:from|fram|frorn)\s+(?P<from>.+?)[\s.,]*$",
    );
    let segment = clean_text(segment);
    if let Some(c) = deliver.captures(&segment) {
        let qty = &c["qty"];
        // `0/4` -> total 4; the progress counter before the slash is ignored.
        let total = qty.rsplit('/').next().unwrap_or(qty);
        let (restricted, commodity) = repair::restricted_marker(&c["what"]);
        return Some(ObjectiveItem::Deliver {
            scu: repair::small_number(total),
            commodity: repair::name(commodity),
            restricted,
            to: repair::name(&c["to"]),
        });
    }
    if let Some(c) = collect.captures(&segment) {
        let (_, commodity) = repair::restricted_marker(&c["what"]);
        return Some(ObjectiveItem::Collect {
            commodity: repair::name(commodity),
            from: repair::name(&c["from"]),
        });
    }
    None
}

/// Turns bullets into cargo lines: each `Deliver` takes the `Collect`s that follow it.
/// Short pickup names are completed from the details text (`Ruin Station` ->
/// `Ruin Station above Pyro VI`). A `Deliver` without any `Collect` (scrolled off) gets
/// the most common pickup of the contract, marked uncertain.
pub fn build_cargo(items: &[ObjectiveItem], details: &str) -> Vec<Cargo> {
    let mut cargo: Vec<Cargo> = Vec::new();
    for item in items {
        match item {
            ObjectiveItem::Deliver {
                scu,
                commodity,
                restricted,
                to,
            } => cargo.push(Cargo {
                scu: scu.unwrap_or(0),
                commodity: commodity.clone(),
                restricted: *restricted,
                pickups: Vec::new(),
                dropoff: Place::new(to.clone()),
            }),
            ObjectiveItem::Collect { from, .. } => {
                if let Some(last) = cargo.last_mut() {
                    let place = Place::new(complete_place(from, details));
                    if !last.pickups.contains(&place) {
                        last.pickups.push(place);
                    }
                }
            }
        }
    }

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for p in cargo.iter().flat_map(|c| &c.pickups) {
        *counts.entry(p.name.as_str()).or_default() += 1;
    }
    let common = counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(name, _)| name.to_owned())
        .unwrap_or_default();
    for c in &mut cargo {
        if c.pickups.is_empty() {
            c.pickups.push(Place::uncertain(common.clone()));
        }
    }
    cargo
}

/// Prepositions linking a place to its parent in the details text. `at` is left out on
/// purpose: `Wide Forest Station at ArcCorp's L1 Lagrange point` is a description, not a
/// name the game uses elsewhere.
const LINKS: [&str; 4] = ["above", "on", "in", "within"];

/// Completes a short place name from the details text: finds `<name> <link> <parent>` and
/// returns it. The parent is one word, plus a numeral after a system name (`Pyro VI`,
/// `Pyro 5b`).
pub fn complete_place(short: &str, details: &str) -> String {
    let short = clean_text(short);
    let want: Vec<String> = short.split(' ').map(normalize).collect();
    let words: Vec<&str> = details.split_whitespace().collect();
    let norm: Vec<String> = words.iter().map(|w| normalize(w)).collect();
    if want.is_empty() || want.iter().all(String::is_empty) {
        return short;
    }
    for start in 0..norm.len() {
        let end = start + want.len();
        if end >= norm.len() || norm[start..end] != want[..] {
            continue;
        }
        if !LINKS.contains(&norm[end].as_str()) {
            continue;
        }
        let Some(parent) = words.get(end + 1).map(|w| trim_punct(w)) else {
            continue;
        };
        if parent.is_empty() {
            continue;
        }
        let mut out = format!("{short} {} {parent}", words[end]);
        if !words[end + 1].ends_with(['.', ',', ';', ':'])
            && let Some(next) = words.get(end + 2).map(|w| trim_punct(w))
        {
            if let Some(numeral) = repair::roman(next) {
                out.push(' ');
                out.push_str(&numeral);
            } else if is_numeral(next) {
                out.push(' ');
                out.push_str(next);
            }
        }
        return out;
    }
    short
}

fn trim_punct(w: &str) -> &str {
    w.trim_end_matches(['.', ',', ';', ':', ')'])
}

fn is_numeral(w: &str) -> bool {
    let roman = !w.is_empty() && w.chars().all(|c| matches!(c, 'I' | 'V' | 'X'));
    let digits = w.trim_end_matches(|c: char| c.is_ascii_lowercase());
    roman || (!digits.is_empty() && digits.len() <= 2 && digits.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    #[test]
    fn parses_wrapped_objectives() {
        let items = objective_items(&lines(
            "<> Deliver 0/4 SCU of [!] E'tam to Riker Memorial Spaceport\n\
             in Area18.\n\
             o Collect [!] E'tam from Samson & Son's Salvage\n\
             Center.\n\
             Deliver 0/2 SCU of Neon to Riker Memorial Spaceport in Area18.\n\
             Collec",
        ));
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0],
            ObjectiveItem::Deliver {
                scu: Some(4),
                commodity: "E'tam".into(),
                restricted: true,
                to: "Riker Memorial Spaceport in Area18".into(),
            }
        );
        assert_eq!(
            items[1],
            ObjectiveItem::Collect {
                commodity: "E'tam".into(),
                from: "Samson & Son's Salvage Center".into(),
            }
        );
    }

    #[test]
    fn completes_places_from_details() {
        let details = "starts in the system at Ruin\nStation above Pyro VI and then goes to \
            Baijini Point above ArcCorp. Freight elevator at Seer's Canyon on Pyro 5b \
            - Freight elevator at Samson & Son's Salvage Center on Wala";
        assert_eq!(
            complete_place("Ruin Station", details),
            "Ruin Station above Pyro VI"
        );
        assert_eq!(
            complete_place("Baijini Point", details),
            "Baijini Point above ArcCorp"
        );
        assert_eq!(
            complete_place("Seer's Canyon", details),
            "Seer's Canyon on Pyro 5b"
        );
        assert_eq!(
            complete_place("Samson & Son's Salvage Center", details),
            "Samson & Son's Salvage Center on Wala"
        );
        assert_eq!(complete_place("Elsewhere", details), "Elsewhere");
        assert_eq!(
            complete_place(
                "Bueno Ravine",
                "run from Bueno Ravine on Pyro Ill to Rayari"
            ),
            "Bueno Ravine on Pyro III"
        );
    }

    #[test]
    fn reward_box_values() {
        let (r, a, c) = reward_box("# 535,000\nN/A\nDead Saints");
        assert_eq!(r, Some(535_000));
        assert_eq!(a.as_deref(), Some("N/A"));
        assert_eq!(c.as_deref(), Some("Dead Saints"));
        let (r, a, c) = reward_box(
            "Reward ¤ 49,750\nContract Availability 2h lm\nContracted By Covalex Independent Contractors",
        );
        assert_eq!(r, Some(49_750));
        assert_eq!(a.as_deref(), Some("2h 1m"));
        assert_eq!(c.as_deref(), Some("Covalex Independent Contractors"));
    }

    #[test]
    fn offer_entry_values() {
        let o = offer_entry(
            "JUNIOR | DIRECT MEDIUM HAUL | HUR-\n55k\nL2 FAITHFUL DREAM STATION → PORT\n\
             1h 56m\nTRESSLER [BP]*\nCOVALEX INDEPENDENT CONTRACTORS",
        );
        assert_eq!(o.reward, Some(55_000));
        assert_eq!(o.reward_step, 1_000);
        assert_eq!(o.availability.as_deref(), Some("1h 56m"));
        assert_eq!(
            o.contractor.as_deref(),
            Some("Covalex Independent Contractors")
        );
        assert_eq!(
            o.title.as_deref(),
            Some(
                "JUNIOR | DIRECT MEDIUM HAUL | HUR- L2 FAITHFUL DREAM STATION → PORT TRESSLER [BP]*"
            )
        );
        // No countdown for N/A contracts; the reward can share a title line.
        let o = offer_entry(
            "DEAD SAINTS - ROOKIE |\nRIKER MEMORIAL SPACEPORT [100 535k\nREP]\nDEAD SAINTS",
        );
        assert_eq!(o.reward, Some(535_000));
        assert_eq!(o.availability, None);
        assert_eq!(o.contractor.as_deref(), Some("Dead Saints"));
        let o = offer_entry("SOME HAUL [BP]*\n1.2M\n59m 17s\nRED WIND LINEHAUL");
        assert_eq!((o.reward, o.reward_step), (Some(1_200_000), 100_000));
        assert_eq!(o.availability.as_deref(), Some("59m 17s"));
        // A title alone: no contractor.
        assert_eq!(offer_entry("TRESSLER [BP]*").contractor, None);
    }

    #[test]
    fn garbled_reward_box_falls_back_to_the_offer_list() {
        let offer = "JUNIOR | DIRECT MEDIUM HAUL | HUR-\n55k\nL2 FAITHFUL DREAM STATION → PORT\n\
             1h 56m\nTRESSLER [BP]*\nCOVALEX INDEPENDENT CONTRACTORS";
        let shot = |reward: &str| {
            RegionText {
            title: Some("Junior | DIRECT Medium Haul | HUR-L2 Faithful\nDream Station > Port Tressler [BP]*".into()),
            reward: Some(reward.into()),
            offer: Some(offer.into()),
            ..RegionText::default()
        }
        };
        // Reward drawn over the contractor: lost, and the contractor garbled.
        let c = contract(&shot("1h 56m\nCovalex Independent Contrect9Ea"));
        assert_eq!(c.reward, Some(55_000));
        assert_eq!(c.availability.as_deref(), Some("1h 56m"));
        assert_eq!(
            c.contractor.as_deref(),
            Some("Covalex Independent Contractors")
        );
        // Part of the amount read: the list disagrees, it wins.
        let c = contract(&shot("¤ 5,250\n1h 56m\nCovalex Independent Contractors"));
        assert_eq!(c.reward, Some(55_000));
        assert_eq!(
            c.contractor.as_deref(),
            Some("Covalex Independent Contractors")
        );
        // A good box is kept: exact amount, contractor as written.
        let c = contract(&shot("¤ 55,250\n1h 56m\nCovalex Independent Contractors"));
        assert_eq!(c.reward, Some(55_250));
        // The list of another contract is ignored.
        let other = RegionText {
            title: Some("Rookie | Small Haul | from Baijini Point [BP]*".into()),
            ..shot("1h 56m\nCovalex")
        };
        assert_eq!(contract(&other).reward, None);
        // Without the list, the box as read.
        let alone = RegionText {
            offer: None,
            ..shot("1h 56m\nCovalex Independent Contrect9Ea")
        };
        let c = contract(&alone);
        assert_eq!(c.reward, None);
        assert_eq!(
            c.contractor.as_deref(),
            Some("Covalex Independent Contrect9Ea")
        );
    }

    #[test]
    fn details_values() {
        let info = details_info(
            "You'll need room for 16 SCU cargo containers, and a 32 SCU ship.\n\
             Reputation Awarded: 2OO",
        );
        assert_eq!(info.max_box, Some(16));
        assert_eq!(info.rep, Some(200));
        assert_eq!(details_info("Max size will be 8 SCU.").max_box, Some(8));
        assert_eq!(details_info("used is 2 ScU, so").max_box, Some(2));
    }

    #[test]
    fn missing_collect_is_inferred() {
        let items = objective_items(&lines(
            "Deliver 0/32 SCU of Hydrogen to A.\nCollect Hydrogen from Yela.\n\
             Deliver 0/20 SCU of Medical Supplies to B.",
        ));
        let cargo = build_cargo(&items, "");
        assert_eq!(cargo[1].pickups, vec![Place::uncertain("Yela")]);
    }

    #[test]
    fn title_separators() {
        assert_eq!(
            title_text("Rookie I DIRECT Bulk Haul l Ruin\nStation > Baijini Point").as_deref(),
            Some("Rookie | DIRECT Bulk Haul | Ruin Station > Baijini Point")
        );
    }
}
