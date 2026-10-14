//! Numbers and times shown on the Routes pane (pure, tested).

use sc_contracts::truth;

/// `525,500`.
pub fn money(v: u64) -> String {
    truth::thousands(v)
}

/// `<1 min`, `42 min`, `1h 05m`.
pub fn mins(m: f64) -> String {
    if m > 0.0 && m < 0.5 {
        "<1 min".into()
    } else if m >= 60.0 {
        let total = m.round() as u64;
        format!("{}h {:02}m", total / 60, total % 60)
    } else {
        format!("{m:.0} min")
    }
}

/// Number without needless decimals: 4 -> `4`, 1.5 -> `1.5`.
pub fn num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

pub fn opt_num(v: Option<f64>) -> String {
    v.map(num).unwrap_or_default()
}

/// Empty or unreadable input means "not set".
pub fn parse_opt(s: &str) -> Option<f64> {
    s.trim()
        .replace(',', "")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(mins(0.2), "<1 min");
        assert_eq!(mins(0.0), "0 min");
        assert_eq!(mins(42.4), "42 min");
        assert_eq!(mins(65.0), "1h 05m");
        assert_eq!(mins(119.7), "2h 00m");
        assert_eq!(num(4.0), "4");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(opt_num(None), "");
        assert_eq!(parse_opt(" 1,200 "), Some(1200.0));
        assert_eq!(parse_opt("-3"), None);
        assert_eq!(parse_opt(""), None);
        assert_eq!(money(525_500), "525,500");
    }
}
