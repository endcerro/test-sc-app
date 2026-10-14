# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

## Project

**SC Companion** — a Rust desktop companion app for Star Citizen, hosting several tools.
Roadmap:

1. **Universe data** (done): location tree + coordinates + distances, fetched once at launch.
2. **OCR** of in-game screens (commodity terminals, contracts) → resolve names to locations.
   Hauling contract screens: parsing + engine/preprocessing benchmark done (`docs/ocr.md`).
3. **Route optimizers**: hauling contracts done (`crates/sc-planner`, ported from the old
   Hauling Planner: which contracts to take and in what stop order, travel times from the
   universe distances); trading routes fed by OCR'd prices to come.
4. **GUI** with egui/eframe (`apps/sc-companion`): pane shell, **Map** pane, **Contracts**
   pane (drop hauling contract screenshots or press Ctrl+Shift+C in game -> OCR -> saved list) and **Routes** pane (plans the
   saved contracts) done; the Contracts view is a temporary UI (the reader/store under it
   stay); trading comes with its tool.

Read `docs/architecture.md` (design) and `docs/data-sources.md` (where the data comes from,
file schemas, quirks) before changing the data layer, and `docs/ocr.md` before changing OCR.

## Workspace

| Path | Role |
| --- | --- |
| `crates/sc-universe` | Pure domain model: `Universe`, `Location`, `LocationKind`, `UniverseBuilder`, geometry (`distance`, `route`), name lookup (`normalize`, `find_fuzzy`). **No IO.** |
| `crates/sc-data` | Data sources → `Universe`. `ScunpackedSource` (local dir or GitHub + disk cache), `HttpClient` trait, `Cache`, aliases (`scunpacked/aliases.rs`). |
| `crates/sc-data/data/aliases.json` | Hand-maintained aliases (embedded), keyed by starmap UUID. |
| `crates/sc-data/tests/fixtures/scunpacked` | Trimmed real scunpacked files (game 4.10.2) used by offline tests. |
| `crates/sc-contracts` | Pure: `HaulingContract` model (screen: one line per Deliver objective), `Contract` (flows: 1 pickup -> N drop-offs or N pickups -> 1 drop-off; `from_objectives`/`to_objectives`), `#HAULING v1` truth format (`truth`), OCR text -> contract (`parse`, `repair`; the offer list entry backs up a garbled reward box), scrolled-shot merge (`merge`), name snapping and place resolution (`snap`, embedded `data/commodities.json`), scoring (`score`). **No IO.** |
| `crates/sc-planner` | Pure: hauling route planner. `problem` (plain numbers), `route` (exact A* stop order, heuristic fallback), `select` (beam search over contract sets per `Goal`, distinct alternatives), `travel` (minutes between two locations: same place, same-body hop, quantum via `Universe::route`, jump crossings, missing-data guess), `plan` (`Contract`s + `Universe` + a place resolver -> `PlanOutput`: ranked routes, stops, cargo, warnings, why each contract was left out), `settings` (`Ship`, `Settings`). **No IO.** |
| `crates/sc-ocr` | Pixels -> text. `layout` (regions in a 1920x1080 reference frame, OFFERS-tab anchor, yellow bullet detection, highlighted offer list entry), `preprocess` (`Pipeline`: colour mask, top-hat, binarise, scale, despeckle, invert), `engine` (`OcrEngine` trait; `paddle`, `ocrs`, `winocr`, `tesseract` adapters behind features), `screen` (whole screenshot with the recommended settings; lines keep their screenshot box and confidence), `models` (pinned model URLs + sha256; `fetch` behind the `download` feature). |
| `tools/ocr-bench` | OCR benchmark: grid over engines x pipelines x crop modes, result cache, CER + end-to-end scoring, `summary.md` reports. Engines are opt-in features. |
| `samples/` | Contract screenshots, `truth/<stem>.txt` (`#HAULING v1`), `truth/regions/<stem>/<region>.txt` (exact region text). |
| `apps/sc-companion` | Binary. No subcommand → eframe GUI (`src/gui/`); subcommands → CLI (`src/cli.rs`: `summary`, `tree`, `find`, `dist`, `trading`, `contract`). OCR behind the `ocr` feature (off by default). |
| `apps/sc-companion/src/contracts` | No egui: `reader` (screenshot -> `ShotText`: text + line boxes, Paddle, models fetched on first use), `parse` (flows + snapping corrections), `fields` (`Field`: text/set of every editable value), `sources` (screenshot box a value was read from), `places` (place tree, suggestions, fixes, taught names), `review` (field marks, problems, next to check, duplicates), `board_text` (`#HAULING v1` import/export), `capture` (hotkey screen grab -> `shots/<ms>.png`, Windows via `xcap`), `prefs` (`board.json`: list order), `store` (`<id>.json` per contract + `place_names.json`; shots of the same contract merge; old `.txt` stores converted). |
| `apps/sc-companion/src/routes.rs` | No egui: `RouteSetup` (ships + planning settings) saved as `planner.json` in the contracts dir. |
| `apps/sc-companion/src/gui/routes` | Routes pane (ported from the hauling planner): setup bar (ship, start place, times, goal), planning thread, route cards (trip strip, cargo strip, stop table), "Not in the top route". |
| `apps/sc-companion/src/gui/widgets.rs` | Widgets shared by the Contracts and Routes panes (buffered fields, place autocomplete, buttons, badge). |
| `apps/sc-companion/src/gui/contracts` | Contracts pane (temporary view, ported from the hauling planner's board): drop box + paste/export box, background OCR `worker`, `hotkey` (Ctrl+Shift+C polled with `GetAsyncKeyState` via `device_query`, works while the game has focus), newest/oldest-first toggle (`#N` stays by age), `card` per contract, source crops, Next to check (F8). |
| `apps/sc-companion/src/gui/theme.rs` | App theme (planner palette, Segoe UI + Segoe UI Symbol on Windows; `theme::sym` falls back to plain text without them). |
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
  non-Latin-1 symbols in UI strings unless they go through `theme::sym(ctx, fancy, plain)`
  (egui's default fonts lack e.g. `→`, `✕`; the theme loads Segoe UI Symbol on Windows).
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
