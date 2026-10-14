//! Terminal inspector for the universe data (`summary`, `tree`, `find`, `dist`, `trading`)
//! and contract screenshot reader (`contract`).

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use sc_data::UniverseSource;
use sc_universe::{Leg, Location, LocationId, LocationKind, LookupOptions, Match, Universe};

use crate::SourceArgs;
use crate::contracts::places::Places;
use crate::contracts::review::{self, Lookup};
use crate::contracts::{self, ContractReader, ContractStore, Entry, fields, store::Added};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Data provenance and counts per system / kind.
    Summary,
    /// Print the location tree.
    Tree {
        /// System or location to start from (default: everything).
        root: Option<String>,
        #[arg(long, default_value_t = 3)]
        depth: usize,
        /// Hide locations hidden on the in-game starmap.
        #[arg(long)]
        visible_only: bool,
        /// List aliases (alternative names, venues) under each location.
        #[arg(long)]
        aliases: bool,
    },
    /// Fuzzy search a location by name.
    Find {
        text: String,
        /// Restrict to a system or location subtree.
        #[arg(long = "in")]
        scope: Option<String>,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    /// Distance / route between two locations.
    Dist { from: String, to: String },
    /// List locations with commodity trading.
    Trading {
        /// System or location subtree.
        scope: Option<String>,
    },
    /// OCR hauling contract screenshots and print them as `#HAULING v1` (needs the `ocr`
    /// feature).
    Contract {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Add them to the saved contract list shown in the GUI (shots of the same contract
        /// are merged; see --contracts-dir).
        #[arg(long)]
        save: bool,
        /// Also print the raw OCR text of each region.
        #[arg(long)]
        raw: bool,
    },
}

pub fn run(source: &SourceArgs, command: Command) -> Result<()> {
    let started = Instant::now();
    let loaded = source.build().load().context("loading universe data")?;
    for note in &loaded.notes {
        eprintln!("note: {note}");
    }
    if !loaded.warnings.is_empty() {
        eprintln!(
            "note: {} data warnings (RUST_LOG=sc_data=debug to list)",
            loaded.warnings.len()
        );
    }
    let u = &loaded.universe;
    eprintln!("loaded {} locations in {:.0?}", u.len(), started.elapsed());

    match command {
        Command::Summary => summary(u),
        Command::Tree {
            root,
            depth,
            visible_only,
            aliases,
        } => {
            let roots: Vec<LocationId> = match root {
                Some(name) => vec![resolve(u, &name, None)?],
                None => u.systems().map(|s| s.id).collect(),
            };
            for r in roots {
                print_tree(u, r, 0, depth, visible_only, aliases);
            }
        }
        Command::Find { text, scope, limit } => {
            let scope = scope.map(|s| resolve(u, &s, None)).transpose()?;
            let matches = u.find_fuzzy(
                &text,
                LookupOptions {
                    scope,
                    limit,
                    min_score: 0.6,
                },
            );
            if matches.is_empty() {
                println!("no match");
            }
            for m in matches {
                let l = u.get(m.id);
                let via = u
                    .matched_alias(&m)
                    .map(|a| format!("  (via {} {:?})", a.kind.label(), a.name))
                    .unwrap_or_default();
                println!(
                    "{:.3}  {:<14} {}{via}",
                    m.score,
                    l.kind.label(),
                    u.path_string(m.id)
                );
            }
        }
        Command::Dist { from, to } => {
            let (a, b) = (resolve(u, &from, None)?, resolve(u, &to, None)?);
            let route = u.route(a, b)?;
            println!("{}  ->  {}", u.path_string(a), u.path_string(b));
            for leg in &route.legs {
                match leg {
                    Leg::Travel { from, to, distance } => println!(
                        "  travel {:>12}  {} -> {}",
                        distance.to_string(),
                        u.get(*from).name,
                        u.get(*to).name
                    ),
                    Leg::Jump { from, to, .. } => {
                        println!(
                            "  jump                {} -> {}",
                            u.get(*from).name,
                            u.get(*to).name
                        )
                    }
                }
            }
            println!("total {} ({} jump(s))", route.total, route.jumps());
        }
        Command::Trading { scope } => {
            let scope = scope.map(|s| resolve(u, &s, None)).transpose()?;
            let mut rows: Vec<String> = u
                .trading_locations()
                .filter(|l| scope.is_none_or(|s| u.is_within(l.id, s)))
                .map(|l| format!("{:<14} {}", l.kind.label(), u.path_string(l.id)))
                .collect();
            rows.sort();
            for r in &rows {
                println!("{r}");
            }
            println!("{} trading locations", rows.len());
        }
        Command::Contract { files, save, raw } => {
            read_contracts(u, &files, save.then(|| source.contracts_dir()), raw)?;
        }
    }
    Ok(())
}

/// Resolves a user-typed name: exact match first (must be unique), then best fuzzy match.
fn resolve(u: &Universe, name: &str, scope: Option<LocationId>) -> Result<LocationId> {
    if let Some(sys) = u.system(name) {
        return Ok(sys.id);
    }
    let exact: Vec<Match> = u
        .find_exact(name)
        .into_iter()
        .filter(|m| scope.is_none_or(|s| u.is_within(m.id, s)))
        .collect();
    // An own-name match wins over locations that merely have it as an alias.
    let own: Vec<&Match> = exact.iter().filter(|m| m.alias.is_none()).collect();
    let candidates: Vec<&Match> = if own.is_empty() {
        exact.iter().collect()
    } else {
        own
    };
    match candidates.as_slice() {
        [one] => {
            note_alias(u, one);
            return Ok(one.id);
        }
        [] => {}
        many => {
            let list: Vec<String> = many.iter().map(|m| u.path_string(m.id)).collect();
            bail!("{name:?} is ambiguous:\n  {}", list.join("\n  "));
        }
    }
    let opts = LookupOptions {
        scope,
        ..LookupOptions::default()
    };
    match u.find_fuzzy(name, opts).first() {
        Some(m) => {
            eprintln!("note: {name:?} -> {} ({:.2})", u.matched_name(m), m.score);
            note_alias(u, m);
            Ok(m.id)
        }
        None => bail!("no location matches {name:?}"),
    }
}

fn note_alias(u: &Universe, m: &Match) {
    if let Some(a) = u.matched_alias(m) {
        eprintln!(
            "note: {:?} is a {} of {}",
            a.name,
            a.kind.label(),
            u.path_string(m.id)
        );
    }
}

fn summary(u: &Universe) {
    let m = u.meta();
    println!("source:   {}", m.source);
    println!(
        "version:  {}",
        m.game_version.as_deref().unwrap_or("unknown")
    );
    println!("revision: {}", m.revision.as_deref().unwrap_or("-"));
    if m.from_cache {
        println!("          (offline: cached data)");
    }
    for sys in u.systems() {
        let mut counts: std::collections::BTreeMap<LocationKind, usize> = Default::default();
        let (mut trading, mut aliases) = (0, 0);
        for l in u.descendants(sys.id).skip(1) {
            *counts.entry(l.kind).or_default() += 1;
            trading += usize::from(l.has_commodity_trading());
            aliases += l.aliases.len();
        }
        let parts: Vec<String> = counts.iter().map(|(k, n)| format!("{n} {k}")).collect();
        println!(
            "\n{} — {} trading locations, {} aliases\n  {}",
            sys.name,
            trading,
            aliases,
            parts.join(", ")
        );
    }
    println!("\njump links:");
    for j in u.jump_links() {
        println!("  {}  <->  {}", u.path_string(j.a), u.path_string(j.b));
    }
}

fn print_tree(
    u: &Universe,
    id: LocationId,
    indent: usize,
    max_depth: usize,
    visible_only: bool,
    aliases: bool,
) {
    let l: &Location = u.get(id);
    if visible_only && l.hidden {
        return;
    }
    let mut flags = String::new();
    if l.has_commodity_trading() {
        flags.push_str(" [trade]");
    }
    if l.hidden {
        flags.push_str(" [hidden]");
    }
    if matches!(l.position, sc_universe::Position::Inherited { .. }) {
        flags.push_str(" [~pos]");
    }
    println!(
        "{}{} ({}){}",
        "  ".repeat(indent),
        l.name,
        l.kind.label(),
        flags
    );
    if aliases {
        for a in &l.aliases {
            println!(
                "{}= {} ({}, {})",
                "  ".repeat(indent + 1),
                a.name,
                a.kind.label(),
                a.source
            );
        }
    }
    if indent < max_depth {
        for c in &l.children {
            print_tree(u, *c, indent + 1, max_depth, visible_only, aliases);
        }
    } else if !l.children.is_empty() {
        println!("{}… {} more", "  ".repeat(indent + 1), l.children.len());
    }
}

fn read_contracts(
    u: &Universe,
    files: &[PathBuf],
    store_dir: Option<PathBuf>,
    raw: bool,
) -> Result<()> {
    let mut reader = ContractReader::new();
    let mut store = store_dir.map(|dir| {
        let (store, warnings) = ContractStore::open(dir);
        for w in warnings {
            eprintln!("warning: {w}");
        }
        store
    });
    for path in files {
        let started = Instant::now();
        let shot = reader
            .read(path, &mut |status| eprintln!("{status}"))
            .with_context(|| format!("reading {}", path.display()))?;
        eprintln!("read {} in {:.1?}", path.display(), started.elapsed());
        if raw {
            print_raw(&shot);
        }
        let entry = match &mut store {
            Some(store) => {
                let added = store.add_shot(shot, u)?;
                let verb = match added {
                    Added::New(_) => "saved as",
                    Added::Merged(_) => "merged into",
                };
                eprintln!("{verb} {}", store.path(added.id()).display());
                store
                    .get(added.id())
                    .cloned()
                    .context("saved contract not found")?
            }
            None => {
                let parsed = contracts::parse(std::slice::from_ref(&shot), u);
                Entry {
                    contract: parsed.contract,
                    shots: vec![shot],
                    reads: parsed.reads,
                    ..Entry::default()
                }
            }
        };
        println!(
            "{}",
            sc_contracts::truth::serialize(&entry.contract.to_objectives())
        );
        for flow in &entry.contract.flows {
            println!("flow: {}", contracts::flow_line(flow));
        }
        let names = store
            .as_ref()
            .map(|s| s.place_names().clone())
            .unwrap_or_default();
        let places = Places::new(u, &names);
        let lk = Lookup {
            universe: u,
            places: &places,
        };
        let r = review::review(&entry, lk);
        for p in &r.problems {
            println!("problem: {p}");
        }
        if !r.unsure.is_empty() {
            println!("check: {} (low-confidence read)", r.unsure.join(", "));
        }
        for n in &r.notes {
            println!("check: {n}");
        }
        for f in &r.fixes {
            println!("fix: use \"{}\" for \"{}\"", f.suggestion, f.written);
        }
        for f in fields::all(&entry.contract) {
            if review::mark(&entry, f, lk) == review::Mark::Bad {
                println!(
                    "missing or unknown: {} \"{}\"",
                    f.label(),
                    fields::text(&entry.contract, f)
                );
            }
        }
        println!();
    }
    Ok(())
}

/// Each region's lines with their engine confidence and box.
fn print_raw(shot: &contracts::ShotText) {
    for region in sc_ocr::Region::ALL {
        println!("--- {}", region.name());
        for (text, b) in shot.lines(region) {
            match b {
                Some(b) => println!(
                    "{:>5} [{},{} {}x{}] {text}",
                    b.conf.map_or("-".to_owned(), |c| format!("{c:.3}")),
                    b.x,
                    b.y,
                    b.w,
                    b.h
                ),
                None => println!("    - {text}"),
            }
        }
    }
    println!("---");
}
