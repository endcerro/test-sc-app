//! Flows built from the hand-written truth files: the sample shapes, the one-pickup-or-one-
//! drop-off invariant, and the round trip back to objective lines.

use std::path::PathBuf;

use sc_contracts::{Contract, HaulingContract, truth};

fn truth_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/truth")
}

fn truth_of(name: &str) -> HaulingContract {
    let text = std::fs::read_to_string(truth_dir().join(format!("{name}.txt")))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    truth::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// `commodity: pickups -> drop-off scu, ...` per flow.
fn shape(c: &Contract) -> Vec<String> {
    c.flows
        .iter()
        .map(|f| {
            let pickups: Vec<&str> = f.pickups.iter().map(|p| p.name.as_str()).collect();
            let drops: Vec<String> = f
                .dropoffs
                .iter()
                .map(|d| format!("{} {}", d.place.name, d.scu))
                .collect();
            format!(
                "{}: {} -> {}",
                f.display_commodity(),
                pickups.join(" + "),
                drops.join(", ")
            )
        })
        .collect()
}

#[test]
fn sample_shapes() {
    let c = Contract::from_objectives(&truth_of("06-covalex-wideforest-multidrop"));
    assert_eq!(
        shape(&c),
        [
            "Corundum: ARC-L1 Wide Forest Station -> Seraphim Station above Crusader 5, \
          Baijini Point above ArcCorp 6"
        ]
    );
    assert_eq!(c.total_scu(), 11);

    let c = Contract::from_objectives(&truth_of("07-covalex-lyria-multipickup"));
    assert_eq!(
        shape(&c),
        [
            "Aluminum (Ore): Shubin Mining Facility SAL-5 on Lyria + Shubin Mining Facility \
             SAL-2 on Lyria -> Baijini Point above ArcCorp 3",
            "Tungsten (Ore): Shubin Mining Facility SAL-5 on Lyria -> Baijini Point above \
             ArcCorp 4",
            "Corundum (Raw): Shubin Mining Facility SAL-2 on Lyria -> Baijini Point above \
             ArcCorp 2",
        ]
    );

    let c = Contract::from_objectives(&truth_of("09-redwind-yela-scroll"));
    assert_eq!(
        shape(&c),
        [
            "Ship Ammunition: ArcCorp's Yela facility -> Sunset Mesa on Pyro II 32, \
             Chawla's Beach on Pyro IV 32",
            "Hydrogen: ArcCorp's Yela facility -> Sunset Mesa on Pyro II 32, \
             Chawla's Beach on Pyro IV 32",
            "Medical Supplies: ArcCorp's Yela facility -> Sunset Mesa on Pyro II 20, \
             Chawla's Beach on Pyro IV 20",
        ]
    );
    assert!(
        c.flows
            .iter()
            .all(|f| f.pickups.iter().all(|p| !p.uncertain))
    );
}

/// Objective lines as comparable strings, ignoring order and the `?` flag.
fn lines(c: &HaulingContract) -> Vec<String> {
    let mut v: Vec<String> = c
        .cargo
        .iter()
        .map(|l| {
            let pickups: Vec<&str> = l.pickups.iter().map(|p| p.name.as_str()).collect();
            format!(
                "{} {} {} {}",
                l.scu,
                l.display_commodity(),
                pickups.join("+"),
                l.dropoff.name
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn every_truth_file_gives_valid_flows_and_round_trips() {
    let mut seen = 0;
    for entry in std::fs::read_dir(truth_dir()).unwrap_or_else(|e| panic!("{e}")) {
        let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
        if path.extension().is_none_or(|e| e != "txt") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        let t = truth::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let c = Contract::from_objectives(&t);
        assert!(c.flows.iter().all(|f| f.is_valid()), "{}", path.display());
        assert_eq!(
            c.total_scu(),
            t.cargo.iter().map(|l| l.scu).sum::<u32>(),
            "{}",
            path.display()
        );
        assert_eq!(lines(&c.to_objectives()), lines(&t), "{}", path.display());
        seen += 1;
    }
    assert!(seen >= 10, "only {seen} truth files found");
}
