//! Text shown on contract cards (pure, tested; painting is in `card.rs`).

use sc_contracts::truth;

use crate::contracts::Entry;

/// `525,500` (empty when unknown).
pub fn reward(reward: Option<u64>) -> String {
    reward.map(truth::thousands).unwrap_or_default()
}

/// Digits of a typed number (`525,500 aUEC` -> 525500); `None` when there are none.
pub fn parse_number<T: std::str::FromStr>(text: &str) -> Option<T> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `44 SCU · 3 lines · captured 2026-10-14 04:17 UTC · 2 screenshots · edited`.
pub fn footer(e: &Entry) -> String {
    let lines = e.contract.flows.len();
    let mut parts = vec![
        format!("{} SCU", e.contract.total_scu()),
        format!("{lines} line{}", if lines == 1 { "" } else { "s" }),
        format!("captured {}", utc_minutes(e.id)),
    ];
    if e.shots.len() > 1 {
        parts.push(format!("{} screenshots", e.shots.len()));
    }
    if e.edited {
        parts.push("edited".to_owned());
    }
    parts.join(" \u{b7} ")
}

/// Unix milliseconds -> `YYYY-MM-DD HH:MM UTC`.
pub fn utc_minutes(unix_ms: u64) -> String {
    let secs = unix_ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (y, m, d) = civil_from_days(days);
    let (h, min) = (secs % 86_400 / 3600, secs % 3600 / 60);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02} UTC")
}

/// Days since 1970-01-01 -> (year, month, day), proleptic Gregorian (H. Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use sc_contracts::{Contract, DropOff, Flow, Place};

    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(reward(Some(525_500)), "525,500");
        assert_eq!(reward(None), "");
        assert_eq!(parse_number::<u64>("525,500 aUEC"), Some(525_500));
        assert_eq!(parse_number::<u32>(" 16 "), Some(16));
        assert_eq!(parse_number::<u32>(""), None);
        assert_eq!(parse_number::<u32>("99999999999"), None);
    }

    #[test]
    fn dates() {
        assert_eq!(utc_minutes(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_minutes(951_782_400_000), "2000-02-29 00:00 UTC");
        assert_eq!(utc_minutes(1_792_008_239_027), "2026-10-14 20:03 UTC");
    }

    #[test]
    fn footer_text() {
        let flow = |scu| Flow {
            commodity: "Gold".into(),
            pickups: vec![Place::new("A")],
            dropoffs: vec![DropOff {
                place: Place::new("B"),
                scu,
            }],
            ..Flow::default()
        };
        let mut e = Entry {
            id: 0,
            contract: Contract {
                flows: vec![flow(32), flow(20)],
                ..Contract::default()
            },
            shots: vec![Default::default(), Default::default()],
            corrections: Vec::new(),
            edited: true,
            stale: false,
        };
        assert_eq!(
            footer(&e),
            "52 SCU \u{b7} 2 lines \u{b7} captured 1970-01-01 00:00 UTC \u{b7} 2 screenshots \
             \u{b7} edited"
        );
        e.contract.flows.truncate(1);
        e.shots.truncate(1);
        e.edited = false;
        assert_eq!(
            footer(&e),
            "32 SCU \u{b7} 1 line \u{b7} captured 1970-01-01 00:00 UTC"
        );
    }
}
