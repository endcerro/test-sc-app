# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

## Project

**SC Companion** — a Rust desktop companion app for Star Citizen, hosting several tools.
Roadmap:

1. **Universe data** (done): location tree + coordinates + distances, fetched once at launch.
2. **OCR** of in-game screens (commodity terminals, contracts) → resolve names to locations.
3. **Cargo route optimizer** fed by OCR'd prices and the universe distances.
4. **GUI** with egui/eframe (`apps/sc-companion` becomes the eframe app).

Read `docs/architecture.md` (design) and `docs/data-sources.md` (where the data comes from,
file schemas, quirks) before changing the data layer.

## Workspace

| Path | Role |
| --- | --- |
| `crates/sc-universe` | Pure domain model: `Universe`, `Location`, `LocationKind`, `UniverseBuilder`, geometry (`distance`, `route`), name lookup (`normalize`, `find_fuzzy`). **No IO.** |
| `crates/sc-data` | Data sources → `Universe`. `ScunpackedSource` (local dir or GitHub + disk cache), `HttpClient` trait, `Cache`, aliases (`scunpacked/aliases.rs`). |
| `crates/sc-data/data/aliases.json` | Hand-maintained aliases (embedded), keyed by starmap UUID. |
| `crates/sc-data/tests/fixtures/scunpacked` | Trimmed real scunpacked files (game 4.10.2) used by offline tests. |
| `apps/sc-companion` | Binary. Currently a CLI inspector (`summary`, `tree`, `find`, `dist`, `trading`). |

## Commands

```sh
cargo build
cargo test --workspace                       # offline, must always pass
cargo test -p sc-data -- --ignored           # live GitHub download (network)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p sc-companion -- summary
cargo run -p sc-companion -- tree stanton --depth 2 --visible-only
cargo run -p sc-companion -- find "port tresler"
cargo run -p sc-companion -- dist Lorville "Ruin Station"
cargo run -p sc-companion -- find "riker memorial spaceport"        # alias -> Area18
cargo run -p sc-companion -- tree --depth 12 --aliases | grep -B1 " = "   # review aliases
cargo run -p sc-companion -- --data-dir /path/to/scunpacked-data summary   # offline
```

CI (`.github/workflows/ci.yml`) runs fmt check, clippy `-D warnings` and tests.

## Conventions

- `sc-universe` stays IO-free and dependency-light; anything touching disk/network goes in
  `sc-data` (or a future crate). The GUI must not reach into source DTOs.
- Errors: `thiserror` enums in libraries, `anyhow` only in `apps/`.
- No `unwrap`/`expect` in library code (clippy warns; CI denies warnings). In tests, prefer
  `unwrap_or_else(|e| panic!("{e}"))`.
- Units: distances and coordinates are **metres as `f64`**, in the owning system's frame
  (star at origin). Use `Distance` for display (picks m/km/Mm/Gm).
- `LocationId` is only valid for one `Universe` instance; persist `GameUuid` instead.
- Source DTOs (`sc-data/src/scunpacked/dto.rs`) declare only the fields we use, with
  `#[serde(default)]`, so upstream schema additions never break loading.
- Tests are offline by default. Network tests are `#[ignore]` with a reason.
- When changing classification/filters/alias rules, re-check real data with the CLI
  (`summary`, `tree --aliases`, `find`) and keep `docs/data-sources.md` in sync.
- Workspace edition 2024, MSRV 1.89 (set by `uuid`).

## Data notes (short version)

- Source of truth: `StarCitizenWiki/scunpacked-data`, files `starmap.json` (tree, metadata,
  amenities) + `starmap_positions.json` (coordinates, system codes, jump connections) +
  optional `labels.json` (only `mission_location_*` strings, cached as
  `mission_locations.json`).
- **Aliases**: names that are not starmap objects resolve to the location they belong to
  (`Riker Memorial Spaceport` → `Area18`, distance 0). Sources: "<venue> in <location>"
  label strings + `crates/sc-data/data/aliases.json`. Lookups return the target's own
  `LocationId`; `Match::alias` tells which name matched. Prefer adding a curated entry
  (by UUID) over special-casing names in code.
- Launch flow: resolve `master` → commit sha (GitHub API, fallback git smart HTTP) → load
  `cache/<sha>/` or download from `raw.githubusercontent.com/<repo>/<sha>/…` → on any network
  failure, use the last cached revision.
- Commodity prices and terminal lists are **not** in game files; UEX Corp API is the planned
  source (see `docs/data-sources.md`).
- Known quirks: placeholder names (`<= UNINITIALIZED =>`), many duplicate names
  ("RSI Event Platform"), L-point stations are children of the star, `Pyro IV` is parented to
  `Pyro V` in game data, surface coordinates ignore planet rotation.

## Environment notes

In Claude Code web sessions, `api.github.com` is not reachable, so the revision is resolved via
git smart HTTP and the game version shows as "unknown"; `raw.githubusercontent.com` and
crates.io work. UEX / star-citizen.wiki hosts are blocked there. GUI work cannot be viewed in
that environment.
