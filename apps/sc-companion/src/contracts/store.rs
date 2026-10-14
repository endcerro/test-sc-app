//! The saved contract list: one `<id>.json` per contract in the store directory, holding the
//! contract as flows (with the player's edits and loaded flags), what snapping changed, the
//! values the player confirmed and the raw OCR text and line boxes of every screenshot (so
//! it can be re-parsed, scrolled shots merged, and values shown on their pixels). Place
//! names the player taught are in `place_names.json`.
//!
//! `id` is the time the contract was added, in Unix milliseconds. Stores written by the
//! first version (`<id>.txt` in `#HAULING v1` + `<id>.ocr.json`) are converted on open.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use sc_contracts::{Contract, DropOff, Flow, Place, truth};
use sc_universe::{Universe, normalize};
use serde::{Deserialize, Serialize};

use super::fields::{self, Field};
use super::{Read, ReadKind, ShotText, parse};

/// One saved contract.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub contract: Contract,
    /// Screenshots it was read from, in the order they were added. Empty for imported
    /// contracts without OCR text.
    pub shots: Vec<ShotText>,
    /// Names snapping changed when reading the screenshots (see `contracts::parse`).
    pub reads: Vec<Read>,
    /// Values the player confirmed, as (field label, text): typed, or Enter pressed on them.
    pub confirmed: Vec<(String, String)>,
    /// Changed by the player since it was parsed.
    pub edited: bool,
    /// A screenshot was merged in after the player edited it, so the contract does not
    /// include that screenshot yet (see [`ContractStore::reparse`]).
    pub stale: bool,
}

impl Entry {
    /// Whether a newly read contract is this one again: a scrolled continuation or a
    /// re-capture. Needs a title, and the same title, reward and contractor.
    fn same_contract(&self, other: &Contract) -> bool {
        let c = &self.contract;
        c.title.is_some()
            && c.title == other.title
            && c.reward == other.reward
            && c.contractor == other.contractor
    }

    /// Rebuilds the contract from the screenshots, keeping the loaded flags of the flows
    /// that are still there (same commodity and pickups).
    fn reparse(&mut self, universe: &Universe) {
        let parsed = parse(&self.shots, universe);
        let mut contract = parsed.contract;
        for flow in &mut contract.flows {
            flow.loaded = self
                .contract
                .flows
                .iter()
                .any(|old| old.loaded && same_flow(old, flow));
        }
        self.contract = contract;
        self.reads = parsed.reads;
        self.confirmed.clear();
        self.edited = false;
        self.stale = false;
    }
}

fn without_loaded(c: &Contract) -> Contract {
    let mut c = c.clone();
    for f in &mut c.flows {
        f.loaded = false;
    }
    c
}

fn same_flow(a: &Flow, b: &Flow) -> bool {
    let names =
        |f: &Flow| -> Vec<String> { f.pickups.iter().map(|p| normalize(&p.name)).collect() };
    a.restricted == b.restricted
        && normalize(&a.commodity) == normalize(&b.commodity)
        && names(a) == names(b)
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
    /// Place names as written -> known place, taught by the player.
    place_names: BTreeMap<String, String>,
}

impl ContractStore {
    /// `.../sc-companion/contracts` in the user's data directory.
    pub fn default_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "sc-companion")
            .map(|d| d.data_dir().join("contracts"))
            .unwrap_or_else(|| PathBuf::from("contracts"))
    }

    /// Loads every entry of `dir` (missing directory = empty store) and converts entries
    /// of the old format. Files that cannot be read are skipped and reported in the
    /// returned warnings.
    pub fn open(dir: impl Into<PathBuf>) -> (Self, Vec<String>) {
        let mut store = Self {
            dir: dir.into(),
            entries: Vec::new(),
            place_names: BTreeMap::new(),
        };
        let mut warnings = Vec::new();
        match std::fs::read_to_string(store.names_path()) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(names) => store.place_names = names,
                Err(e) => warnings.push(format!("{}: {e}", store.names_path().display())),
            },
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                warnings.push(format!("{}: {e}", store.names_path().display()));
            }
            Err(_) => {}
        }
        let paths: Vec<PathBuf> = std::fs::read_dir(&store.dir)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        for path in &paths {
            if let Some(id) = entry_id(path, ".json") {
                match load_entry(path, id) {
                    Ok(entry) => store.entries.push(entry),
                    Err(e) => warnings.push(format!("{}: {e:#}", path.display())),
                }
            }
        }
        for path in &paths {
            let Some(id) = entry_id(path, ".txt") else {
                continue;
            };
            if store.get(id).is_some() {
                continue;
            }
            let imported = load_old_entry(path, id).and_then(|entry| {
                store.save(&entry)?;
                remove_if_exists(path)?;
                remove_if_exists(&path.with_file_name(format!("{id}.ocr.json")))?;
                Ok(entry)
            });
            match imported {
                Ok(entry) => store.entries.push(entry),
                Err(e) => warnings.push(format!("{}: {e:#}", path.display())),
            }
        }
        store.entries.sort_by_key(|e| std::cmp::Reverse(e.id));
        (store, warnings)
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
    /// one, otherwise as a new entry. Merging re-parses the entry from all its shots,
    /// unless the player edited it (then it is only marked stale).
    pub fn add_shot(&mut self, shot: ShotText, universe: &Universe) -> Result<Added> {
        let parsed = parse(std::slice::from_ref(&shot), universe);
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.same_contract(&parsed.contract))
        {
            let mut entry = self.entries[i].clone();
            entry.shots.push(shot);
            if entry.edited {
                entry.stale = true;
            } else {
                entry.reparse(universe);
            }
            self.save(&entry)?;
            let id = entry.id;
            self.entries[i] = entry;
            return Ok(Added::Merged(id));
        }
        let entry = Entry {
            id: self.next_id(),
            contract: parsed.contract,
            shots: vec![shot],
            reads: parsed.reads,
            ..Entry::default()
        };
        self.save(&entry)?;
        let id = entry.id;
        self.entries.insert(0, entry);
        Ok(Added::New(id))
    }

    /// Adds a contract typed or pasted by the player (no screenshot). Returns its id.
    pub fn add_contract(&mut self, contract: Contract) -> Result<u64> {
        let entry = Entry {
            id: self.next_id(),
            contract,
            ..Entry::default()
        };
        self.save(&entry)?;
        let id = entry.id;
        self.entries.insert(0, entry);
        Ok(id)
    }

    /// Sets a field from what the player typed (an unchanged value too: that confirms it).
    pub fn edit(&mut self, id: u64, f: Field, value: &str) -> Result<()> {
        self.modify(id, |e| {
            fields::set(&mut e.contract, f, value);
            let text = fields::text(&e.contract, f);
            let key = (f.label().to_owned(), text);
            if !key.1.is_empty() && !e.confirmed.contains(&key) {
                e.confirmed.push(key);
            }
        })
    }

    /// Ticks or unticks "loaded" on line `k` (not an edit of what was read).
    pub fn set_loaded(&mut self, id: u64, k: usize, loaded: bool) -> Result<()> {
        self.modify(id, |e| {
            if let Some(f) = e.contract.flows.get_mut(k) {
                f.loaded = loaded;
            }
        })
    }

    pub fn add_line(&mut self, id: u64) -> Result<()> {
        self.modify(id, |e| fields::add_line(&mut e.contract))
    }

    pub fn remove_line(&mut self, id: u64, k: usize) -> Result<()> {
        self.modify(id, |e| {
            if k < e.contract.flows.len() {
                e.contract.flows.remove(k);
            }
        })
    }

    /// The "Use ..." fix: replaces a misread place everywhere in the contract.
    pub fn apply_fix(&mut self, id: u64, written: &str, suggestion: &str) -> Result<()> {
        self.modify(id, |e| {
            fields::replace_place(&mut e.contract, written, suggestion);
            let key = ("pickup".to_owned(), suggestion.to_owned());
            let drop_key = ("drop-off".to_owned(), suggestion.to_owned());
            for k in [key, drop_key] {
                if !e.confirmed.contains(&k) {
                    e.confirmed.push(k);
                }
            }
        })
    }

    /// Changes an entry with `change` and saves it. Changes to what was read (anything but
    /// the loaded flags) mark it edited.
    fn modify(&mut self, id: u64, change: impl FnOnce(&mut Entry)) -> Result<()> {
        let Some(i) = self.entries.iter().position(|e| e.id == id) else {
            bail!("contract {id} not found");
        };
        let mut entry = self.entries[i].clone();
        change(&mut entry);
        if entry == self.entries[i] {
            return Ok(());
        }
        entry.edited |=
            without_loaded(&entry.contract) != without_loaded(&self.entries[i].contract);
        self.save(&entry)?;
        self.entries[i] = entry;
        Ok(())
    }

    /// Place names the player taught: as written -> known place.
    pub fn place_names(&self) -> &BTreeMap<String, String> {
        &self.place_names
    }

    /// Remembers that the game's `written` name means `place`.
    pub fn remember(&mut self, written: &str, place: &str) -> Result<()> {
        let mut names = self.place_names.clone();
        names.insert(written.trim().to_owned(), place.trim().to_owned());
        self.save_names(names)
    }

    pub fn forget(&mut self, written: &str) -> Result<()> {
        let mut names = self.place_names.clone();
        names.remove(written);
        self.save_names(names)
    }

    fn save_names(&mut self, names: BTreeMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let path = self.names_path();
        std::fs::write(&path, serde_json::to_string_pretty(&names)?)
            .with_context(|| format!("writing {}", path.display()))?;
        self.place_names = names;
        Ok(())
    }

    fn names_path(&self) -> PathBuf {
        self.dir.join("place_names.json")
    }

    /// Rebuilds an entry from its screenshots, discarding the player's edits (loaded flags
    /// are kept).
    pub fn reparse(&mut self, id: u64, universe: &Universe) -> Result<()> {
        let Some(i) = self.entries.iter().position(|e| e.id == id) else {
            bail!("contract {id} not found");
        };
        let mut entry = self.entries[i].clone();
        if entry.shots.is_empty() {
            bail!("no screenshot text recorded for this contract");
        }
        entry.reparse(universe);
        self.save(&entry)?;
        self.entries[i] = entry;
        Ok(())
    }

    pub fn remove(&mut self, id: u64) -> Result<()> {
        remove_if_exists(&self.path(id))?;
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
        let json = serde_json::to_string_pretty(&EntryFile::from(entry))?;
        let path = self.path(entry.id);
        // Write then rename, so a crash never leaves a truncated contract.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
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

    pub fn path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
}

/// `<digits><suffix>` -> id.
fn entry_id(path: &Path, suffix: &str) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(suffix)?.parse().ok()
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

fn load_entry(path: &Path, id: u64) -> Result<Entry> {
    let json = std::fs::read_to_string(path)?;
    let file: EntryFile = serde_json::from_str(&json)?;
    if file.version != FORMAT_VERSION {
        bail!("unsupported format version {}", file.version);
    }
    Ok(file.into_entry(id))
}

/// First format: `<id>.txt` (`#HAULING v1`) + optional `<id>.ocr.json` (shots).
fn load_old_entry(txt: &Path, id: u64) -> Result<Entry> {
    let objectives = truth::parse(&std::fs::read_to_string(txt)?)?;
    let ocr = txt.with_file_name(format!("{id}.ocr.json"));
    let shots = match std::fs::read_to_string(&ocr) {
        Ok(json) => {
            serde_json::from_str(&json).with_context(|| format!("reading {}", ocr.display()))?
        }
        Err(_) => Vec::new(),
    };
    Ok(Entry {
        id,
        contract: Contract::from_objectives(&objectives),
        shots,
        ..Entry::default()
    })
}

// On-disk DTOs. `sc-contracts` stays serde-free; these mirror its types.

const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct EntryFile {
    version: u32,
    contract: ContractFile,
    #[serde(default)]
    shots: Vec<ShotText>,
    #[serde(default)]
    reads: Vec<ReadFile>,
    #[serde(default)]
    confirmed: Vec<(String, String)>,
    #[serde(default)]
    edited: bool,
    #[serde(default)]
    stale: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct ReadFile {
    /// `commodity` or `place`.
    kind: String,
    read: String,
    known: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct ContractFile {
    title: Option<String>,
    reward: Option<u64>,
    contractor: Option<String>,
    availability: Option<String>,
    max_box: Option<u32>,
    rep: Option<u32>,
    flows: Vec<FlowFile>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct FlowFile {
    commodity: String,
    restricted: bool,
    pickups: Vec<PlaceFile>,
    dropoffs: Vec<DropOffFile>,
    loaded: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct PlaceFile {
    name: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    uncertain: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct DropOffFile {
    name: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    uncertain: bool,
    scu: u32,
}

impl From<&Entry> for EntryFile {
    fn from(e: &Entry) -> Self {
        let place = |p: &Place| PlaceFile {
            name: p.name.clone(),
            uncertain: p.uncertain,
        };
        let c = &e.contract;
        Self {
            version: FORMAT_VERSION,
            contract: ContractFile {
                title: c.title.clone(),
                reward: c.reward,
                contractor: c.contractor.clone(),
                availability: c.availability.clone(),
                max_box: c.max_box,
                rep: c.rep,
                flows: c
                    .flows
                    .iter()
                    .map(|f| FlowFile {
                        commodity: f.commodity.clone(),
                        restricted: f.restricted,
                        pickups: f.pickups.iter().map(place).collect(),
                        dropoffs: f
                            .dropoffs
                            .iter()
                            .map(|d| DropOffFile {
                                name: d.place.name.clone(),
                                uncertain: d.place.uncertain,
                                scu: d.scu,
                            })
                            .collect(),
                        loaded: f.loaded,
                    })
                    .collect(),
            },
            shots: e.shots.clone(),
            reads: e
                .reads
                .iter()
                .map(|r| ReadFile {
                    kind: match r.kind {
                        ReadKind::Commodity => "commodity",
                        ReadKind::Place => "place",
                    }
                    .to_owned(),
                    read: r.read.clone(),
                    known: r.known.clone(),
                })
                .collect(),
            confirmed: e.confirmed.clone(),
            edited: e.edited,
            stale: e.stale,
        }
    }
}

impl EntryFile {
    fn into_entry(self, id: u64) -> Entry {
        let place = |p: PlaceFile| Place {
            name: p.name,
            uncertain: p.uncertain,
        };
        let c = self.contract;
        Entry {
            id,
            contract: Contract {
                title: c.title,
                reward: c.reward,
                contractor: c.contractor,
                availability: c.availability,
                max_box: c.max_box,
                rep: c.rep,
                flows: c
                    .flows
                    .into_iter()
                    .map(|f| Flow {
                        commodity: f.commodity,
                        restricted: f.restricted,
                        pickups: f.pickups.into_iter().map(place).collect(),
                        dropoffs: f
                            .dropoffs
                            .into_iter()
                            .map(|d| DropOff {
                                place: Place {
                                    name: d.name,
                                    uncertain: d.uncertain,
                                },
                                scu: d.scu,
                            })
                            .collect(),
                        loaded: f.loaded,
                    })
                    .collect(),
            },
            shots: self.shots,
            reads: self
                .reads
                .into_iter()
                .map(|r| Read {
                    kind: if r.kind == "commodity" {
                        ReadKind::Commodity
                    } else {
                        ReadKind::Place
                    },
                    read: r.read,
                    known: r.known,
                })
                .collect(),
            confirmed: self.confirmed,
            edited: self.edited,
            stale: self.stale,
        }
    }
}

#[cfg(test)]
mod tests {
    use sc_contracts::HaulingContract;
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
            ..ShotText::default()
        }
    }

    fn truth_of(name: &str) -> HaulingContract {
        let text = std::fs::read_to_string(samples().join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        truth::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn flows_of(name: &str) -> Contract {
        Contract::from_objectives(&truth_of(name))
    }

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"))
    }

    fn add(store: &mut ContractStore, stem: &str) -> Added {
        store
            .add_shot(shot(stem), &universe())
            .unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn add_save_reload_remove() {
        let dir = temp();
        let (mut store, warnings) = ContractStore::open(dir.path());
        assert!(warnings.is_empty() && store.entries().is_empty());

        let a = add(&mut store, "01-dead-saints-riker");
        let b = add(&mut store, "06-covalex-wideforest-multidrop");
        assert!(matches!((a, b), (Added::New(_), Added::New(_))));
        assert!(b.id() > a.id());
        assert_eq!(store.entries()[0].id, b.id(), "newest first");
        assert_eq!(
            store.get(b.id()).map(|e| &e.contract),
            Some(&flows_of("06-covalex-wideforest-multidrop"))
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
    fn scrolled_shots_merge_and_keep_loaded_flags() {
        let dir = temp();
        let (mut store, _) = ContractStore::open(dir.path());
        let first = add(&mut store, "09-redwind-yela-scroll-1");
        store
            .set_loaded(first.id(), 0, true)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!store.entries()[0].edited, "loaded is not an edit");

        let second = add(&mut store, "09-redwind-yela-scroll-2");
        assert_eq!(second, Added::Merged(first.id()));
        assert_eq!(store.entries().len(), 1);
        let entry = &store.entries()[0];
        let mut want = flows_of("09-redwind-yela-scroll");
        want.flows[0].loaded = true;
        assert_eq!(entry.contract, want);
        assert_eq!(entry.shots.len(), 2);

        // A re-capture of the same screen changes nothing.
        add(&mut store, "09-redwind-yela-scroll-2");
        assert_eq!(store.entries()[0].contract, want);
    }

    #[test]
    fn edited_entries_are_not_reparsed_on_merge() {
        let dir = temp();
        let (mut store, _) = ContractStore::open(dir.path());
        let id = add(&mut store, "09-redwind-yela-scroll-1").id();
        store
            .edit(id, Field::Availability, "1h")
            .unwrap_or_else(|e| panic!("{e}"));
        let contract = store.entries()[0].contract.clone();

        assert_eq!(
            add(&mut store, "09-redwind-yela-scroll-2"),
            Added::Merged(id)
        );
        let entry = &store.entries()[0];
        assert!(entry.edited && entry.stale);
        assert_eq!(entry.contract, contract);
        assert_eq!(entry.shots.len(), 2);

        store
            .reparse(id, &universe())
            .unwrap_or_else(|e| panic!("{e}"));
        let entry = &store.entries()[0];
        assert!(!entry.edited && !entry.stale);
        assert_eq!(entry.contract, flows_of("09-redwind-yela-scroll"));
        // Survives a reload.
        assert_eq!(ContractStore::open(dir.path()).0.entries(), store.entries());
    }

    #[test]
    fn imports_the_old_format() {
        let dir = temp();
        let truth = truth::serialize(&truth_of("07-covalex-lyria-multipickup"));
        std::fs::write(dir.path().join("100.txt"), truth).unwrap_or_else(|e| panic!("{e}"));
        let shots = serde_json::to_string(&[shot("07-covalex-lyria-multipickup")])
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("100.ocr.json"), shots).unwrap_or_else(|e| panic!("{e}"));

        let (store, warnings) = ContractStore::open(dir.path());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(store.entries().len(), 1);
        assert_eq!(
            store.entries()[0].contract,
            flows_of("07-covalex-lyria-multipickup")
        );
        assert_eq!(store.entries()[0].shots.len(), 1);
        assert!(dir.path().join("100.json").is_file());
        assert!(!dir.path().join("100.txt").exists());
        assert!(!dir.path().join("100.ocr.json").exists());
        assert_eq!(ContractStore::open(dir.path()).0.entries(), store.entries());
    }

    #[test]
    fn edits_confirm_and_mark_edited() {
        let dir = temp();
        let (mut store, _) = ContractStore::open(dir.path());
        let id = add(&mut store, "06-covalex-wideforest-multidrop").id();
        let edit = |store: &mut ContractStore, f, v: &str| {
            store.edit(id, f, v).unwrap_or_else(|e| panic!("{e}"));
        };
        // Enter on an unchanged value confirms it, without editing.
        edit(&mut store, Field::Reward, "49,750");
        let e = &store.entries()[0];
        assert!(!e.edited);
        assert_eq!(e.confirmed, [("reward".to_owned(), "49,750".to_owned())]);

        store
            .set_loaded(id, 0, true)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!store.entries()[0].edited, "loaded is not an edit");
        edit(&mut store, Field::DropScu(0, 1), "7");
        store.add_line(id).unwrap_or_else(|e| panic!("{e}"));
        store.remove_line(id, 1).unwrap_or_else(|e| panic!("{e}"));
        store
            .apply_fix(id, "Baijini Point above ArcCorp", "Baijini Point")
            .unwrap_or_else(|e| panic!("{e}"));
        let e = &store.entries()[0];
        assert!(e.edited);
        assert_eq!(e.contract.flows.len(), 1);
        assert_eq!(e.contract.flows[0].scu(), 12);
        assert_eq!(e.contract.flows[0].dropoffs[1].place.name, "Baijini Point");
        assert_eq!(ContractStore::open(dir.path()).0.entries(), store.entries());

        let manual = store
            .add_contract(Contract::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.entries()[0].id, manual);
    }

    #[test]
    fn taught_place_names_persist() {
        let dir = temp();
        let (mut store, _) = ContractStore::open(dir.path());
        store
            .remember("ArcCorp's Yela facility", "ArcCorp Mining Area 157")
            .unwrap_or_else(|e| panic!("{e}"));
        let (again, warnings) = ContractStore::open(dir.path());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            again
                .place_names()
                .get("ArcCorp's Yela facility")
                .map(String::as_str),
            Some("ArcCorp Mining Area 157")
        );
        store
            .forget("ArcCorp's Yela facility")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(ContractStore::open(dir.path()).0.place_names().is_empty());
    }

    #[test]
    fn unreadable_files_are_reported() {
        let dir = temp();
        std::fs::write(dir.path().join("123.json"), "{").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("124.txt"), "not a contract")
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap_or_else(|e| panic!("{e}"));
        let (store, warnings) = ContractStore::open(dir.path());
        assert!(store.entries().is_empty());
        assert_eq!(warnings.len(), 2);
    }
}
