# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

## Project

**SC Companion** — a Rust desktop companion app for Star Citizen, hosting several tools.
Roadmap:

1. **Universe data** (done): location tree + coordinates + distances, fetched once at launch.
2. **OCR** of in-game screens (commodity terminals, contracts) → resolve names to locations.
   Hauling contract screens: parsing + engine/preprocessing benchmark done (`docs/ocr.md`).
3. **Cargo route optimizer** fed by OCR'd prices and the universe distances.
4. **GUI** with egui/eframe (`apps/sc-companion`): pane shell, **Map** pane and **Contracts**
   pane (drop hauling contract screenshots -> OCR -> saved list) done; the Contracts view is a
   temporary UI (the reader/store under it stay); trading comes with its tool.

Read `docs/architecture.md` (design) and `docs/data-sources.md` (where the data comes from,
file schemas, quirks) before changing the data layer, and `docs/ocr.md` before changing OCR.

## Workspace

| Path | Role |
| --- | --- |
| `crates/sc-universe` | Pure domain model: `Universe`, `Location`, `LocationKind`, `UniverseBuilder`, geometry (`distance`, `route`), name lookup (`normalize`, `find_fuzzy`). **No IO.** |
| `crates/sc-data` | Data sources → `Universe`. `ScunpackedSource` (local dir or GitHub + disk cache), `HttpClient` trait, `Cache`, aliases (`scunpacked/aliases.rs`). |
| `crates/sc-data/data/aliases.json` | Hand-maintained aliases (embedded), keyed by starmap UUID. |
| `crates/sc-data/tests/fixtures/scunpacked` | Trimmed real scunpacked files (game 4.10.2) used by offline tests. |
| `crates/sc-contracts` | Pure: `HaulingContract` model (screen: one line per Deliver objective), `Contract` (flows: 1 pickup -> N drop-offs or N pickups -> 1 drop-off; `from_objectives`/`to_objectives`), `#HAULING v1` truth format (`truth`), OCR text -> contract (`parse`, `repair`), scrolled-shot merge (`merge`), name snapping and place resolution (`snap`, embedded `data/commodities.json`), scoring (`score`). **No IO.** |
| `crates/sc-ocr` | Pixels -> text. `layout` (regions in a 1920x1080 reference frame, OFFERS-tab anchor, yellow bullet detection), `preprocess` (`Pipeline`: colour mask, top-hat, binarise, scale, despeckle, invert), `engine` (`OcrEngine` trait; `paddle`, `ocrs`, `winocr`, `tesseract` adapters behind features), `screen` (whole screenshot with the recommended settings), `models` (pinned model URLs + sha256; `fetch` behind the `download` feature). |
| `tools/ocr-bench` | OCR benchmark: grid over engines x pipelines x crop modes, result cache, CER + end-to-end scoring, `summary.md` reports. Engines are opt-in features. |
| `samples/` | Contract screenshots, `truth/<stem>.txt` (`#HAULING v1`), `truth/regions/<stem>/<region>.txt` (exact region text). |
| `apps/sc-companion` | Binary. No subcommand → eframe GUI (`src/gui/`); subcommands → CLI (`src/cli.rs`: `summary`, `tree`, `find`, `dist`, `trading`, `contract`). OCR behind the `ocr` feature (off by default). |
| `apps/sc-companion/src/contracts` | No egui: `reader` (screenshot -> `ShotText` with Paddle, models fetched on first use), `parse` (flows + snapping corrections), `checks` (what to double-check), `store` (saved list: `<id>.json` with flows, edits, loaded flags and raw OCR text; shots of the same contract merge; old `.txt` stores converted). |
| `apps/sc-companion/src/gui/contracts` | Contracts pane (temporary view): drop box (drag & drop or file dialog), background OCR `worker`, editable `card` per contract (`draft` = fields as typed text), capture preview, re-parse. |
| `apps/sc-companion/src/gui/map` | Map pane: `camera` (f64 orthographic 2D/3D projection), `scene` (pure what-to-draw + LOD, tested), `paint`, `style`, `inspector`. |

## Commands

```sh
cargo build
cargo test --workspace                       # offline, must always pass
cargo test -p sc-data -- --ignored           # live GitHub download (network)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p sc-companion                                            # GUI
cargo run -p sc-companion -- --data-dir crates/sc-data/tests/fixtures/scunpacked   # GUI, offline fixture
cargo run -p sc-companion --release --features ocr                   # GUI with the Contracts OCR
cargo run -p sc-companion --release --features ocr -- contract samples/05-dead-saints-ruin-grimhex.webp
cargo run -p sc-companion --release --features ocr -- contract --save samples/09-*.png   # merged into the saved list
cargo run -p sc-companion -- summary
cargo run -p sc-companion -- tree stanton --depth 2 --visible-only
cargo run -p sc-companion -- find "port tresler"
cargo run -p sc-companion -- dist Lorville "Ruin Station"
cargo run -p sc-companion -- find "riker memorial spaceport"        # alias -> Area18
cargo run -p sc-companion -- tree --depth 12 --aliases | grep -B1 " = "   # review aliases
cargo run -p sc-companion -- --data-dir /path/to/scunpacked-data summary   # offline
cargo run -p ocr-bench --release --features all-engines -- fetch-models   # once (network)
cargo run -p ocr-bench --release --features all-engines -- run --preset quick      # ~5 min
cargo run -p ocr-bench --release --features all-engines -- run --universe          # reduced, ~25 min
cargo run -p ocr-bench --release --features all-engines -- run --universe --preset full --name full  # overnight, resumable
cargo run -p ocr-bench --release -- dump --sample 05                      # crops for inspection
```

CI (`.github/workflows/ci.yml`) runs fmt check, clippy `-D warnings` and tests.

## Conventions

- `sc-universe` stays IO-free and dependency-light; anything touching disk/network goes in
  `sc-data` (or a future crate). The GUI must not reach into source DTOs, and egui stays out
  of the library crates.
- GUI: keep computation (projection, scene/LOD) separate from painting so it can be unit
  tested; world maths in f64 relative to the camera centre, f32 only once in pixels. Avoid
  non-Latin-1 symbols in UI strings (egui's default fonts lack e.g. `→`, `✕`).
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
- OCR engines are cargo features, **off by default** (also `sc-companion`'s `ocr`), so CI
  needs no models, ONNX Runtime or Windows APIs. Text interpretation belongs in `sc-contracts`, not in engine adapters. After
  changing parsing, re-run `run` (cached OCR text is reused, only scoring reruns); after
  changing layout, pipelines or adapters, bump `CACHE_VERSION` in `tools/ocr-bench/src/grid.rs`.
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
