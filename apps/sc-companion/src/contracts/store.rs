//! The saved contract list: one entry per contract, two files each in the store directory.
//!
//! - `<id>.txt`: the contract in the `#HAULING v1` truth format, plus `# source: <path>`
//!   comments. Human readable; edits are picked up at the next launch (but replaced if a new
//!   screenshot of the same contract is merged in, since that re-parses the OCR text).
//! - `<id>.ocr.json`: the raw OCR text of every screenshot ([`ShotText`]), so scrolled shots
//!   can be merged later and contracts re-parsed.
//!
//! `id` is the time the contract was added, in Unix milliseconds.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sc_contracts::{HaulingContract, truth};
use sc_universe::Universe;

use super::{ShotText, parse};

/// One saved contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub contract: HaulingContract,
    /// Screenshots it was read from, in the order they were added. Empty when the OCR file
    /// is missing (e.g. a hand-written `.txt`).
    pub shots: Vec<ShotText>,
}

impl Entry {
    pub fn sources(&self) -> impl Iterator<Item = &Path> {
        self.shots.iter().filter_map(|s| s.source.as_deref())
    }

    /// Whether a newly read contract is this one again: a scrolled continuation or a
    /// re-capture. Needs a title, and the same title, reward and contractor.
    fn same_contract(&self, other: &HaulingContract) -> bool {
        let c = &self.contract;
        c.title.is_some()
            && c.title == other.title
            && c.reward == other.reward
            && c.contractor == other.contractor
    }
}

/// What [`ContractStore::add_shot`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    New(u64),
    /// Merged into an existing entry (another screenshot of the same contract).
    Merged(u64),
}

impl Added {
    pub fn id(self) -> u64 {
        match self {
            Self::New(id) | Self::Merged(id) => id,
        }
    }
}

#[derive(Debug)]
pub struct ContractStore {
    dir: PathBuf,
    /// Newest first.
    entries: Vec<Entry>,
}

impl ContractStore {
    /// `.../sc-companion/contracts` in the user's data directory.
    pub fn default_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "sc-companion")
            .map(|d| d.data_dir().join("contracts"))
            .unwrap_or_else(|| PathBuf::from("contracts"))
    }

    /// Loads every entry of `dir` (missing directory = empty store). Files that cannot be
    /// read are skipped and reported in the returned warnings.
    pub fn open(dir: impl Into<PathBuf>) -> (Self, Vec<String>) {
        let dir = dir.into();
        let mut entries = Vec::new();
        let mut warnings = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&dir) {
            for path in read_dir.flatten().map(|e| e.path()) {
                let Some(id) = entry_id(&path) else {
                    continue;
                };
                match load_entry(&path, id) {
                    Ok(entry) => entries.push(entry),
                    Err(e) => warnings.push(format!("{}: {e:#}", path.display())),
                }
            }
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.id));
        (Self { dir, entries }, warnings)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Newest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Parses `shot` and saves it: merged into the entry of the same contract if there is
    /// one, otherwise as a new entry.
    pub fn add_shot(&mut self, shot: ShotText, universe: &Universe) -> Result<Added> {
        let contract = parse(std::slice::from_ref(&shot), universe);
        if let Some(i) = self.entries.iter().position(|e| e.same_contract(&contract)) {
            let mut entry = self.entries[i].clone();
            entry.shots.push(shot);
            entry.contract = parse(&entry.shots, universe);
            self.save(&entry)?;
            let id = entry.id;
            self.entries[i] = entry;
            return Ok(Added::Merged(id));
        }
        let entry = Entry {
            id: self.next_id(),
            contract,
            shots: vec![shot],
        };
        self.save(&entry)?;
        let id = entry.id;
        self.entries.insert(0, entry);
        Ok(Added::New(id))
    }

    pub fn remove(&mut self, id: u64) -> Result<()> {
        for path in [self.txt_path(id), self.ocr_path(id)] {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(e).with_context(|| format!("removing {}", path.display()));
                }
                _ => {}
            }
        }
        self.entries.retain(|e| e.id != id);
        Ok(())
    }

    pub fn clear(&mut self) -> Result<()> {
        let ids: Vec<u64> = self.entries.iter().map(|e| e.id).collect();
        ids.into_iter().try_for_each(|id| self.remove(id))
    }

    fn save(&self, entry: &Entry) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let mut text = truth::serialize(&entry.contract);
        for source in entry.sources() {
            text.push_str(&format!("# source: {}\n", source.display()));
        }
        let txt = self.txt_path(entry.id);
        std::fs::write(&txt, text).with_context(|| format!("writing {}", txt.display()))?;
        let ocr = self.ocr_path(entry.id);
        let json = serde_json::to_string_pretty(&entry.shots)?;
        std::fs::write(&ocr, json).with_context(|| format!("writing {}", ocr.display()))?;
        Ok(())
    }

    /// Current time in ms, bumped past the newest id so ids stay unique and ordered.
    fn next_id(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let newest = self.entries.iter().map(|e| e.id).max().unwrap_or(0);
        now.max(newest + 1)
    }

    fn txt_path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id}.txt"))
    }

    fn ocr_path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id}.ocr.json"))
    }
}

/// `<digits>.txt` -> id.
fn entry_id(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".txt")?.parse().ok()
}

fn load_entry(txt: &Path, id: u64) -> Result<Entry> {
    let text = std::fs::read_to_string(txt)?;
    let contract = truth::parse(&text)?;
    let ocr = txt.with_file_name(format!("{id}.ocr.json"));
    let shots = match std::fs::read_to_string(&ocr) {
        Ok(json) => {
            serde_json::from_str(&json).with_context(|| format!("reading {}", ocr.display()))?
        }
        Err(_) => Vec::new(),
    };
    Ok(Entry {
        id,
        contract,
        shots,
    })
}

#[cfg(test)]
mod tests {
    use sc_universe::{DataMeta, UniverseBuilder};

    use super::*;

    /// No places: snapping only touches commodities, so exact transcripts parse to truth.
    fn universe() -> Universe {
        UniverseBuilder::new().build(DataMeta::default()).0
    }

    fn samples() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/truth")
    }

    /// The exact region transcripts of a sample, as if OCR were perfect.
    fn shot(stem: &str) -> ShotText {
        let dir = samples().join("regions").join(stem);
        let read =
            |r: &str| std::fs::read_to_string(dir.join(format!("{r}.txt"))).unwrap_or_default();
        ShotText {
            source: Some(PathBuf::from(format!("samples/{stem}.png"))),
            title: read("title"),
            reward: read("reward"),
            details: read("details"),
            objectives: read("objectives"),
        }
    }

    fn truth_of(name: &str) -> HaulingContract {
        let text = std::fs::read_to_string(samples().join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        truth::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn add_save_reload_remove() {
        let dir = temp();
        let u = universe();
        let (mut store, warnings) = ContractStore::open(dir.path());
        assert!(warnings.is_empty() && store.entries().is_empty());

        let a = store
            .add_shot(shot("01-dead-saints-riker"), &u)
            .unwrap_or_else(|e| panic!("{e}"));
        let b = store
            .add_shot(shot("05-dead-saints-ruin-grimhex"), &u)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!((a, b), (Added::New(_), Added::New(_))));
        assert!(b.id() > a.id());
        assert_eq!(store.entries()[0].id, b.id(), "newest first");
        assert_eq!(
            store.get(a.id()).map(|e| &e.contract),
            Some(&truth_of("01-dead-saints-riker"))
        );

        let (reloaded, warnings) = ContractStore::open(dir.path());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(reloaded.entries(), store.entries());

        store.remove(a.id()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.entries().len(), 1);
        assert_eq!(ContractStore::open(dir.path()).0.entries().len(), 1);
        store.clear().unwrap_or_else(|e| panic!("{e}"));
        assert!(ContractStore::open(dir.path()).0.entries().is_empty());
    }

    #[test]
    fn scrolled_shots_merge_into_one_entry() {
        let dir = temp();
        let u = universe();
        let (mut store, _) = ContractStore::open(dir.path());
        let first = store
            .add_shot(shot("09-redwind-yela-scroll-1"), &u)
            .unwrap_or_else(|e| panic!("{e}"));
        let second = store
            .add_shot(shot("09-redwind-yela-scroll-2"), &u)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(second, Added::Merged(first.id()));
        assert_eq!(store.entries().len(), 1);
        let entry = &store.entries()[0];
        assert_eq!(entry.contract, truth_of("09-redwind-yela-scroll"));
        assert_eq!(entry.sources().count(), 2);

        // A re-capture of the same screen changes nothing.
        let again = store
            .add_shot(shot("09-redwind-yela-scroll-2"), &u)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again, Added::Merged(first.id()));
        assert_eq!(
            store.entries()[0].contract,
            truth_of("09-redwind-yela-scroll")
        );
    }

    #[test]
    fn unreadable_files_are_reported() {
        let dir = temp();
        std::fs::write(dir.path().join("123.txt"), "not a contract")
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap_or_else(|e| panic!("{e}"));
        let (store, warnings) = ContractStore::open(dir.path());
        assert!(store.entries().is_empty());
        assert_eq!(warnings.len(), 1);
    }
}
