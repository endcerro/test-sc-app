//! Parsing the exact region transcripts (`samples/truth/regions/<stem>/*.txt`) must
//! reproduce the hand-written `#HAULING` truth: with perfect OCR the parser is perfect.

use std::path::{Path, PathBuf};

use sc_contracts::{RegionText, parse, score, truth};

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/truth")
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn regions(stem: &str) -> RegionText {
    let dir = samples().join("regions").join(stem);
    RegionText {
        title: read(&dir.join("title.txt")),
        reward: read(&dir.join("reward.txt")),
        details: read(&dir.join("details.txt")),
        objectives: read(&dir.join("objectives.txt"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
    }
}

fn truth_for(name: &str) -> sc_contracts::HaulingContract {
    let text = read(&samples().join(format!("{name}.txt")))
        .unwrap_or_else(|| panic!("no truth for {name}"));
    truth::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn check(name: &str, shots: &[RegionText]) {
    let pred = parse::contract_from_shots(shots);
    let s = score(&pred, &truth_for(name));
    assert!(
        s.is_perfect(),
        "{name}:\n{}\n{}",
        sc_contracts::score::report(&s),
        truth::serialize(&pred)
    );
}

#[test]
fn single_shots_parse_to_truth() {
    for stem in [
        "01-dead-saints-riker",
        "02-ling-bueno-rayari",
        "03-redwind-ruin-baijini",
        "04-redwind-orbituary-teasa",
        "05-dead-saints-ruin-grimhex",
        "06-covalex-wideforest-multidrop",
        "07-covalex-lyria-multipickup",
        "08-covalex-baijini-riker",
        "09-redwind-yela-scroll-1",
    ] {
        check(stem, &[regions(stem)]);
    }
}

#[test]
fn scrolled_shots_merge_to_truth() {
    check(
        "09-redwind-yela-scroll",
        &[
            regions("09-redwind-yela-scroll-1"),
            regions("09-redwind-yela-scroll-2"),
        ],
    );
}
