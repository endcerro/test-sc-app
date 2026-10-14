//! Loading tests against a trimmed copy of real scunpacked-data files
//! (`tests/fixtures/scunpacked`, game version 4.10.2-LIVE.12881860).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use sc_data::http::HttpClient;
use sc_data::{
    DataError, LoadOptions, LoadedUniverse, RemoteConfig, ScunpackedSource, UniverseSource,
};
use sc_universe::{AliasKind, LocationId, LocationKind, LookupOptions, Position, Universe};

const SHA: &str = "5ee0a3dd20ba0e8dfd13512c57a2309231c8b8f7";
const REPO: &str = "StarCitizenWiki/scunpacked-data";

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scunpacked")
}

fn load(options: LoadOptions) -> LoadedUniverse {
    ScunpackedSource::from_dir(fixture_dir())
        .with_options(options)
        .load()
        .unwrap_or_else(|e| panic!("fixture load failed: {e}"))
}

fn one(u: &Universe, name: &str) -> LocationId {
    match u.find_exact(name).as_slice() {
        [m] => m.id,
        other => panic!("expected one {name:?}, found {}", other.len()),
    }
}

#[test]
fn builds_system_tree() {
    let u = load(LoadOptions::default()).universe;
    let systems: Vec<&str> = u.systems().map(|s| s.name.as_str()).collect();
    assert!(
        systems.contains(&"Stanton") && systems.contains(&"Pyro"),
        "{systems:?}"
    );

    let lorville = one(&u, "Lorville");
    assert_eq!(u.get(lorville).kind, LocationKind::City);
    assert_eq!(u.path_string(lorville), "Stanton › Hurston › Lorville");
    let kinds: Vec<LocationKind> = u.ancestors(lorville).map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        [
            LocationKind::Planet,
            LocationKind::Star,
            LocationKind::System
        ]
    );

    let l1 = one(&u, "HUR-L1 Green Glade Station");
    assert_eq!(u.parent(l1).map(|p| p.kind), Some(LocationKind::Star));
    assert!(u.nearest_body(l1).is_none());
    assert!(u.get(l1).has_commodity_trading());

    let gateway = u.parent(one(&u, "Stanton-Pyro Jump Point"));
    assert_eq!(gateway.map(|g| g.kind), Some(LocationKind::Gateway));
}

#[test]
fn filters_noise_by_default() {
    let u = load(LoadOptions::default()).universe;
    assert!(u.find_exact("<= UNINITIALIZED =>").is_empty());
    assert!(
        u.find_exact("Stanton System").is_empty(),
        "SolarSystem UI object dropped"
    );
    assert!(u.find_exact("QV Logistics Station").is_empty());
    assert_eq!(u.iter_kind(LocationKind::Asteroid).count(), 0);

    let all = load(LoadOptions {
        include_asteroid_scatter: true,
        include_hidden_unpositioned: true,
        ..LoadOptions::default()
    })
    .universe;
    assert!(!all.find_exact("QV Logistics Station").is_empty());
    assert!(all.iter_kind(LocationKind::Asteroid).count() > 0);
}

#[test]
fn positions_and_inheritance() {
    let u = load(LoadOptions::default()).universe;
    let hurston = u.get(one(&u, "Hurston"));
    assert!(hurston.position.is_exact());
    assert_eq!(hurston.radius_m, Some(1_000_000.0));

    // The RSI Event Platform on Arial has no coordinates: it inherits Arial's.
    let arial = one(&u, "Arial");
    let on_arial: Vec<_> = u
        .children(arial)
        .filter(|c| c.name == "RSI Event Platform")
        .collect();
    assert_eq!(on_arial.len(), 1);
    assert!(matches!(on_arial[0].position, Position::Inherited { from, .. } if from == arial));
}

#[test]
fn distances_and_routes() {
    let u = load(LoadOptions::default()).universe;
    let lorville = one(&u, "Lorville");

    let d = u
        .distance(lorville, one(&u, "Everus Harbor"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!((200.0..300.0).contains(&d.km()), "Lorville-Everus {d}");

    let d = u
        .distance(lorville, one(&u, "Area18"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!((22.0..24.0).contains(&d.gm()), "Lorville-Area18 {d}");

    assert_eq!(u.jump_links().len(), 1);
    let r = u
        .route(lorville, one(&u, "Ruin Station"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r.jumps(), 1);
    assert!(
        (110.0..115.0).contains(&r.total.gm()),
        "Lorville-Ruin {}",
        r.total
    );
}

#[test]
fn lookup_on_real_names() {
    let u = load(LoadOptions::default()).universe;
    // Spelling variants are folded.
    assert_eq!(u.find_exact("Covalex Distribution Center S1DC06").len(), 1);
    // Duplicated names need a scope.
    assert_eq!(u.find_exact("RSI Event Platform").len(), 3);
    let mic = one(&u, "microTech");
    let m = u.find_fuzzy(
        "RSI Event Platform",
        sc_universe::LookupOptions {
            scope: Some(mic),
            ..Default::default()
        },
    );
    assert_eq!(m.len(), 1);
    // OCR-ish noise.
    let best = u.resolve("EVERUS HARB0R", Default::default());
    assert_eq!(best.map(|l| l.name.as_str()), Some("Everus Harbor"));
}

#[test]
fn venue_aliases_resolve_to_their_location() {
    let loaded = load(LoadOptions::default());
    let u = &loaded.universe;
    let area18 = one(u, "Area18");
    let lorville = one(u, "Lorville");

    // From labels.json ("... in Area18") and/or the curated file: same location either way.
    let m = u.find_exact("riker memorial spaceport");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].id, area18);
    assert_eq!(u.matched_name(&m[0]), "Riker Memorial Spaceport");
    assert_eq!(
        u.matched_alias(&m[0]).map(|a| a.kind),
        Some(AliasKind::Venue)
    );
    assert_eq!(one(u, "Adira Falls Apartments"), area18);
    assert_eq!(one(u, "L19 Admin Office"), lorville);
    assert_eq!(one(u, "Area 18"), area18);

    let d = u
        .distance(one(u, "Riker Memorial Spaceport"), area18)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(d.metres(), 0.0);
    let d = u
        .distance(
            one(u, "Teasa Spaceport"),
            one(u, "Riker Memorial Spaceport"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert!((22.0..24.0).contains(&d.gm()), "Teasa-Riker {d}");

    let best = u.resolve("RIKER MEM0RIAL SPACEPRT", LookupOptions::default());
    assert_eq!(best.map(|l| l.id), Some(area18));

    // Noise is not imported: "on <planet>", articles, venue names that are locations.
    for l in u.locations() {
        for a in &l.aliases {
            assert!(
                [
                    "Riker Memorial Spaceport",
                    "Adira Falls Apartments",
                    "Teasa Spaceport",
                    "L19 Admin Office",
                    "Area 18"
                ]
                .contains(&a.name.as_str()),
                "unexpected alias {:?} on {}",
                a.name,
                l.name
            );
        }
    }
    // Curated targets absent from the fixture (New Babbage...) are skipped silently.
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert!(loaded.notes.is_empty(), "{:?}", loaded.notes);
}

#[test]
fn label_aliases_can_be_disabled() {
    let u = load(LoadOptions {
        skip_label_aliases: true,
        ..LoadOptions::default()
    })
    .universe;
    assert!(u.find_exact("Adira Falls Apartments").is_empty());
    // Curated aliases stay.
    assert_eq!(one(&u, "Riker Memorial Spaceport"), one(&u, "Area18"));
}

// ---- remote flow with a fake HTTP client ---------------------------------------------------

#[derive(Default)]
struct FakeHttp {
    routes: HashMap<String, Vec<u8>>,
    calls: Mutex<Vec<String>>,
}

impl FakeHttp {
    fn online() -> Self {
        let mut routes = HashMap::new();
        routes.insert(
            format!("https://api.github.com/repos/{REPO}/commits/master"),
            format!(r#"{{"sha":"{SHA}","commit":{{"message":"4.10.2-LIVE.12881860"}}}}"#)
                .into_bytes(),
        );
        let files = sc_data::scunpacked::FILES
            .into_iter()
            .chain([sc_data::scunpacked::LABELS]);
        for f in files {
            let bytes = std::fs::read(fixture_dir().join(f)).unwrap_or_else(|e| panic!("{e}"));
            routes.insert(sc_data::scunpacked::fetch::raw_url(REPO, SHA, f), bytes);
        }
        Self {
            routes,
            ..Default::default()
        }
    }

    fn offline() -> Self {
        Self::default()
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

impl HttpClient for FakeHttp {
    fn get(&self, url: &str, _accept: Option<&str>) -> Result<Vec<u8>, DataError> {
        if let Ok(mut c) = self.calls.lock() {
            c.push(url.to_owned());
        }
        self.routes
            .get(url)
            .cloned()
            .ok_or_else(|| DataError::Http {
                url: url.to_owned(),
                message: "offline".into(),
            })
    }
}

fn remote(cache: &std::path::Path, http: Arc<FakeHttp>) -> ScunpackedSource {
    ScunpackedSource::remote(RemoteConfig {
        repo: REPO.into(),
        git_ref: "master".into(),
        cache_dir: cache.to_path_buf(),
    })
    .with_http_client(http)
}

#[test]
fn remote_download_then_cache_hit_then_offline_fallback() {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));

    // 1. Cold cache: downloads the two universe files and the labels.
    let http = Arc::new(FakeHttp::online());
    let loaded = remote(tmp.path(), http.clone())
        .load()
        .unwrap_or_else(|e| panic!("{e}"));
    let meta = loaded.universe.meta();
    assert_eq!(meta.game_version.as_deref(), Some("4.10.2-LIVE.12881860"));
    assert_eq!(meta.revision.as_deref(), Some(SHA));
    assert!(!meta.from_cache);
    assert_eq!(http.calls().len(), 4, "{:?}", http.calls());
    assert!(!loaded.universe.find_exact("Lorville").is_empty());
    // Only the mission location strings of labels.json are cached.
    let cached = tmp.path().join(SHA);
    assert!(
        cached
            .join(sc_data::scunpacked::MISSION_LOCATIONS)
            .is_file()
    );
    assert!(!cached.join(sc_data::scunpacked::LABELS).exists());
    assert!(
        !loaded
            .universe
            .find_exact("Adira Falls Apartments")
            .is_empty()
    );

    // 2. Same revision: only the ref lookup, no download.
    let http = Arc::new(FakeHttp::online());
    let loaded = remote(tmp.path(), http.clone())
        .load()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(http.calls().len(), 1, "{:?}", http.calls());
    assert!(!loaded.universe.meta().from_cache);

    // 3. Offline: falls back to the cached revision, keeps the version.
    let http = Arc::new(FakeHttp::offline());
    let loaded = remote(tmp.path(), http)
        .load()
        .unwrap_or_else(|e| panic!("{e}"));
    let meta = loaded.universe.meta();
    assert!(meta.from_cache);
    assert_eq!(meta.game_version.as_deref(), Some("4.10.2-LIVE.12881860"));
    assert!(loaded.notes.iter().any(|n| n.contains("cached")));
}

#[test]
fn labels_are_optional() {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));

    // Labels download fails: the universe still loads, with curated aliases only.
    let mut http = FakeHttp::online();
    http.routes.retain(|url, _| !url.ends_with("/labels.json"));
    let loaded = remote(tmp.path(), Arc::new(http))
        .load()
        .unwrap_or_else(|e| panic!("{e}"));
    let u = &loaded.universe;
    assert!(u.find_exact("Adira Falls Apartments").is_empty());
    assert!(!u.find_exact("Riker Memorial Spaceport").is_empty());
    assert!(
        loaded.notes.iter().any(|n| n.contains("labels.json")),
        "{:?}",
        loaded.notes
    );

    // Offline with that cache (no mission_locations.json): still loads.
    let loaded = remote(tmp.path(), Arc::new(FakeHttp::offline()))
        .load()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(loaded.universe.meta().from_cache);

    // Back online: only the missing labels are fetched.
    let http = Arc::new(FakeHttp::online());
    let loaded = remote(tmp.path(), http.clone())
        .load()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(http.calls().len(), 2, "{:?}", http.calls());
    assert!(http.calls()[1].ends_with("/labels.json"));
    assert!(
        !loaded
            .universe
            .find_exact("Adira Falls Apartments")
            .is_empty()
    );
}

#[test]
fn falls_back_to_smart_http_when_api_fails() {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut http = FakeHttp::online();
    http.routes
        .retain(|url, _| !url.starts_with("https://api.github.com"));
    let pkt = |s: &str| format!("{:04x}{s}", s.len() + 4);
    let refs = format!(
        "{}0000{}{}0000",
        pkt("# service=git-upload-pack\n"),
        pkt(&format!("{SHA} HEAD\0caps\n")),
        pkt(&format!("{SHA} refs/heads/master\n")),
    );
    http.routes.insert(
        format!("https://github.com/{REPO}.git/info/refs?service=git-upload-pack"),
        refs.into_bytes(),
    );
    let loaded = remote(tmp.path(), Arc::new(http))
        .load()
        .unwrap_or_else(|e| panic!("{e}"));
    let meta = loaded.universe.meta();
    assert_eq!(meta.revision.as_deref(), Some(SHA));
    assert_eq!(meta.game_version, None);
}

#[test]
fn offline_without_cache_is_unavailable() {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let err = remote(tmp.path(), Arc::new(FakeHttp::offline())).load();
    assert!(matches!(err, Err(DataError::Unavailable(_))), "{err:?}");
}

/// Real network: `cargo test -p sc-data -- --ignored`.
#[test]
#[ignore = "needs network access to GitHub"]
fn live_master_invariants() {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let loaded = ScunpackedSource::remote(RemoteConfig {
        cache_dir: tmp.path().to_path_buf(),
        ..RemoteConfig::with_default_cache()
    })
    .load()
    .unwrap_or_else(|e| panic!("{e}"));
    let u = &loaded.universe;

    for sys in ["Stanton", "Pyro", "Nyx"] {
        assert!(u.system(sys).is_some(), "missing system {sys}");
    }
    assert!(u.trading_locations().count() > 200);
    assert!(!u.jump_links().is_empty());
    for l in u.locations() {
        assert_eq!(u.get(l.system).kind, LocationKind::System);
        if l.kind != LocationKind::System {
            assert!(l.parent.is_some(), "{} has no parent", l.name);
            assert!(u.ancestors(l.id).count() < 16, "{} too deep", l.name);
        }
    }
    for name in [
        "Lorville",
        "Area18",
        "New Babbage",
        "Orison",
        "Port Tressler",
        "Everus Harbor",
    ] {
        assert!(!u.find_exact(name).is_empty(), "missing {name}");
    }
    for (venue, city) in [
        ("Riker Memorial Spaceport", "Area18"),
        ("Teasa Spaceport", "Lorville"),
        ("August Dunlow Spaceport", "Orison"),
        ("NB Int. Spaceport", "New Babbage"),
        ("Area 18", "Area18"),
    ] {
        let m = u.find_exact(venue);
        assert_eq!(
            m.first().map(|m| u.get(m.id).name.as_str()),
            Some(city),
            "{venue}"
        );
    }
    assert!(
        loaded.notes.iter().all(|n| !n.contains("curated")),
        "{:?}",
        loaded.notes
    );
}
