//! The hand-written truth files under `samples/truth` all parse and round-trip.

use std::path::PathBuf;

use sc_contracts::truth;

fn truth_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/truth")
}

#[test]
fn all_truth_files_round_trip() {
    let mut seen = 0;
    let entries = std::fs::read_dir(truth_dir()).unwrap_or_else(|e| panic!("{e}"));
    for entry in entries {
        let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
        if path.extension().is_none_or(|e| e != "txt") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = truth::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!c.cargo.is_empty(), "{}: no cargo", path.display());
        assert!(c.reward.is_some(), "{}: no reward", path.display());
        let again = truth::parse(&truth::serialize(&c)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c, again, "{}", path.display());
        seen += 1;
    }
    assert!(seen >= 10, "only {seen} truth files found");
}
