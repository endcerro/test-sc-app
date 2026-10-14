//! Several contracts as `#HAULING v1` text, to paste in or export: one header, then each
//! contract starting at its `CONTRACT:` line. The header is optional when pasting.

use sc_contracts::{HaulingContract, truth};

/// A line of pasted text that could not be read (1-based; 0 = not tied to a line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub line: usize,
    pub message: String,
}

/// Reads every contract of `text`. Contracts with an error are skipped and reported.
pub fn parse(text: &str) -> (Vec<HaulingContract>, Vec<Issue>) {
    let mut blocks: Vec<(usize, Vec<&str>)> = Vec::new();
    let mut issues = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("CONTRACT:") {
            blocks.push((i, vec![line]));
        } else if let Some((_, lines)) = blocks.last_mut() {
            lines.push(line);
        } else if !(t.is_empty() || t.starts_with('#')) {
            issues.push(Issue {
                line: i + 1,
                message: format!("before the first CONTRACT: line: `{t}`"),
            });
        }
    }
    let mut contracts = Vec::new();
    for (start, lines) in blocks {
        let block = format!("{}\n{}", truth::HEADER, lines.join("\n"));
        match truth::parse(&block) {
            Ok(c) => contracts.push(c),
            Err(truth::TruthError::Line { line, message }) => issues.push(Issue {
                // `line` counts the added header line.
                line: start + line - 1,
                message,
            }),
            Err(e) => issues.push(Issue {
                line: start + 1,
                message: e.to_string(),
            }),
        }
    }
    if contracts.is_empty() && issues.is_empty() && !text.trim().is_empty() {
        issues.push(Issue {
            line: 0,
            message: "no CONTRACT: line found".to_owned(),
        });
    }
    (contracts, issues)
}

/// The contracts as one `#HAULING v1` text.
pub fn write(contracts: &[HaulingContract]) -> String {
    let mut out = format!("{}\n", truth::HEADER);
    for c in contracts {
        let text = truth::serialize(c);
        out.push('\n');
        out.push_str(text.trim_start_matches(truth::HEADER).trim_start());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "#HAULING v1

CONTRACT: Junior Rank - Small Cargo Haul
REWARD: 59,750 aUEC
CARGO: 3 SCU | Aluminum (Ore) | SAL-5 + SAL-2 | Baijini Point

# a comment
CONTRACT: Second
REWARD: 1,000 aUEC
CARGO: 2 SCU | Gold | A | B
";

    #[test]
    fn round_trip() {
        let (contracts, issues) = parse(TWO);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(contracts.len(), 2);
        assert_eq!(contracts[0].cargo[0].pickups.len(), 2);
        assert_eq!(contracts[1].title.as_deref(), Some("Second"));
        let again = parse(&write(&contracts));
        assert_eq!(again, (contracts, Vec::new()));
    }

    #[test]
    fn issues_point_at_lines() {
        let text = "stray\nCONTRACT: A\nREWARD: x\nCONTRACT: B\nCARGO: 1 SCU | Gold\n";
        let (contracts, issues) = parse(text);
        assert!(contracts.is_empty());
        let lines: Vec<usize> = issues.iter().map(|i| i.line).collect();
        assert_eq!(lines, [1, 3, 5]);
        assert_eq!(parse("hello").1.len(), 1);
        assert_eq!(parse("  ").1.len(), 0);
    }
}
