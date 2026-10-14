//! Behaviour tests on a small hand-built universe.

use sc_universe::{
    Alias, AliasKind, Amenity, BuildWarning, DataMeta, DistanceError, GameUuid, JumpSpec, Leg,
    LocationKind, LookupOptions, NodeSpec, Universe, UniverseBuilder, Vec3,
};

fn uuid(n: u128) -> GameUuid {
    GameUuid(uuid::Uuid::from_u128(n))
}

fn node(
    n: u128,
    parent: Option<u128>,
    name: &str,
    kind: LocationKind,
    pos: Option<Vec3>,
) -> NodeSpec {
    let mut s = NodeSpec::new(uuid(n), name, kind);
    s.parent = parent.map(uuid);
    s.position = pos;
    s
}

/// Stanton: star(1) > Hurston(2) > Lorville(3), Everus Harbor(4), Arial(5) > Outpost w/o pos(6)
///          star > HUR-L1(7), Stanton-Pyro jump point(8), RSI Event Platform(9) on Hurston
/// Pyro:    star(100) > Pyro-Stanton jump point(101), Bloom(102) > RSI Event Platform(103)
fn sample() -> (Universe, Vec<BuildWarning>) {
    let mut b = UniverseBuilder::new();
    b.add_system("stanton", "Stanton")
        .add_system("pyro", "Pyro");

    let mut star = node(1, None, "Stanton", LocationKind::Star, Some(Vec3::ZERO));
    star.system = Some("stanton".into());
    b.add_node(star);
    b.add_node(node(
        2,
        Some(1),
        "Hurston",
        LocationKind::Planet,
        Some(Vec3::new(12.85e9, 0.0, 0.0)),
    ));
    let mut lorville = node(
        3,
        Some(2),
        "Lorville",
        LocationKind::City,
        Some(Vec3::new(12.85e9 + 1.0e6, 0.0, 0.0)),
    );
    lorville.amenities.push(Amenity {
        key: "Commodity Trading - Freight Elevator".into(),
        display: "Commodity Trading".into(),
    });
    b.add_node(lorville);
    b.add_node(node(
        4,
        Some(2),
        "Everus Harbor",
        LocationKind::Station,
        Some(Vec3::new(12.85e9 + 1.15e6, 0.0, 0.0)),
    ));
    b.add_node(node(
        5,
        Some(2),
        "Arial",
        LocationKind::Moon,
        Some(Vec3::new(12.85e9, 52.66e6, 0.0)),
    ));
    b.add_node(node(6, Some(5), "HDMS-Bezdek", LocationKind::Outpost, None));
    b.add_node(node(
        7,
        Some(1),
        "HUR-L1 Green Glade Station",
        LocationKind::Station,
        Some(Vec3::new(11.56e9, 0.0, 0.0)),
    ));
    b.add_node(node(
        8,
        Some(1),
        "Stanton-Pyro Jump Point",
        LocationKind::JumpPoint,
        Some(Vec3::new(0.0, 28.0e9, 0.0)),
    ));
    b.add_node(node(
        9,
        Some(2),
        "RSI Event Platform",
        LocationKind::PointOfInterest,
        Some(Vec3::new(12.85e9, 1.0e6, 0.0)),
    ));

    let mut pyro = node(100, None, "Pyro", LocationKind::Star, Some(Vec3::ZERO));
    pyro.system = Some("pyro".into());
    b.add_node(pyro);
    b.add_node(node(
        101,
        Some(100),
        "Pyro-Stanton Jump Point",
        LocationKind::JumpPoint,
        Some(Vec3::new(-37.6e9, 37.4e9, 0.0)),
    ));
    b.add_node(node(
        102,
        Some(100),
        "Bloom",
        LocationKind::Planet,
        Some(Vec3::new(-8.9e9, -15.4e9, 0.0)),
    ));
    b.add_node(node(
        103,
        Some(102),
        "RSI Event Platform",
        LocationKind::PointOfInterest,
        Some(Vec3::new(-8.9e9, -15.4e9, 1.0e6)),
    ));

    b.add_jump(JumpSpec {
        a: uuid(8),
        b: uuid(101),
        fuel_cost: Some(0.24),
    });
    b.build(DataMeta::default())
}

fn id_of(u: &Universe, name: &str) -> sc_universe::LocationId {
    let ids = u.find_exact(name);
    assert_eq!(ids.len(), 1, "expected exactly one {name}");
    ids[0].id
}

#[test]
fn tree_shape() {
    let (u, warnings) = sample();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(u.systems().count(), 2);

    let lorville = id_of(&u, "Lorville");
    let names: Vec<&str> = u.ancestors(lorville).map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Hurston", "Stanton", "Stanton"]);
    assert_eq!(
        u.parent(lorville).map(|l| l.kind),
        Some(LocationKind::Planet)
    );
    assert_eq!(u.path_string(lorville), "Stanton › Hurston › Lorville");
    assert_eq!(
        u.get(lorville).system,
        u.system("stanton")
            .map(|s| s.id)
            .unwrap_or_else(|| panic!())
    );
}

#[test]
fn nearest_body() {
    let (u, _) = sample();
    let l1 = id_of(&u, "HUR-L1 Green Glade Station");
    assert!(u.nearest_body(l1).is_none());
    let outpost = id_of(&u, "HDMS-Bezdek");
    assert_eq!(
        u.nearest_body(outpost).map(|l| l.name.as_str()),
        Some("Arial")
    );
}

#[test]
fn inherits_missing_positions() {
    let (u, _) = sample();
    let outpost = u.get(id_of(&u, "HDMS-Bezdek"));
    let arial = id_of(&u, "Arial");
    match outpost.position {
        sc_universe::Position::Inherited { from, .. } => assert_eq!(from, arial),
        other => panic!("expected inherited position, got {other:?}"),
    }
}

#[test]
fn same_system_distance() {
    let (u, _) = sample();
    let d = u
        .distance(id_of(&u, "Lorville"), id_of(&u, "Everus Harbor"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!((d.km() - 150.0).abs() < 1.0, "{d}");
}

#[test]
fn cross_system_distance_needs_route() {
    let (u, _) = sample();
    let lorville = id_of(&u, "Lorville");
    let bloom = id_of(&u, "Bloom");
    assert_eq!(
        u.distance(lorville, bloom),
        Err(DistanceError::DifferentSystems(lorville, bloom))
    );
    let route = u.route(lorville, bloom).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(route.jumps(), 1);
    assert_eq!(route.legs.len(), 3);
    assert!(matches!(
        route.legs[1],
        Leg::Jump {
            fuel_cost: Some(_),
            ..
        }
    ));
    let direct_legs = u
        .distance(lorville, id_of(&u, "Stanton-Pyro Jump Point"))
        .unwrap_or_else(|e| panic!("{e}"))
        + u.distance(id_of(&u, "Pyro-Stanton Jump Point"), bloom)
            .unwrap_or_else(|e| panic!("{e}"));
    assert!((route.total.metres() - direct_legs.metres()).abs() < 1.0);
}

#[test]
fn same_system_route_is_single_leg() {
    let (u, _) = sample();
    let r = u
        .route(id_of(&u, "Lorville"), id_of(&u, "Arial"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r.jumps(), 0);
    assert_eq!(r.legs.len(), 1);
}

#[test]
fn fuzzy_lookup_handles_ocr_noise() {
    let (u, _) = sample();
    let best = u.resolve("LORVILE", LookupOptions::default());
    assert_eq!(best.map(|l| l.name.as_str()), Some("Lorville"));
    let best = u.resolve("Everus Harb0r", LookupOptions::default());
    assert_eq!(best.map(|l| l.name.as_str()), Some("Everus Harbor"));
    assert!(u.resolve("zzzzzz", LookupOptions::default()).is_none());
}

#[test]
fn scoped_lookup_disambiguates_duplicates() {
    let (u, _) = sample();
    assert_eq!(u.find_exact("RSI Event Platform").len(), 2);
    let pyro = u.system("pyro").map(|s| s.id);
    let m = u.find_fuzzy(
        "RSI Event Platform",
        LookupOptions {
            scope: pyro,
            ..Default::default()
        },
    );
    assert_eq!(m.len(), 1);
    assert_eq!(u.parent(m[0].id).map(|p| p.name.as_str()), Some("Bloom"));
}

#[test]
fn trading_locations() {
    let (u, _) = sample();
    let names: Vec<&str> = u.trading_locations().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Lorville"]);
}

#[test]
fn warnings_for_bad_data() {
    let mut b = UniverseBuilder::new();
    let mut star = node(1, None, "Star", LocationKind::Star, Some(Vec3::ZERO));
    star.system = Some("test".into());
    b.add_node(star);
    b.add_node(node(1, None, "Dup", LocationKind::Star, None));
    b.add_node(node(2, Some(999), "Dangling", LocationKind::Outpost, None));
    b.add_node(node(3, Some(4), "CycleA", LocationKind::Outpost, None));
    b.add_node(node(4, Some(3), "CycleB", LocationKind::Outpost, None));
    b.add_jump(JumpSpec {
        a: uuid(1),
        b: uuid(777),
        fuel_cost: None,
    });
    let (u, w) = b.build(DataMeta::default());

    assert!(
        w.iter()
            .any(|w| matches!(w, BuildWarning::DuplicateUuid { .. }))
    );
    assert!(
        w.iter()
            .any(|w| matches!(w, BuildWarning::DanglingParent { .. }))
    );
    assert!(w.iter().any(|w| matches!(w, BuildWarning::Cycle { .. })));
    assert!(
        w.iter()
            .any(|w| matches!(w, BuildWarning::JumpEndpointMissing { .. }))
    );
    // Dangling and cyclic nodes have no system code of their own: dropped.
    assert!(w.iter().any(|w| matches!(w, BuildWarning::NoSystem { .. })));
    assert!(u.find_exact("Dangling").is_empty());
    // System was auto-declared with a capitalised name.
    assert!(u.system("Test").is_some());
    assert!(u.jump_links().is_empty());
}

#[test]
fn aliases_resolve_to_their_target() {
    let mut b = UniverseBuilder::new();
    let mut star = node(1, None, "Stanton", LocationKind::Star, Some(Vec3::ZERO));
    star.system = Some("stanton".into());
    b.add_node(star);
    b.add_node(node(
        2,
        Some(1),
        "ArcCorp",
        LocationKind::Planet,
        Some(Vec3::new(18.5e9, 0.0, 0.0)),
    ));
    b.add_node(node(
        3,
        Some(2),
        "Area18",
        LocationKind::City,
        Some(Vec3::new(18.5e9 + 0.8e6, 0.0, 0.0)),
    ));
    // A location whose own name equals another location's alias.
    b.add_node(node(
        4,
        Some(1),
        "Teasa Spaceport",
        LocationKind::Station,
        Some(Vec3::new(1.0e9, 0.0, 0.0)),
    ));
    let venue = |n: &str| Alias::new(n, AliasKind::Venue, "test");
    b.add_alias(uuid(3), venue("Riker Memorial Spaceport"))
        .add_alias(uuid(3), venue("riker memorial  spaceport")) // duplicate once normalised
        .add_alias(uuid(3), Alias::new("AREA18", AliasKind::AltName, "test")) // = own name
        .add_alias(uuid(3), venue("Teasa Spaceport"))
        .add_alias(uuid(999), venue("Nowhere"));
    let (u, w) = b.build(DataMeta::default());

    assert_eq!(
        w,
        [BuildWarning::AliasTargetMissing {
            uuid: uuid(999),
            alias: "Nowhere".into()
        }]
    );
    let area18 = id_of(&u, "Area18");
    let names: Vec<&str> = u
        .get(area18)
        .aliases
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, ["Riker Memorial Spaceport", "Teasa Spaceport"]);

    let m = u.find_exact("RIKER MEMORIAL SPACEPORT");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].id, area18);
    assert_eq!(u.matched_name(&m[0]), "Riker Memorial Spaceport");
    assert_eq!(
        u.matched_alias(&m[0]).map(|a| a.kind),
        Some(AliasKind::Venue)
    );
    let d = u
        .distance(m[0].id, area18)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(d.metres(), 0.0);

    // OCR noise on an alias still lands on the target.
    let best = u.resolve("Riker Memorial Spaceprt", LookupOptions::default());
    assert_eq!(best.map(|l| l.id), Some(area18));

    // Own name wins over an alias on a tie.
    let m = u.find_exact("Teasa Spaceport");
    assert_eq!(m.len(), 2);
    assert_eq!(u.get(m[0].id).name, "Teasa Spaceport");
    assert_eq!(m[0].alias, None);
    assert_eq!(m[1].id, area18);

    // Space-insensitive fallback.
    assert_eq!(u.find_exact("Area 18").first().map(|m| m.id), Some(area18));
}
