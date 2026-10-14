//! Travel times and whole plans on the trimmed real universe fixture, every route checked
//! from its own steps.

use std::collections::HashMap;
use std::path::PathBuf;

use sc_contracts::{Contract, snap, truth};
use sc_data::UniverseSource;
use sc_planner::{ContractStatus, Goal, LegKind, PlanError, PlanOutput, Settings, Ship, leg, plan};
use sc_universe::{LocationId, Universe};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn universe() -> Universe {
    sc_data::ScunpackedSource::from_dir(root().join("crates/sc-data/tests/fixtures/scunpacked"))
        .load()
        .unwrap_or_else(|e| panic!("{e}"))
        .universe
}

fn resolve(u: &Universe) -> impl Fn(&str) -> Option<LocationId> + '_ {
    |name| snap::resolve_place(u, name).map(|m| m.id)
}

fn id(u: &Universe, name: &str) -> LocationId {
    resolve(u)(name).unwrap_or_else(|| panic!("{name} not in the fixture"))
}

/// Contracts from `#HAULING v1` text.
fn contracts(text: &str) -> Vec<Contract> {
    text.split("\n\n")
        .filter(|block| block.contains("CONTRACT:"))
        .map(|block| {
            let c = truth::parse(&format!("#HAULING v1\n{block}"))
                .unwrap_or_else(|e| panic!("{e}: {block}"));
            Contract::from_objectives(&c)
        })
        .collect()
}

const BOARD: &str = "
CONTRACT: Lorville to Area18
REWARD: 60,000 aUEC
CARGO: 20 SCU | Gold | Lorville | Area18

CONTRACT: Hurston local
REWARD: 25,000 aUEC
CARGO: 10 SCU | Waste | Everus Harbor | Covalex Distribution Centre S1DC06
CARGO: 6 SCU | Waste | Everus Harbor | Lorville

CONTRACT: Interstellar
REWARD: 300,000 aUEC
CARGO: 40 SCU | Ammonia | Ruin Station above Terminus | Area18

CONTRACT: Two pickups
REWARD: 45,000 aUEC
AVAILABILITY: 0h 9m
CARGO: 12 SCU | Iron | Lorville + HUR-L1 Green Glade Station | Area18

CONTRACT: Unknown place
REWARD: 90,000 aUEC
CARGO: 5 SCU | Gold | Nowhere Yard | Area18
";

/// Every route: within the contract cap, under the hard maximum, each line fully loaded
/// before it's delivered, everything delivered, cargo totals consistent.
fn check(out: &PlanOutput, contracts: &[Contract], hard_max: Option<f64>) {
    assert_eq!(
        out.contracts.len(),
        contracts.len(),
        "every contract gets a status"
    );
    for r in &out.routes {
        assert!(r.contracts.len() <= out.max_contracts);
        let mut loaded: HashMap<(usize, usize), f64> = HashMap::new();
        let mut delivered: HashMap<(usize, usize), f64> = HashMap::new();
        let mut cargo = 0.0;
        for b in &r.on_board_at_start {
            *loaded.entry((b.contract, b.line)).or_default() += b.scu;
            cargo += b.scu;
        }
        for st in &r.steps {
            for d in &st.delivered {
                let k = (d.contract, d.line);
                let total: u32 = contracts[d.contract - 1].flows[d.line - 1].scu();
                let got = loaded.get(&k).copied().unwrap_or(0.0);
                assert!(
                    (got - f64::from(total)).abs() < 0.01,
                    "route {} delivers {} before loading all of it",
                    r.rank,
                    d.commodity
                );
                *delivered.entry(k).or_default() += d.scu;
                cargo -= d.scu;
            }
            for p in &st.picked {
                *loaded.entry((p.contract, p.line)).or_default() += p.scu;
                cargo += p.scu;
            }
            assert!(
                (cargo - st.cargo_after).abs() < 0.05,
                "cargo total drifts at step {}",
                st.n
            );
            if let Some(m) = hard_max {
                assert!(st.cargo_after <= m + 1e-6, "over the hard maximum");
            }
        }
        for &c in &r.contracts {
            for (i, f) in contracts[c - 1].flows.iter().enumerate() {
                let got = delivered.get(&(c, i + 1)).copied().unwrap_or(0.0);
                assert!(
                    (got - f64::from(f.scu())).abs() < 0.01,
                    "route {} delivers {got} of {} SCU {} of contract {c}",
                    r.rank,
                    f.scu(),
                    f.commodity
                );
            }
        }
    }
}

#[test]
fn travel_legs() {
    let u = universe();
    let (ship, s) = (Ship::default(), Settings::default());
    let leg = |a: &str, b: &str| leg(&u, &ship, &s, id(&u, a), id(&u, b));

    let same = leg("Lorville", "Lorville");
    assert_eq!((same.kind, same.minutes), (LegKind::SamePlace, 0.0));

    let hop = leg("Lorville", "Everus Harbor");
    assert_eq!(hop.kind, LegKind::SameBody);
    assert_eq!(hop.minutes, s.hop_minutes);
    assert!(hop.estimated);

    let qt = leg("Lorville", "Area18");
    assert_eq!(qt.kind, LegKind::Quantum);
    let gm = qt.distance.map_or(0.0, |d| d.gm());
    assert!(gm > 1.0, "Hurston to ArcCorp is gigametres: {gm}");
    assert!(qt.minutes > ship.spool_seconds / 60.0);
    assert!((qt.minutes - qt.quantum_minutes).abs() < 1e-9);

    let jump = leg("Area18", "Ruin Station");
    assert_eq!((jump.kind, jump.jumps), (LegKind::Jump, 1));
    assert!(jump.minutes >= jump.quantum_minutes + s.stop_minutes - 1e-9);

    let missing = leg("Area18", "Nyx");
    assert_eq!(missing.kind, LegKind::Missing);
    assert!(missing.estimated && missing.minutes > 0.0);
}

#[test]
fn plans_the_board_for_every_goal() {
    let u = universe();
    let board = contracts(BOARD);
    for goal in [
        Goal::RatePerHour { min_minutes: 20.0 },
        Goal::MaxReward {
            budget_minutes: 60.0,
        },
        Goal::CompleteAll,
    ] {
        let s = Settings {
            goal,
            ..Settings::new("Lorville")
        };
        let out =
            plan(&board, &u, resolve(&u), &Ship::default(), &s).unwrap_or_else(|e| panic!("{e}"));
        check(&out, &board, None);
        assert!(!out.routes.is_empty(), "{goal:?}: {out:#?}");
        assert!(
            matches!(&out.contracts[4].status, ContractStatus::LeftOut(why) if why.contains("Nowhere Yard")),
            "{:?}",
            out.contracts[4].status
        );
        if let Goal::MaxReward { budget_minutes } = goal {
            assert!(
                out.routes
                    .iter()
                    .all(|r| r.total_minutes <= budget_minutes + 1e-6)
            );
        }
        if goal == Goal::CompleteAll {
            assert_eq!(out.routes.len(), 1);
            assert_eq!(out.routes[0].contracts, [1, 2, 3, 4]);
            let r = &out.routes[0];
            assert!(
                r.warnings
                    .iter()
                    .any(|w| w.contains("split between its 2 pickups"))
            );
            assert!(r.warnings.iter().any(|w| w.contains("accept it first")));
            assert!(r.warnings.iter().any(|w| w.contains("same-planet hop")));
            assert!(r.steps.iter().any(|st| st.travel.kind == LegKind::Jump));
        }
    }
}

#[test]
fn loaded_cargo_needs_no_pickup() {
    let u = universe();
    let mut board = contracts(BOARD);
    board.truncate(1);
    board[0].flows[0].loaded = true;
    let out = plan(
        &board,
        &u,
        resolve(&u),
        &Ship::default(),
        &Settings::new("Everus Harbor"),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    check(&out, &board, None);
    let r = &out.routes[0];
    assert_eq!(r.on_board_at_start.len(), 1);
    assert_eq!(r.steps.len(), 1, "straight to Area18: {:#?}", r.steps);
    assert_eq!(r.steps[0].location, "Area18");
}

#[test]
fn hard_maximum_leaves_out_oversized_contracts() {
    let u = universe();
    let board = contracts(BOARD);
    let ship = Ship {
        hard_max: Some(30.0),
        capacity_estimate: Some(24.0),
        ..Ship::default()
    };
    let out = plan(&board, &u, resolve(&u), &ship, &Settings::new("Lorville"))
        .unwrap_or_else(|e| panic!("{e}"));
    check(&out, &board, Some(30.0));
    assert!(
        matches!(&out.contracts[2].status, ContractStatus::LeftOut(why) if why.contains("needs multiple trips")),
        "{:?}",
        out.contracts[2].status
    );
    assert!(out.routes.iter().all(|r| !r.contracts.contains(&3)));
}

#[test]
fn contracts_that_cannot_be_planned_say_why() {
    let u = universe();
    let mut board = contracts(BOARD);
    board[0].reward = None;
    board[1].flows[0].dropoffs[0].scu = 0;
    let out = plan(
        &board,
        &u,
        resolve(&u),
        &Ship::default(),
        &Settings::new("Lorville"),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let why = |i: usize| match &out.contracts[i].status {
        ContractStatus::LeftOut(why) => why.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(why(0), "no readable reward");
    assert!(why(1).contains("cargo amount missing"), "{}", why(1));
}

#[test]
fn start_must_be_known() {
    let u = universe();
    let board = contracts(BOARD);
    let run = |start: &str| {
        plan(
            &board,
            &u,
            resolve(&u),
            &Ship::default(),
            &Settings::new(start),
        )
    };
    assert_eq!(run(" ").err(), Some(PlanError::NoStart));
    assert!(matches!(
        run("Nowhere Station XYZ"),
        Err(PlanError::UnknownStart(_))
    ));
}

/// The sample contracts mostly name places outside the trimmed fixture: they must all get
/// a status, and whatever routes come out must hold.
#[test]
fn truth_samples() {
    let u = universe();
    let dir = root().join("samples/truth");
    let mut board = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{e}")) {
        let path = e.unwrap_or_else(|e| panic!("{e}")).path();
        if path.extension().is_some_and(|x| x == "txt") {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
            let c = truth::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            board.push(Contract::from_objectives(&c));
        }
    }
    assert!(board.len() >= 5);
    let out = plan(
        &board,
        &u,
        resolve(&u),
        &Ship::default(),
        &Settings::new("Area18"),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    check(&out, &board, None);
}
