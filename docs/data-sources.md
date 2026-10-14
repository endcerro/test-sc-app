# Data sources

Investigation done on 2026-10-14 against scunpacked-data commit `5ee0a3d`
(game version `4.10.2-LIVE.12881860`). Numbers below are from that revision.

## Summary

| Source | What it gives | Role |
| --- | --- | --- |
| **scunpacked-data** (GitHub, StarCitizenWiki) | Game files extracted per patch: starmap tree, in-game coordinates, amenities, jump connections | **Source of truth** for the universe tree and distances. Implemented. |
| **UEX Corp API 2.0** (`api.uexcorp.space/2.0`) | Community-maintained star systems / planets / moons / stations / cities / outposts / **terminals** with `id_*` hierarchy, commodity **prices**, routes | Planned: terminals + prices for the cargo optimizer. Not implemented. |
| **Star Citizen Wiki API** (`api.star-citizen.wiki`) | `/api/locations`, `/api/starsystems`, `/api/celestial-objects/<code>`: lore + RSI starmap data | Optional: descriptions, lore. No in-game coordinates. |
| **RSI starmap** (`robertsspaceindustries.com/api/starmap`) | Lore starmap (galactic positions, affiliations) | Not useful for in-game distances. |

Game files contain **no commodity prices** (the economy is server-side), so prices must
come from OCR (this app) and/or UEX.

## scunpacked-data

Repository: <https://github.com/StarCitizenWiki/scunpacked-data>

- One branch per game version (`3.24.2-LIVE.9327108`, …); `master` is the latest LIVE build.
  The commit message on `master` is the game version.
- Files are served at `https://raw.githubusercontent.com/<repo>/<sha>/<file>` (immutable per sha).
- Repo top level: `starmap.json`, `starmap_positions.json`, `trade_locations.json`,
  `labels.json` (localisation, ~11 MB), `resources/{commodities,commodity_trade_locations,locations,resources}.json`,
  `ships/`, `items/`, `contracts/`, `blueprints/`, `factions/`, `manufacturers.json`, `tags.json`.

### `starmap.json` (~2.1 MB, 2119 objects)

Array of starmap objects. Fields we use:

| Field | Notes |
| --- | --- |
| `UUID` | Stable object id (`GameUuid`). |
| `Name` | Already English. 76 are placeholders: `<= UNINITIALIZED =>`, `<= PLACEHOLDER =>`. |
| `ParentUUID` | Tree link. Stars have `null`. |
| `Type.Name` | `Star`, `Planet`, `Moon`, `LandingZone`, `Manmade`, `Manmade_VisibleOnInteraction`, `Outpost`, `Outpost_InvalidQT`, `PointOfInterest`, `Asteroid`, `Asteroid_ValidQT`, `Anomaly`, `JumpPoint`, `NavPoint`, plus UI helpers `SolarSystem`, `YouAreHere`, `CardinalPoint`, `QuantumTracePoint`. |
| `Type.ValidQuantumTravelDestination` | QT target flag. |
| `Size` | Body radius in metres for stars/planets/moons (Hurston = 1 000 000). |
| `HideInStarmap` | Hidden until discovered. |
| `LocationHierarchyTag.Name` | `Stanton1`, `Stanton1a`, `Pyro3`, … |
| `Jurisdiction.Name` | `Hurston Dynamics`, `Crusader Industries`, … |
| `Amenities[].Name/DisplayName` | `Commodity Trading - Freight Elevator`, `Commodity Trading - Loading Dock`, `Refinery`, `Hangar L`, `Clinic`, … 225 objects have a commodity-trading amenity. |

Also present, unused for now: `QuantumTravel{ObstructionRadius, ArrivalRadius, AdoptionRadius, …}`,
`RespawnLocationType`, `Affiliation`, `RadarContactType`, `AsteroidRing`, `Description`.

### `starmap_positions.json` (~475 KB)

```json
{ "entities": [ { "uuid", "name", "type", "system", "parent_uuid", "hidden", "qt_valid", "x", "y", "z" } ],
  "connections": [ { "entry_uuid", "exit_uuid", "entry_system", "exit_system", "fuel_cost" } ] }
```

- 1840 entity rows, 1247 unique uuids. Duplicated uuids are asteroid scatter sharing a
  template; we keep the first row.
- `system` is a lowercase code: `stanton`, `pyro`, `nyx`.
- **Coordinates**: metres, per-system frame, star at origin. Planets/moons have absolute
  positions (Hurston at 12.85 Gm from Stanton). Surface sites are the body centre plus an
  **unrotated** local offset (Hurston outposts sit ~1000 km from its centre = its radius), so
  surface distances are approximate (planet rotation is ignored).
- 885 starmap objects have no row here; for trading locations, 209 of 225 have coordinates,
  the rest (mostly Pyro `PYAM-FARSTAT-*`) inherit their parent's.
- `connections`: jump tunnels (Nyx ↔ Pyro, Pyro ↔ Stanton). Endpoints are the `Anomaly`
  "… Jump Point" objects under each `… Gateway` station. Treated as bidirectional.
- 13 entities exist only here (internal markers like `UGF_*`, type `unknown`); skipped.

### `labels.json` (~11 MB) — venue aliases

The game's English localisation (90 946 `key → string` entries). Only the 1 923
`mission_location_*` strings are used: many read **"<venue> in <location>"**, which ties
names that are not starmap objects to the location they belong to:

```text
mission_location_stanton_0299  "Riker Memorial Spaceport in Area18"
mission_location_stanton_799   "Teasa Spaceport in Lorville"
mission_location_nyx_003       "Cafe Musain in Levski"
```

Kept as `AliasKind::Venue` aliases of the location (`scunpacked/aliases.rs`) only when:

- the venue is a proper name (not lowercase, no `a`/`an`/`the`: drops "cave",
  "a Landing Pad Locker in Orison", "the Service Entrance in the QV Breaker Station"),
- the venue is not itself a location name and has no nested `on` ("Building A on a Resort
  Platform in Empyrean Park"),
- the location resolves to exactly one kept node that is not a planet/moon/star
  ("X **on** Hurston" means *somewhere on the planet*, not the same place, so `on` is ignored),
- every string mentioning the venue points at the same location.

On 4.10.2 that yields 15 venues (Lorville ×6, Area18 ×3, New Babbage ×2, Levski ×2,
Orison ×1, Grim HEX ×1). Only the `mission_location_*` subset is cached
(`<cache>/<sha>/mission_locations.json`, ~125 KB). The file is optional: if it can't be
downloaded, loading continues with curated aliases only and a note.

### Curated aliases (`crates/sc-data/data/aliases.json`)

Embedded at build time. Keyed by starmap UUID (stable across patches), with the expected
name as a sanity check (mismatch → load note). For names the game data never ties to a
location: `NB Int. Spaceport` / `New Babbage Interstellar Spaceport` (labels only say
"*on* microTech"), `Area 18`, and mission wording such as `ArcCorp's Yela facility`
(= `ArcCorp Mining Area 157`, the only ArcCorp site on Yela; hauling sample 09). The four main spaceports are also listed as a safety net.
Re-check with `sc-companion tree --aliases` after a patch.

### Not used (yet)

- `trade_locations.json`: hauling-mission endpoints (965) keyed by class name, with their own
  UUIDs and no starmap link (`StarmapObjectUUID` is null). Includes sub-areas ("Lobby",
  "On-Call Area") and city venues: `LandingZone_Stanton_3_Spaceport` = "Riker Memorial
  Spaceport" (`25321821-…`). Class names encode the place loosely (`LandingZone_Stanton_<n>`
  = the landing zone on the body tagged `Stanton<n>`, `DC_Stan_Covalex_S1_S1DC06_*`).
- `contracts/*.json` (5 153 files): mission definitions. Hauling contracts list pickup/dropoff
  pools as **trade_locations UUIDs** (e.g. Riker `25321821-…`, a separate `Area18` entry
  `23f264b8-…`), not starmap UUIDs. Mapping trade locations → starmap nodes (via display
  names + the aliases above, or class-name rules) is the way to read hauling contracts later.
- `resources/commodities.json`: 206 commodities (keys, names, SCU box sizes, density) — will be
  needed by the cargo tool.
- `resources/commodity_trade_locations.json`: commodity → trade locations by tag matching
  (mission-oriented, not terminal prices).

### Quirks to remember

- Many duplicate names: 49 "RSI Event Platform", 31 "QV Breaker Station", 19 "Derelict
  Outpost", several "Contested Zone". Lookups must be scoped (subtree) or disambiguated.
- L-point stations (`HUR-L1 Green Glade Station`) are children of the **star**, not the planet.
  The `HUR`/`CRU`/`ARC`/`MIC` prefix tells the related planet.
- `Pyro IV` is parented to `Pyro V` in the game data. Kept as-is.
- Hundreds of hidden objects without coordinates or amenities (305 "QV Logistics Station" in
  Nyx) are mission spawn templates; filtered by default (`LoadOptions`).
- Objects with no parent and no system (quantum beacons, mission targets) are dropped with a
  `BuildWarning::NoSystem`.
- Stanton has a `Terra Gateway` / `Stanton - Terra Jump Point` with no connection (Terra not
  released); Nyx lists jump points to Bremen, Odin, Tohil, Virgil without coordinates.

## UEX Corp API 2.0 (planned)

Base `https://api.uexcorp.space/2.0/`. Relevant resources: `star_systems`, `planets`, `moons`,
`orbits`, `space_stations`, `cities`, `outposts`, `poi`, `terminals`, `commodities`,
`commodities_prices` (per terminal), `commodities_routes`. Records carry `id_star_system`,
`id_planet`, `id_orbit`, `id_moon`, `id_space_station`, `id_city`, `id_outpost` links and
names. Public read endpoints; an app token raises rate limits. Field names to be confirmed
against the live docs when implemented (the host was not reachable from the dev sandbox).

Joining plan: match UEX locations to universe nodes by normalised name within the parent's
subtree (`Universe::find_fuzzy` with `scope`), keep the UEX id on a side table; terminals
become children (or attachments) of the matched location.

## Star Citizen Wiki API (optional)

`/api/locations` (systems, jump points, stars, planets, moons, asteroid formations, places),
`/api/starsystems` (RSI starmap-derived), `/api/celestial-objects/<code>`. Records from
`/api/locations` and `/api/starsystems` share no key (the wiki bridges by name). Useful for
descriptions; not needed for routing.
