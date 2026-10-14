# Architecture

## Crates

```text
apps/sc-companion  ──►  sc-data  ──►  sc-universe
      (bin)            (sources,        (model, pure)
                        cache, http)
```

- **sc-universe**: no IO. Owns the model and all algorithms on it (tree queries, distances,
  routing, name lookup). Everything else depends on it.
- **sc-data**: turns external data into a `Universe`. One module per source
  (`scunpacked/` today, `uex/` later). Owns HTTP (`http::HttpClient`, `UreqClient`) and the
  on-disk `Cache`.
- **apps/sc-companion**: entry point. CLI inspector today; the eframe GUI later. Loading is
  synchronous, so in the GUI it runs on a background thread at startup.

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
`hidden`, `hierarchy_tag`, `jurisdiction`, `amenities`.

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
6. build indices and jump links.

Warnings never fail the build; they're returned next to the `Universe`.

### Distances

- `Universe::distance(a, b)`: straight line, same system only.
- `Universe::route(a, b)`: fewest-jumps BFS over systems via `jump_links`, then straight
  lines inside each system: `a → entry JP`, jump, `exit JP → … → b`. Jumps add no distance.
- Accuracy: body/station positions are exact; surface sites ignore planet rotation, so error is
  bounded by the body's diameter. Quantum travel obstruction/arrival radii are not modelled yet.

### Name lookup (for OCR)

`normalize()` folds Unicode, case, punctuation, apostrophes, `&`, and `centre/center`.
`find_exact` returns every id for a name (duplicates are common). `find_fuzzy` scores with
max(Jaro-Winkler, normalised Levenshtein) and can be scoped to a subtree to tell apart
duplicates ("RSI Event Platform" on Hurston vs microTech). Future: an alias table
(old names, abbreviations like `ARC-L1`, terminal names).

## Loading pipeline (scunpacked)

```text
resolve ref ──► sha (+version)        GitHub API, fallback git smart HTTP (info/refs)
    │
    ├─ cache/<sha>/ complete? ──yes──► load from disk
    │        no
    ├─ download starmap.json + starmap_positions.json @ sha  ──► atomic write ──► manifest
    │
    └─ any failure ──► cache manifest "latest" ──► load (from_cache = true) ──► else Unavailable
```

Cache layout: `<cache_dir>/manifest.json` + `<cache_dir>/<sha>/<files>`, last 2 revisions
kept. Default dir: `directories::ProjectDirs::from("", "", "sc-companion").cache_dir()/scunpacked`.

Conversion (`scunpacked/convert.rs`): starmap.json is the base (names, types, amenities,
parents); positions add coordinates and system codes; placeholders and UI types are dropped;
asteroid scatter and hidden unpositioned templates are dropped unless `LoadOptions` asks;
children of dropped nodes are re-attached to the nearest kept ancestor.

## Next steps

- `sc-data::uex`: terminals + prices, joined onto the universe (see `data-sources.md`).
- Alias table for OCR names; terminal-level locations.
- eframe app shell with async startup load and a universe browser.
- OCR crate and the cargo route optimizer on top of `Universe::route`.
