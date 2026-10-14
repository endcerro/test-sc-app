# Architecture

## Crates

```text
apps/sc-companion  ──►  sc-data  ──►  sc-universe
      (bin)            (sources,        (model, pure)
        │               cache, http)        ▲     ▲
        │                                   │     │ route, nearest_body
        │                                   │   sc-planner ◄── apps/sc-companion
        │                                   │   (contracts → routes, pure)
        │                                   │     │
        │                                   │ normalize, find_fuzzy
        └──────────► sc-ocr        sc-contracts ◄── apps/sc-companion
tools/ocr-bench ──►  (pixels → text)  (text → HaulingContract, pure)
```

- **sc-contracts** / **sc-ocr** / **tools/ocr-bench**: contract OCR, see `docs/ocr.md`. The
  app uses them in `src/contracts/` (reader + saved list) behind its `ocr` feature.
  `HaulingContract` is the contract as the screen lists it (one line per `Deliver` objective:
  one drop-off, 1+ pickups; truth files and scoring use it). `Contract` is the contract to
  plan: commodity **flows**, each with one pickup and N drop-offs (SCU per drop-off) or N
  pickups and one drop-off (total SCU; the game does not split it per pickup).
  `Contract::from_objectives` merges `Deliver` lines of the same commodity from the same
  single pickup; `to_objectives` goes back.

- **sc-planner**: hauling route planning on `Contract` flows and the `Universe`, ported from
  the old Hauling Planner; see "Route planning" below.
- **sc-universe**: no IO. Owns the model and all algorithms on it (tree queries, distances,
  routing, name lookup). Everything else depends on it.
- **sc-data**: turns external data into a `Universe`. One module per source
  (`scunpacked/` today, `uex/` later). Owns HTTP (`http::HttpClient`, `UreqClient`) and the
  on-disk `Cache`.
- **apps/sc-companion**: entry point. No subcommand opens the eframe GUI; subcommands run the
  CLI inspector. Both build the source from the same `SourceArgs` flags. Loading is
  synchronous, so in the GUI it runs on a background thread at startup (`gui/loader.rs`).

## Universe model

Arena of `Location`s indexed by `LocationId(u32)`; tree links are ids.

```text
System (synthetic, one per system code)
└─ Star
   ├─ Planet ── Moon ── Outpost / POI / Station ...
   │        └─ City (LandingZone) / Station (orbital, e.g. Everus Harbor) / Outpost / POI
   ├─ Station (L-point stations, rest stops)
   ├─ Gateway ── JumpPoint
   └─ AsteroidBase ...
```

`Location` keeps: `uuid` (`GameUuid`, stable), `name`, `kind` (`LocationKind`), `raw_type`
(source type string), `parent`/`children`, `system`, `position`, `radius_m`, `qt_valid`,
`hidden`, `hierarchy_tag`, `jurisdiction`, `amenities`, `aliases`.

`Alias { name, kind, source }` is another name that resolves to the location itself:
`AliasKind::AltName` (`Area 18`) or `AliasKind::Venue`, a named place inside it
(`Riker Memorial Spaceport` in `Area18`). Venues are deliberately **not** separate nodes:
for travel they are the same place, so lookups return the location's own id and distances
are 0 without special cases. If hauling ever needs sub-area granularity, venues can become
child nodes fed by the same data.

`Position`:
- `Exact(Vec3)`: from the source.
- `Inherited { from, pos }`: source had none; copied from the closest positioned ancestor.
- `Unknown`: nothing usable (system roots).

`Universe` indices: `by_uuid`, normalised-name index, system list, `jump_links`, `DataMeta`
(source, game version, revision, fetch time, `from_cache`).

### Building

`UniverseBuilder` takes flat `NodeSpec`s (uuid, parent uuid, optional system code, …) and
`JumpSpec`s, then `build(meta)`:

1. resolve parents (unknown parent → `DanglingParent` warning),
2. cut parent cycles (`Cycle` warning),
3. assign each node the system code of its root (`NoSystem` → dropped),
4. create system roots, attach roots below them, sort children (kind, name),
5. propagate positions top-down,
6. attach aliases (`add_alias(target_uuid, Alias)`; unknown target → `AliasTargetMissing`,
   duplicates of the name or an earlier alias dropped),
7. build indices and jump links.

Warnings never fail the build; they're returned next to the `Universe`.

### Distances

- `Universe::distance(a, b)`: straight line, same system only.
- `Universe::route(a, b)`: fewest-jumps BFS over systems via `jump_links`, then straight
  lines inside each system: `a → entry JP`, jump, `exit JP → … → b`. Jumps add no distance.
- Accuracy: body/station positions are exact; surface sites ignore planet rotation, so error is
  bounded by the body's diameter. Quantum travel obstruction/arrival radii are not modelled yet.

### GUI

`gui/mod.rs` is the shell: a top bar with pane tabs (Map, Contracts, Routes; Trading is a placeholder)
and data provenance, a spinner while loading, an error + Retry on failure. Each pane owns its
side panels and central area.

**Map pane** (`gui/map/`), one star system at a time:

- `camera.rs`: orthographic camera, `center` (system frame, f64 metres) + `scale` (m/px),
  top-down or tilted (yaw around Z, pitch towards the plane). Projection is done in f64
  relative to the centre so metre offsets survive at 10¹¹ m. Zoom keeps the point under the
  cursor fixed; focus changes animate (log-interpolated scale).
- `scene.rs`: pure "what to draw". `MapIndex` precomputes each location's *cluster radius*
  (sphere containing its subtree). Focus drill-down + LOD: a node's children are drawn when
  its cluster spans > 30 px on screen or it is on the focus path; otherwise they fold into its
  marker as `+N`. Orbits: planets around the star (so the `Pyro IV` mis-parenting is
  harmless), moons around their body. Tilted view adds a polar grid and drop-lines to the
  orbital plane. Jump points get a clickable `» <system>` label.
- `paint.rs` / `style.rs`: markers per `LocationKind`, real-size translucent bodies, dimmed
  hidden locations, hollow inherited positions, trading ring, greedy label decluttering.
- `inspector.rs`: all `Location` fields, amenities, aliases, children, jump links, and the
  distance/route from the focus or a pinned "measure from" location.

**Contracts pane** (`gui/contracts/`, a temporary view ported from the hauling planner's
contract board; the `contracts` modules underneath are meant to stay): a drop box (files
dropped anywhere in the window, or a file dialog) feeds `worker.rs`, a thread that owns the
`ContractReader` and reads one screenshot at a time; a paste box imports and exports
`#HAULING v1` text (`contracts::board_text`). Each screenshot is parsed into flows, snapped
and saved through `contracts::ContractStore`, then shown as a card (`card.rs`) on the board,
oldest first (`#1` at the top):

- Every value is a `contracts::fields::Field`. Fields are `widgets::field`s with an edit
  buffer that commit on Enter or when focus leaves (Enter on an unchanged value confirms a
  flagged read); place fields offer known places while typing (`places::Places::suggest`).
- `contracts::review` marks each field: red when it blocks planning (missing reward, SCU or
  place, unknown place), dashed amber when it is a doubtful read (inferred `?` pickup, name
  changed by snapping, ambiguous place, OCR line below `LOW_CONFIDENCE`) and not yet
  confirmed. "Next to check (n) · F8" walks them in board order.
- Hovering a field (or focusing it, under the cargo lines) shows the outlined crop of the
  screenshot it was read from: OCR lines keep their box and confidence (`ShotText::boxes`)
  and `contracts::sources::locate` finds a value back by content.
- Under each place: its universe path, the candidates of a shared name, or "unknown place";
  "Use X for Y" buttons offer a close spelling; replacing an unknown name by a known place
  offers to remember it (`place_names.json`, applied by `Places`).
- The "+" for a place is disabled when it would give a line several pickups *and* several
  drop-offs. Reviews are cached per entry until it changes.

**Routes pane** (`gui/routes/`, ported from the hauling planner's setup bar and results
column): plans the Contracts pane's saved contracts in board order (so `#N` matches) with the
ship and settings of `routes::RouteSetup` (`planner.json` in the contracts dir, saved on
change). Planning runs on a thread with a clone of the Contracts pane's `Places`, so taught
names resolve. The header counts fields still to check and duplicate pairs, and says when the
contracts, ship or settings changed since the plan. Each route card shows reward, time,
aUEC/h, a trip strip (stops spaced by time; jumps dashed thick, guesses red), a cargo strip
(estimate and hard maximum marked), warnings and a stop table.

### Name lookup (for OCR)

`normalize()` folds Unicode, case, punctuation, apostrophes, `&`, and `centre/center`.
The name index holds own names **and aliases**; a `Match { id, score, alias }` says which
(`Universe::matched_name` / `matched_alias`). Rules:

- `find_exact` returns every location for a name (duplicates are common), one match per
  location, own-name matches first. If nothing matches, it retries ignoring spaces
  (`Area 18` = `Area18`, `GrimHEX` = `Grim HEX`).
- `find_fuzzy` scores with max(Jaro-Winkler, normalised Levenshtein) (1.0 when equal
  ignoring spaces), keeps the best name per location, ranks own names before aliases on
  ties, and can be scoped to a subtree to tell apart duplicates ("RSI Event Platform" on
  Hurston vs microTech).

Alias sources: `labels.json` venue strings and the curated `crates/sc-data/data/aliases.json`
(see `data-sources.md`); UEX terminal names will be the next one.

## Route planning (`sc-planner`)

- `plan()` turns each contract line into one order per drop-off (one pickup → N drop-offs keeps
  each drop-off's SCU; N pickups → one drop-off is split evenly, flagged); a line marked loaded
  has no pickups. Contracts that can't be planned (no reward, unknown or ambiguous place,
  several pickups *and* drop-offs, too big for the hard maximum alone) are left out with the
  reason. Place names go through a resolver the caller passes (the app's `Places::resolve`).
- `travel::leg` gives the minutes between two places: 0 for the same place; the pilot's hop
  time when both are on or around the same planet or moon (`nearest_body`); otherwise
  `Universe::route`, each straight leg a quantum jump (spool + distance / speed) plus the stop
  time per jump point crossed; a 120 Gm guess, flagged, when there is no position or route.
- `route::best_route` orders the stops of a contract set: exact A* over (pickups done,
  orders delivered, location), heuristic + local search above 150k states.
  `select::select` beam-searches contract sets (at most 10 minus those held) for the goal
  (best aUEC/h over a minimum session, most reward in a time budget, or all contracts
  fastest) and keeps distinct alternatives; `plan` labels them, adds warnings and says why
  each other contract isn't in the top route. Tests check both against brute force.

## Loading pipeline (scunpacked)

```text
resolve ref ──► sha (+version)        GitHub API, fallback git smart HTTP (info/refs)
    │
    ├─ cache/<sha>/ complete? ──yes──► load from disk
    │        no
    ├─ download starmap.json + starmap_positions.json @ sha  ──► atomic write ──► manifest
    ├─ mission_locations.json missing? ──► download labels.json, keep mission_location_*
    │                                       (failure = note only, curated aliases still apply)
    │
    └─ any failure ──► cache manifest "latest" ──► load (from_cache = true) ──► else Unavailable
```

Cache layout: `<cache_dir>/manifest.json` + `<cache_dir>/<sha>/<files>`, last 2 revisions
kept. Default dir: `directories::ProjectDirs::from("", "", "sc-companion").cache_dir()/scunpacked`.

Conversion (`scunpacked/convert.rs`): starmap.json is the base (names, types, amenities,
parents); positions add coordinates and system codes; placeholders and UI types are dropped;
asteroid scatter and hidden unpositioned templates are dropped unless `LoadOptions` asks;
children of dropped nodes are re-attached to the nearest kept ancestor; aliases (curated, then
labels-derived) are resolved against the kept nodes (`scunpacked/aliases.rs`).

## Next steps

- `sc-data::uex`: terminals + prices, joined onto the universe (see `data-sources.md`).
- More aliases: UEX terminal names, common abbreviations; map `trade_locations` UUIDs (hauling
  contracts) onto locations.
- Trading pane; a map overlay for the selected hauling route.
- Screen capture (hotkey) instead of dropping screenshot files; commodity terminal screens; a trading route optimizer next to the hauling planner.
