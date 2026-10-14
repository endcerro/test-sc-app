//! [scunpacked-data](https://github.com/StarCitizenWiki/scunpacked-data): game files
//! extracted per patch by the Star Citizen Wiki. Source of truth for the location tree and
//! coordinates. See `docs/data-sources.md`.

mod aliases;
mod convert;
pub mod dto;
pub mod fetch;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sc_universe::DataMeta;

use self::dto::{MissionLocations, PositionsFile, StarmapObject};
use crate::http::{HttpClient, UreqClient};
use crate::{Cache, CacheEntry, DataError, LoadOptions, LoadedUniverse, UniverseSource};

pub const DEFAULT_REPO: &str = "StarCitizenWiki/scunpacked-data";
pub const DEFAULT_REF: &str = "master";
const SOURCE_NAME: &str = "scunpacked-data";

const STARMAP: &str = "starmap.json";
const POSITIONS: &str = "starmap_positions.json";
/// Files needed to build the universe.
pub const FILES: [&str; 2] = [STARMAP, POSITIONS];
/// Game localization (11 MB). Only its `mission_location_*` strings are used, for venue
/// aliases; optional.
pub const LABELS: &str = "labels.json";
/// What the cache keeps of [`LABELS`]: just the `mission_location_*` entries.
pub const MISSION_LOCATIONS: &str = "mission_locations.json";

/// Where and what to fetch.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    /// `owner/name` on GitHub.
    pub repo: String,
    /// Branch, tag or commit. Branches other than `master` are named after game versions.
    pub git_ref: String,
    pub cache_dir: PathBuf,
}

impl RemoteConfig {
    /// Defaults with the platform cache dir (e.g. `~/.cache/sc-companion/scunpacked`).
    pub fn with_default_cache() -> Self {
        let cache_dir = directories::ProjectDirs::from("", "", "sc-companion")
            .map(|d| d.cache_dir().join("scunpacked"))
            .unwrap_or_else(|| std::env::temp_dir().join("sc-companion").join("scunpacked"));
        Self {
            repo: DEFAULT_REPO.to_owned(),
            git_ref: DEFAULT_REF.to_owned(),
            cache_dir,
        }
    }
}

#[derive(Debug, Clone)]
enum Mode {
    Dir(PathBuf),
    Remote(RemoteConfig),
}

/// Loads the universe from scunpacked-data, either a local directory or GitHub (+ cache).
#[derive(Clone)]
pub struct ScunpackedSource {
    mode: Mode,
    options: LoadOptions,
    http: Arc<dyn HttpClient>,
}

impl std::fmt::Debug for ScunpackedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScunpackedSource")
            .field("mode", &self.mode)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl ScunpackedSource {
    /// Reads `starmap.json` and `starmap_positions.json` from a local directory (a
    /// checkout of the repo, or a cache revision directory), plus `labels.json` or
    /// `mission_locations.json` if present.
    pub fn from_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            mode: Mode::Dir(dir.into()),
            options: LoadOptions::default(),
            http: Arc::new(UreqClient::new()),
        }
    }

    /// Fetches from GitHub at launch, caching to disk and falling back to the cache offline.
    pub fn remote(config: RemoteConfig) -> Self {
        Self {
            mode: Mode::Remote(config),
            options: LoadOptions::default(),
            http: Arc::new(UreqClient::new()),
        }
    }

    pub fn with_options(mut self, options: LoadOptions) -> Self {
        self.options = options;
        self
    }

    /// Replaces the HTTP client (tests, custom proxies).
    pub fn with_http_client(mut self, http: Arc<dyn HttpClient>) -> Self {
        self.http = http;
        self
    }

    fn load_remote(&self, cfg: &RemoteConfig) -> Result<LoadedUniverse, DataError> {
        let cache = Cache::new(&cfg.cache_dir);
        let mut notes = Vec::new();

        let fresh =
            fetch::resolve_revision(self.http.as_ref(), &cfg.repo, &cfg.git_ref).and_then(|rev| {
                // Keep a version learnt earlier (API) if this lookup could not get one.
                let version = rev
                    .version
                    .clone()
                    .or_else(|| cache.entry(&rev.sha).and_then(|e| e.version));
                if cache.has_all(&rev.sha, &FILES) {
                    notes.push(format!("data up to date (revision {})", short(&rev.sha)));
                    let fetched = cache.entry(&rev.sha).map(|e| e.fetched_at_unix);
                    return Ok((rev.sha, version, fetched));
                }
                for file in FILES {
                    let url = fetch::raw_url(&cfg.repo, &rev.sha, file);
                    tracing::info!("downloading {url}");
                    let bytes = self.http.get(&url, None)?;
                    cache.write_file(&rev.sha, file, &bytes)?;
                }
                let now = now_unix();
                cache.commit(CacheEntry {
                    revision: rev.sha.clone(),
                    version: version.clone(),
                    fetched_at_unix: now,
                })?;
                notes.push(format!("downloaded revision {}", short(&rev.sha)));
                Ok((rev.sha, version, Some(now)))
            });
        // Labels are optional: a failure only costs the derived aliases.
        if let Ok((sha, ..)) = &fresh
            && !self.options.skip_label_aliases
            && !cache.has_all(sha, &[MISSION_LOCATIONS])
            && let Err(err) = self.fetch_mission_locations(&cache, &cfg.repo, sha)
        {
            tracing::warn!("labels download failed: {err}");
            notes.push(format!(
                "could not download {LABELS} ({err}); venue aliases limited to curated ones"
            ));
        }

        let (sha, version, fetched_at, from_cache) = match fresh {
            Ok((sha, version, fetched)) => (sha, version, fetched, false),
            Err(err) => {
                let Some(entry) = cache
                    .latest()
                    .filter(|e| cache.has_all(&e.revision, &FILES))
                else {
                    return Err(DataError::Unavailable(format!(
                        "{err}; and no cached data in {}",
                        cache.root().display()
                    )));
                };
                tracing::warn!("using cached universe data: {err}");
                notes.push(format!(
                    "could not refresh data ({err}); using cached revision {}",
                    short(&entry.revision)
                ));
                (
                    entry.revision,
                    entry.version,
                    Some(entry.fetched_at_unix),
                    true,
                )
            }
        };

        let meta = DataMeta {
            source: SOURCE_NAME.to_owned(),
            game_version: version,
            revision: Some(sha.clone()),
            fetched_at_unix: fetched_at,
            from_cache,
        };
        let mut loaded = load_dir(&cache.revision_dir(&sha), self.options, meta)?;
        loaded.notes.splice(0..0, notes);
        Ok(loaded)
    }

    /// Downloads `labels.json` and caches only its mission location strings.
    fn fetch_mission_locations(
        &self,
        cache: &Cache,
        repo: &str,
        sha: &str,
    ) -> Result<(), DataError> {
        let url = fetch::raw_url(repo, sha, LABELS);
        tracing::info!("downloading {url}");
        let bytes = self.http.get(&url, None)?;
        let labels: MissionLocations =
            serde_json::from_slice(&bytes).map_err(|source| DataError::Json {
                what: url.clone(),
                source,
            })?;
        let json = serde_json::to_vec_pretty(&labels).map_err(|source| DataError::Json {
            what: MISSION_LOCATIONS.into(),
            source,
        })?;
        cache.write_file(sha, MISSION_LOCATIONS, &json)
    }
}

impl UniverseSource for ScunpackedSource {
    fn load(&self) -> Result<LoadedUniverse, DataError> {
        match &self.mode {
            Mode::Dir(dir) => {
                let meta = DataMeta {
                    source: format!("{SOURCE_NAME} ({})", dir.display()),
                    ..DataMeta::default()
                };
                load_dir(dir, self.options, meta)
            }
            Mode::Remote(cfg) => self.load_remote(cfg),
        }
    }
}

fn load_dir(dir: &Path, options: LoadOptions, meta: DataMeta) -> Result<LoadedUniverse, DataError> {
    let starmap: Vec<StarmapObject> = read_json(&dir.join(STARMAP))?;
    let positions: PositionsFile = read_json(&dir.join(POSITIONS))?;
    let mut notes = Vec::new();
    let labels: Option<MissionLocations> = if options.skip_label_aliases {
        None
    } else if let Some(path) = [MISSION_LOCATIONS, LABELS]
        .into_iter()
        .map(|f| dir.join(f))
        .find(|p| p.is_file())
    {
        Some(read_json(&path)?)
    } else {
        notes.push(format!(
            "no {LABELS} in {}; venue aliases limited to curated ones",
            dir.display()
        ));
        None
    };
    let mut loaded = build(&starmap, &positions, labels.as_ref(), options, meta);
    loaded.notes.splice(0..0, notes);
    Ok(loaded)
}

/// Builds a universe from already parsed files. `labels` feeds venue aliases.
pub fn build(
    starmap: &[StarmapObject],
    positions: &PositionsFile,
    labels: Option<&MissionLocations>,
    options: LoadOptions,
    meta: DataMeta,
) -> LoadedUniverse {
    let (builder, notes) = convert::build(starmap, positions, labels, options);
    let (universe, warnings) = builder.build(meta);
    for w in &warnings {
        tracing::debug!("universe data: {w}");
    }
    LoadedUniverse {
        universe,
        warnings,
        notes,
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, DataError> {
    let bytes = fs::read(path).map_err(|e| DataError::io(path, e))?;
    serde_json::from_slice(&bytes).map_err(|source| DataError::Json {
        what: path.display().to_string(),
        source,
    })
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn short(sha: &str) -> &str {
    sha.get(..10).unwrap_or(sha)
}
