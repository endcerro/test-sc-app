//! Scoring a parsed contract against truth.

use std::fmt::Write as _;

use sc_universe::normalize;

use crate::{Cargo, HaulingContract, Place};

/// Field-level comparison of a prediction with the truth.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    /// Scalar fields compared (present in truth or prediction) and whether they matched.
    pub fields: Vec<(&'static str, bool)>,
    /// Cargo lines matching the truth exactly (all four columns).
    pub cargo_exact: usize,
    pub cargo_pred: usize,
    pub cargo_truth: usize,
    /// Cargo columns matched after aligning predicted and true lines in order.
    pub cargo_field_hits: usize,
    /// `4 * max(cargo_pred, cargo_truth)`: missing or extra lines cost all their columns.
    pub cargo_field_total: usize,
    /// Human-readable list of what differs.
    pub mismatches: Vec<String>,
}

impl Score {
    /// Fraction of scalar fields and cargo columns that are right.
    pub fn accuracy(&self) -> f64 {
        let hits = self.fields.iter().filter(|f| f.1).count() + self.cargo_field_hits;
        let total = self.fields.len() + self.cargo_field_total;
        if total == 0 {
            1.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// F1 over exact cargo lines.
    pub fn cargo_f1(&self) -> f64 {
        if self.cargo_pred + self.cargo_truth == 0 {
            return 1.0;
        }
        2.0 * self.cargo_exact as f64 / (self.cargo_pred + self.cargo_truth) as f64
    }

    pub fn is_perfect(&self) -> bool {
        self.mismatches.is_empty()
    }
}

fn opt_eq<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
    a == b
}

fn text_eq(a: &Option<String>, b: &Option<String>) -> bool {
    a.as_deref().map(normalize) == b.as_deref().map(normalize)
}

fn place_key(p: &Place) -> (String, bool) {
    (normalize(&p.name), p.uncertain)
}

fn pickups_key(c: &Cargo) -> Vec<(String, bool)> {
    let mut v: Vec<_> = c.pickups.iter().map(place_key).collect();
    v.sort();
    v
}

/// Which of the 4 cargo columns match: scu, commodity (with the `[!]` marker), pickups,
/// dropoff.
fn cargo_columns(p: &Cargo, t: &Cargo) -> [bool; 4] {
    [
        p.scu == t.scu,
        p.restricted == t.restricted && normalize(&p.commodity) == normalize(&t.commodity),
        pickups_key(p) == pickups_key(t),
        place_key(&p.dropoff) == place_key(&t.dropoff),
    ]
}

fn hits(p: &Cargo, t: &Cargo) -> usize {
    cargo_columns(p, t).iter().filter(|b| **b).count()
}

fn cargo_line(c: &Cargo) -> String {
    let pickups: Vec<String> = c.pickups.iter().map(ToString::to_string).collect();
    format!(
        "{} SCU | {} | {} | {}",
        c.scu,
        c.display_commodity(),
        pickups.join(" + "),
        c.dropoff
    )
}

pub fn score(pred: &HaulingContract, truth: &HaulingContract) -> Score {
    let mut s = Score::default();
    let mut field = |name: &'static str, ok: bool, present: bool, p: String, t: String| {
        if present {
            s.fields.push((name, ok));
            if !ok {
                s.mismatches.push(format!("{name}: got `{p}`, want `{t}`"));
            }
        }
    };
    let show = |v: Option<String>| v.unwrap_or_else(|| "-".to_owned());
    field(
        "title",
        text_eq(&pred.title, &truth.title),
        pred.title.is_some() || truth.title.is_some(),
        show(pred.title.clone()),
        show(truth.title.clone()),
    );
    field(
        "reward",
        opt_eq(&pred.reward, &truth.reward),
        pred.reward.is_some() || truth.reward.is_some(),
        show(pred.reward.map(|v| v.to_string())),
        show(truth.reward.map(|v| v.to_string())),
    );
    field(
        "contractor",
        text_eq(&pred.contractor, &truth.contractor),
        pred.contractor.is_some() || truth.contractor.is_some(),
        show(pred.contractor.clone()),
        show(truth.contractor.clone()),
    );
    field(
        "availability",
        text_eq(&pred.availability, &truth.availability),
        pred.availability.is_some() || truth.availability.is_some(),
        show(pred.availability.clone()),
        show(truth.availability.clone()),
    );
    field(
        "max_box",
        opt_eq(&pred.max_box, &truth.max_box),
        pred.max_box.is_some() || truth.max_box.is_some(),
        show(pred.max_box.map(|v| v.to_string())),
        show(truth.max_box.map(|v| v.to_string())),
    );
    field(
        "rep",
        opt_eq(&pred.rep, &truth.rep),
        pred.rep.is_some() || truth.rep.is_some(),
        show(pred.rep.map(|v| v.to_string())),
        show(truth.rep.map(|v| v.to_string())),
    );

    // Exact multiset matching.
    s.cargo_pred = pred.cargo.len();
    s.cargo_truth = truth.cargo.len();
    let mut used = vec![false; truth.cargo.len()];
    for p in &pred.cargo {
        if let Some(i) = (0..truth.cargo.len()).find(|&i| !used[i] && hits(p, &truth.cargo[i]) == 4)
        {
            used[i] = true;
            s.cargo_exact += 1;
        }
    }

    // Order-preserving alignment maximising matched columns (gaps cost nothing here; they
    // are paid through `cargo_field_total`).
    let (n, m) = (pred.cargo.len(), truth.cargo.len());
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            let diag = dp[i - 1][j - 1] + hits(&pred.cargo[i - 1], &truth.cargo[j - 1]);
            dp[i][j] = diag.max(dp[i - 1][j]).max(dp[i][j - 1]);
        }
    }
    s.cargo_field_hits = dp[n][m];
    s.cargo_field_total = 4 * n.max(m);

    // Walk the alignment back to describe differences.
    let (mut i, mut j) = (n, m);
    let mut notes = Vec::new();
    while i > 0 || j > 0 {
        if i > 0
            && j > 0
            && dp[i][j] == dp[i - 1][j - 1] + hits(&pred.cargo[i - 1], &truth.cargo[j - 1])
            && hits(&pred.cargo[i - 1], &truth.cargo[j - 1]) > 0
        {
            let (p, t) = (&pred.cargo[i - 1], &truth.cargo[j - 1]);
            if hits(p, t) < 4 {
                notes.push(format!(
                    "cargo {j}: got `{}`, want `{}`",
                    cargo_line(p),
                    cargo_line(t)
                ));
            }
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j] == dp[i][j - 1]) {
            notes.push(format!(
                "cargo {j}: missing `{}`",
                cargo_line(&truth.cargo[j - 1])
            ));
            j -= 1;
        } else {
            notes.push(format!("cargo: extra `{}`", cargo_line(&pred.cargo[i - 1])));
            i -= 1;
        }
    }
    notes.reverse();
    s.mismatches.extend(notes);
    s
}

/// Multi-line report of a score, for logs and benchmark summaries.
pub fn report(s: &Score) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "accuracy {:.3}, cargo F1 {:.3} ({}/{} exact)",
        s.accuracy(),
        s.cargo_f1(),
        s.cargo_exact,
        s.cargo_truth
    );
    for m in &s.mismatches {
        let _ = writeln!(out, "  - {m}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::truth;

    fn contract(s: &str) -> HaulingContract {
        truth::parse(&format!("#HAULING v1\n{s}")).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn perfect_match() {
        let t = contract("REWARD: 1,000\nCARGO: 4 SCU | Gold | A on B | C\n");
        let s = score(&t, &t);
        assert!(s.is_perfect());
        assert!((s.accuracy() - 1.0).abs() < 1e-9);
        assert!((s.cargo_f1() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn partial_and_missing() {
        let t = contract(
            "REWARD: 1,000\nREP: 100\nCARGO: 4 SCU | Gold | A | C\nCARGO: 2 SCU | Neon | A | C\n",
        );
        let p = contract("REWARD: 1,000\nCARGO: 4 SCU | Gold | A | D\n");
        let s = score(&p, &t);
        assert_eq!(s.fields, vec![("reward", true), ("rep", false)]);
        assert_eq!(s.cargo_exact, 0);
        assert_eq!(s.cargo_field_hits, 3);
        assert_eq!(s.cargo_field_total, 8);
        assert_eq!(s.mismatches.len(), 3, "{:?}", s.mismatches);
    }

    #[test]
    fn normalised_text_comparison() {
        let t = contract("CONTRACTOR: Red Wind Linehaul\n");
        let p = contract("CONTRACTOR: red wind  linehaul\n");
        assert!(score(&p, &t).is_perfect());
    }
}
