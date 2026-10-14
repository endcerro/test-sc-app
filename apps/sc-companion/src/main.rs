//! SC Companion entry point.
//!
//! For now this is a CLI to inspect the universe data. The egui/eframe GUI will replace the
//! default command once the first tool lands.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sc_data::{LoadOptions, RemoteConfig, ScunpackedSource, UniverseSource};
use sc_universe::{Leg, Location, LocationId, LocationKind, LookupOptions, Universe};

#[derive(Debug, Parser)]
#[command(version, about = "Star Citizen companion - universe data inspector")]
struct Cli {
    /// Read scunpacked-data files from this directory instead of downloading them.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// scunpacked-data branch, tag or commit to fetch.
    #[arg(long = "ref", global = true, default_value = sc_data::scunpacked::DEFAULT_REF)]
    git_ref: String,
    /// Override the cache directory.
    #[arg(long, global = true)]
    cache_dir: Option<PathBuf>,
    /// Keep hidden asteroid scatter objects.
    #[arg(long, global = true)]
    asteroids: bool,
    /// Keep hidden objects that have neither coordinates nor amenities (mission templates).
    #[arg(long, global = true)]
    keep_hidden_unpositioned: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
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
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,sc_data=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let options = LoadOptions {
        include_asteroid_scatter: cli.asteroids,
        include_hidden_unpositioned: cli.keep_hidden_unpositioned,
    };
    let source = match &cli.data_dir {
        Some(dir) => ScunpackedSource::from_dir(dir),
        None => {
            let mut cfg = RemoteConfig::with_default_cache();
            cfg.git_ref.clone_from(&cli.git_ref);
            if let Some(dir) = &cli.cache_dir {
                cfg.cache_dir.clone_from(dir);
            }
            ScunpackedSource::remote(cfg)
        }
    }
    .with_options(options);

    let started = Instant::now();
    let loaded = source.load().context("loading universe data")?;
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

    match cli.command {
        Command::Summary => summary(u),
        Command::Tree {
            root,
            depth,
            visible_only,
        } => {
            let roots: Vec<LocationId> = match root {
                Some(name) => vec![resolve(u, &name, None)?],
                None => u.systems().map(|s| s.id).collect(),
            };
            for r in roots {
                print_tree(u, r, 0, depth, visible_only);
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
                println!(
                    "{:.3}  {:<14} {}",
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
    }
    Ok(())
}

/// Resolves a user-typed name: exact match first (must be unique), then best fuzzy match.
fn resolve(u: &Universe, name: &str, scope: Option<LocationId>) -> Result<LocationId> {
    if let Some(sys) = u.system(name) {
        return Ok(sys.id);
    }
    let exact: Vec<LocationId> = u
        .find_exact(name)
        .iter()
        .copied()
        .filter(|id| scope.is_none_or(|s| u.is_within(*id, s)))
        .collect();
    match exact.as_slice() {
        [one] => return Ok(*one),
        [] => {}
        many => {
            let list: Vec<String> = many.iter().map(|id| u.path_string(*id)).collect();
            bail!("{name:?} is ambiguous:\n  {}", list.join("\n  "));
        }
    }
    let opts = LookupOptions {
        scope,
        ..LookupOptions::default()
    };
    match u.find_fuzzy(name, opts).first() {
        Some(m) => {
            eprintln!("note: {name:?} -> {} ({:.2})", u.get(m.id).name, m.score);
            Ok(m.id)
        }
        None => bail!("no location matches {name:?}"),
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
        let mut trading = 0;
        for l in u.descendants(sys.id).skip(1) {
            *counts.entry(l.kind).or_default() += 1;
            trading += usize::from(l.has_commodity_trading());
        }
        let parts: Vec<String> = counts.iter().map(|(k, n)| format!("{n} {k}")).collect();
        println!(
            "\n{} — {} trading locations\n  {}",
            sys.name,
            trading,
            parts.join(", ")
        );
    }
    println!("\njump links:");
    for j in u.jump_links() {
        println!("  {}  <->  {}", u.path_string(j.a), u.path_string(j.b));
    }
}

fn print_tree(u: &Universe, id: LocationId, indent: usize, max_depth: usize, visible_only: bool) {
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
    if indent < max_depth {
        for c in &l.children {
            print_tree(u, *c, indent + 1, max_depth, visible_only);
        }
    } else if !l.children.is_empty() {
        println!("{}… {} more", "  ".repeat(indent + 1), l.children.len());
    }
}
