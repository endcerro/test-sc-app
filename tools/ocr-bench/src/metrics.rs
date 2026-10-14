//! Text accuracy metrics.

use sc_contracts::repair::clean_text;

/// Normalisation applied to both prediction and truth before comparing: per-line
/// whitespace collapsed, Unicode quotes folded, empty lines dropped, runs of 3+ dashes
/// (the `-----` rule under Red Wind headers) shortened to `---`.
pub fn normalize(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let mut l = clean_text(line);
        while l.contains("----") {
            l = l.replace("----", "---");
        }
        if !l.is_empty() {
            lines.push(l);
        }
    }
    lines.join("\n")
}

/// Levenshtein distance over chars, two-row DP.
pub fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Character error rate: edits / truth length (can exceed 1 with lots of junk).
pub fn cer(pred: &str, truth: &str) -> f64 {
    let p: Vec<char> = normalize(pred).chars().collect();
    let t: Vec<char> = normalize(truth).chars().collect();
    edit_distance(&p, &t) as f64 / t.len().max(1) as f64
}

/// Word error rate over whitespace-separated tokens.
pub fn wer(pred: &str, truth: &str) -> f64 {
    let p = normalize(pred);
    let t = normalize(truth);
    let pw: Vec<&str> = p.split_whitespace().collect();
    let tw: Vec<&str> = t.split_whitespace().collect();
    edit_distance(&pw, &tw) as f64 / tw.len().max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(edit_distance(&c("kitten"), &c("sitting")), 3);
        assert_eq!(edit_distance(&c(""), &c("abc")), 3);
        assert_eq!(edit_distance(&c("abc"), &c("abc")), 0);
    }

    #[test]
    fn rates() {
        assert!(cer("Deliver 0/4", "Deliver 0/4").abs() < 1e-9);
        assert!((cer("Deliver O/4", "Deliver 0/4") - 1.0 / 11.0).abs() < 1e-9);
        assert!((wer("a b c", "a x c") - 1.0 / 3.0).abs() < 1e-9);
        assert!(cer("x\n\n  y ", "x\ny").abs() < 1e-9);
        assert!(cer("------", "----------------------------").abs() < 1e-9);
    }
}
