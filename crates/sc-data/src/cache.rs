//! On-disk cache of downloaded source files, one directory per revision.
//!
//! ```text
//! <root>/manifest.json         { "latest": { "revision": "...", "version": "...", ... } }
//! <root>/<revision>/<file>     raw files exactly as downloaded
//! ```

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::DataError;

/// Revisions kept on disk; older ones are pruned after a successful download.
const KEEP_REVISIONS: usize = 2;
const MANIFEST: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub revision: String,
    pub version: Option<String>,
    pub fetched_at_unix: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    latest: Option<CacheEntry>,
    #[serde(default)]
    history: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn revision_dir(&self, revision: &str) -> PathBuf {
        self.root.join(revision)
    }

    /// True if every file in `files` exists for `revision`.
    pub fn has_all(&self, revision: &str, files: &[&str]) -> bool {
        let dir = self.revision_dir(revision);
        files.iter().all(|f| dir.join(f).is_file())
    }

    /// Latest successfully completed revision, if any.
    pub fn latest(&self) -> Option<CacheEntry> {
        self.read_manifest().latest
    }

    /// Cached entry for a given revision, if it is the latest one.
    pub fn entry(&self, revision: &str) -> Option<CacheEntry> {
        self.latest().filter(|e| e.revision == revision)
    }

    /// Writes one file atomically (temp file + rename).
    pub fn write_file(&self, revision: &str, name: &str, bytes: &[u8]) -> Result<(), DataError> {
        let dir = self.revision_dir(revision);
        fs::create_dir_all(&dir).map_err(|e| DataError::io(&dir, e))?;
        let path = dir.join(name);
        let tmp = dir.join(format!(".{name}.tmp"));
        let mut f = fs::File::create(&tmp).map_err(|e| DataError::io(&tmp, e))?;
        f.write_all(bytes).map_err(|e| DataError::io(&tmp, e))?;
        f.sync_all().map_err(|e| DataError::io(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| DataError::io(&path, e))
    }

    /// Records `entry` as the latest complete revision and prunes old ones.
    pub fn commit(&self, entry: CacheEntry) -> Result<(), DataError> {
        let mut m = self.read_manifest();
        m.history.retain(|r| *r != entry.revision);
        m.history.insert(0, entry.revision.clone());
        for old in m.history.split_off(KEEP_REVISIONS.min(m.history.len())) {
            let dir = self.revision_dir(&old);
            if let Err(e) = fs::remove_dir_all(&dir) {
                tracing::debug!("could not prune {}: {e}", dir.display());
            }
        }
        m.latest = Some(entry);
        let json = serde_json::to_vec_pretty(&m).map_err(|source| DataError::Json {
            what: "cache manifest".into(),
            source,
        })?;
        fs::create_dir_all(&self.root).map_err(|e| DataError::io(&self.root, e))?;
        let path = self.root.join(MANIFEST);
        let tmp = self.root.join(".manifest.json.tmp");
        fs::write(&tmp, json).map_err(|e| DataError::io(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| DataError::io(&path, e))
    }

    fn read_manifest(&self) -> Manifest {
        let path = self.root.join(MANIFEST);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("ignoring corrupt cache manifest {}: {e}", path.display());
                Manifest::default()
            }),
            Err(_) => Manifest::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rev: &str) -> CacheEntry {
        CacheEntry {
            revision: rev.into(),
            version: Some("1.0".into()),
            fetched_at_unix: 1,
        }
    }

    #[test]
    fn write_commit_and_prune() -> Result<(), DataError> {
        let tmp = tempfile::tempdir().map_err(|e| DataError::io("tempdir", e))?;
        let cache = Cache::new(tmp.path());
        assert!(cache.latest().is_none());

        for rev in ["a", "b", "c"] {
            cache.write_file(rev, "f.json", b"{}")?;
            cache.commit(entry(rev))?;
        }
        assert_eq!(cache.latest().map(|e| e.revision), Some("c".into()));
        assert!(cache.has_all("c", &["f.json"]));
        assert!(cache.has_all("b", &["f.json"]));
        assert!(!cache.has_all("a", &["f.json"]), "oldest revision pruned");
        assert!(cache.entry("b").is_none());
        Ok(())
    }
}
