//! How the player likes the contract board shown, kept between sessions in `board.json`
//! next to the saved contracts. No egui here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BoardPrefs {
    /// Newest contract at the top (numbers stay by age either way).
    pub newest_first: bool,
    /// Contracts folded down to their header line.
    pub collapsed: BTreeSet<u64>,
}

impl Default for BoardPrefs {
    fn default() -> Self {
        Self {
            newest_first: true,
            collapsed: BTreeSet::new(),
        }
    }
}

impl BoardPrefs {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("board.json")
    }

    /// The prefs saved in `dir`, or the defaults (with a warning when the file exists but
    /// can't be read).
    pub fn load(dir: &Path) -> (Self, Option<String>) {
        let path = Self::path(dir);
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(p) => (p, None),
                Err(e) => (Self::default(), Some(format!("{}: {e}", path.display()))),
            },
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                (Self::default(), Some(format!("{}: {e}", path.display())))
            }
            Err(_) => (Self::default(), None),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = Self::path(dir);
        std::fs::write(&path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ContractStore;

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(BoardPrefs::load(dir.path()), (BoardPrefs::default(), None));
        let prefs = BoardPrefs {
            newest_first: false,
            collapsed: [3, 5].into(),
        };
        prefs.save(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(BoardPrefs::load(dir.path()).0, prefs);
        let (store, warnings) = ContractStore::open(dir.path());
        assert!(
            store.entries().is_empty() && warnings.is_empty(),
            "{warnings:?}"
        );

        // Files from before a pref existed still load.
        std::fs::write(BoardPrefs::path(dir.path()), r#"{"newest_first":false}"#)
            .unwrap_or_else(|e| panic!("{e}"));
        let (prefs, warning) = BoardPrefs::load(dir.path());
        assert!(!prefs.newest_first && prefs.collapsed.is_empty() && warning.is_none());

        std::fs::write(BoardPrefs::path(dir.path()), "not json").unwrap_or_else(|e| panic!("{e}"));
        let (prefs, warning) = BoardPrefs::load(dir.path());
        assert_eq!(prefs, BoardPrefs::default());
        assert!(warning.is_some());
    }
}
