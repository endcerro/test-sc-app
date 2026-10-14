//! SC Companion entry point.
//!
//! Without a subcommand this opens the egui GUI. Subcommands (`summary`, `tree`, `find`,
//! `dist`, `trading`) inspect the universe data from the terminal; `contract` reads contract
//! screenshots.

mod cli;
mod contracts;
mod gui;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use sc_data::{LoadOptions, RemoteConfig, ScunpackedSource};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Star Citizen companion. Opens the GUI; subcommands inspect the universe data."
)]
struct Cli {
    #[command(flatten)]
    source: SourceArgs,
    #[command(subcommand)]
    command: Option<cli::Command>,
}

/// Where and how to load the universe data and the saved contracts. Shared by the GUI and
/// the CLI.
#[derive(Debug, Clone, clap::Args)]
pub struct SourceArgs {
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
    /// Don't derive venue aliases from the game's labels.json (curated aliases stay).
    #[arg(long, global = true)]
    no_label_aliases: bool,
    /// Saved contract list directory (default: `contracts` in the app data dir).
    #[arg(long, global = true)]
    contracts_dir: Option<PathBuf>,
}

impl SourceArgs {
    pub fn contracts_dir(&self) -> PathBuf {
        self.contracts_dir
            .clone()
            .unwrap_or_else(contracts::ContractStore::default_dir)
    }

    pub fn build(&self) -> ScunpackedSource {
        let options = LoadOptions {
            include_asteroid_scatter: self.asteroids,
            include_hidden_unpositioned: self.keep_hidden_unpositioned,
            skip_label_aliases: self.no_label_aliases,
        };
        match &self.data_dir {
            Some(dir) => ScunpackedSource::from_dir(dir),
            None => {
                let mut cfg = RemoteConfig::with_default_cache();
                cfg.git_ref.clone_from(&self.git_ref);
                if let Some(dir) = &self.cache_dir {
                    cfg.cache_dir.clone_from(dir);
                }
                ScunpackedSource::remote(cfg)
            }
        }
        .with_options(options)
    }
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
    match cli.command {
        Some(command) => cli::run(&cli.source, command),
        None => gui::run(cli.source),
    }
}
