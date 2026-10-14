# Architecture

## Crates

```text
apps/sc-companion  ──►  sc-data  ──►  sc-universe
      (bin)            (sources,        (model, pure)
        │               cache, http)        ▲
        │                                   │ normalize, find_fuzzy
        └──────────► sc-ocr        sc-contracts ◄── apps/sc-companion
tools/ocr-bench ──►  (pixels → text)  (text → HaulingContract, pure)
```

- **sc-contracts** / **sc-ocr** / **tools/ocr-bench**: contract OCR, see `docs/ocr.md`. The
  app uses them in `src/contracts/` (reader + saved list) behind its `ocr` feature.

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

`gui/mod.rs` is the shell: a top bar with pane tabs (Map, Contracts; Trading is a placeholder)
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

**Contracts pane** (`gui/contracts/`, a temporary view: the UI will be redesigned, the
`contracts` reader/store underneath are meant to stay): a drop box (files dropped anywhere in the window, or a
file dialog) feeds `worker.rs`, a thread that owns the `ContractReader` and reads one
screenshot at a time (models downloaded and the engine loaded on the first one). Each result
is parsed, snapped to the universe and saved through `contracts::ContractStore` (merged into
an existing entry when title, reward and contractor match), then shown as a card in the
column. The selected card opens a side panel with the screenshots, raw OCR text and Remove.

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
- Trading pane; map overlays for routes once the optimizer exists.
- Screen capture (hotkey) instead of dropping screenshot files; commodity terminal screens; the cargo route optimizer on top of `Universe::route` and
  `HaulingContract`.
