//! OCR benchmark over `samples/`: every engine x preprocessing pipeline x crop mode,
//! scored per region (CER against exact transcripts) and end to end (parsed contract
//! against the `#HAULING` truth).
//!
//! ```sh
//! cargo run -p ocr-bench --release --features all-engines -- fetch-models
//! cargo run -p ocr-bench --release --features all-engines -- run --preset quick
//! cargo run -p ocr-bench --release --features all-engines -- run --universe
//! cargo run -p ocr-bench --release --features all-engines -- run --universe --preset full --name full
//! cargo run -p ocr-bench --release -- dump --sample 05
//! ```

mod engines;
mod grid;
mod metrics;
mod report;
mod samples;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sc_data::UniverseSource;
use sc_ocr::models::{self, ModelDir};
use sc_ocr::{Layout, Pipeline, Region, crop, layout};

use crate::grid::CropMode;

#[derive(Debug, Parser)]
#[command(about = "Benchmark OCR engines and preprocessing pipelines on contract screenshots.")]
struct Cli {
    /// Samples directory (screenshots + truth/).
    #[arg(long, global = true, default_value = "samples")]
    samples: PathBuf,
    /// Output directory (reports, cache).
    #[arg(long, global = true, default_value = "target/ocr-bench")]
    out: PathBuf,
    /// Model directory. Default: the app cache dir (`.../sc-companion/models`).
    #[arg(long, global = true)]
    models: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Download the model files (the only step that needs network).
    FetchModels,
    /// Write region crops, preprocessed crops and bullet line crops for inspection.
    Dump {
        /// Only samples whose file name contains this.
        #[arg(long)]
        sample: Option<String>,
        /// Pipeline id to apply (as printed in reports), e.g. `whiteblue/-/x2/otsu/-/inv`.
        #[arg(long, default_value = "whiteblue/-/x2/otsu/-/inv")]
        pipeline: String,
    },
    /// Run the grid and write `results.csv` + `summary.md`.
    Run(RunArgs),
    /// Sanity check: snapping the truth contracts to the universe must not change them.
    CheckSnap,
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    /// Run name (sub-directory of --out).
    #[arg(long, default_value = "latest")]
    name: String,
    /// How much of the grid to run: quick (minutes), reduced (~20-30 min), full (hours).
    #[arg(long, value_enum, default_value_t = grid::Preset::Reduced)]
    preset: grid::Preset,
    /// Engine configs whose name contains one of these (comma separated).
    #[arg(long, value_delimiter = ',')]
    engines: Vec<String>,
    /// Only these crop modes: full, fixed, anchored, lines (comma separated).
    #[arg(long, value_delimiter = ',')]
    crops: Vec<String>,
    /// Only pipelines whose id contains all of these (comma separated).
    #[arg(long, value_delimiter = ',')]
    pipelines: Vec<String>,
    /// Only samples whose name contains this.
    #[arg(long)]
    sample: Option<String>,
    /// Resize screenshots to WxH first (e.g. 2560x1440) to test other resolutions.
    #[arg(long)]
    resize: Option<String>,
    /// Worker threads (default: all cores).
    #[arg(long)]
    threads: Option<usize>,
    /// Load the universe (scunpacked-data, cached) to snap place names.
    #[arg(long)]
    universe: bool,
    /// Skip the sequential latency measurement.
    #[arg(long)]
    no_timing: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let model_dir = ModelDir::new(cli.models.clone().unwrap_or_else(models::default_dir));
    match &cli.command {
        Command::FetchModels => fetch_models(&model_dir),
        Command::Dump { sample, pipeline } => dump(&cli, sample.as_deref(), pipeline),
        Command::Run(args) => run(&cli, &model_dir, args),
        Command::CheckSnap => check_snap(&cli),
    }
}

fn fetch_models(dir: &ModelDir) -> Result<()> {
    println!("model dir: {}", dir.root().display());
    models::fetch(dir, models::ALL, |file| {
        println!("downloading {} ...", file.url);
    })?;
    for file in models::ALL {
        let hash = models::sha256_hex(&std::fs::read(dir.path(file))?);
        println!("  {:32} {hash}", file.name);
    }
    Ok(())
}

fn load_universe() -> Result<sc_universe::Universe> {
    let loaded = sc_data::ScunpackedSource::remote(sc_data::RemoteConfig::with_default_cache())
        .load()
        .context("loading universe")?;
    for n in &loaded.notes {
        eprintln!("universe: {n}");
    }
    Ok(loaded.universe)
}

fn check_snap(cli: &Cli) -> Result<()> {
    let universe = load_universe()?;
    let samples = samples::load(&cli.samples, None, None)?;
    let mut changed = 0;
    for case in &samples.cases {
        let mut snapped = case.truth.clone();
        sc_contracts::snap::contract(&mut snapped, Some(&universe));
        let s = sc_contracts::score(&snapped, &case.truth);
        if !s.is_perfect() {
            changed += 1;
            println!("{}: {}", case.name, sc_contracts::score::report(&s));
        }
    }
    println!(
        "{changed}/{} truth contracts changed by snapping",
        samples.cases.len()
    );
    Ok(())
}

fn parse_pipeline(id: &str) -> Result<Pipeline> {
    Pipeline::grid()
        .into_iter()
        .find(|p| p.id() == id)
        .with_context(|| format!("unknown pipeline `{id}`"))
}

fn dump(cli: &Cli, filter: Option<&str>, pipeline: &str) -> Result<()> {
    let p = parse_pipeline(pipeline)?;
    let dir = cli.out.join("dump");
    std::fs::create_dir_all(&dir)?;
    for path in samples::image_paths(&cli.samples)? {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_owned();
        if filter.is_some_and(|f| !stem.contains(f)) {
            continue;
        }
        let img = sc_ocr::load_image(&path)?;
        let l = Layout::anchored(&img);
        println!(
            "{stem}: {}x{} shift ({:.0}, {:.0})",
            img.width(),
            img.height(),
            l.shift.0,
            l.shift.1
        );
        for region in Region::ALL {
            let c = crop(&img, l.rect(region, grid::REGION_MARGIN));
            c.save(dir.join(format!("{stem}.{}.png", region.name())))?;
            std::fs::write(
                dir.join(format!("{stem}.{}.pp.png", region.name())),
                p.apply(&c).to_png()?,
            )?;
        }
        let area = l.rect(Region::Objectives, grid::REGION_MARGIN);
        let bullets = layout::bullets(&img, area, l.scale);
        let lines = layout::bullet_lines(&bullets, area, l.scale);
        for (i, (kind, r)) in lines.into_iter().enumerate() {
            crop(&img, r).save(dir.join(format!("{stem}.line{i:02}-{kind:?}.png")))?;
        }
        println!("  {} bullets", bullets.len());
    }
    println!("wrote {}", dir.display());
    Ok(())
}

/// Latency of each engine with no preprocessing (anchored crops) and of every compared
/// selection (best config per region).
fn measure_latency(
    samples: &samples::Samples,
    selections: &[(String, report::Choice)],
    engines: &[std::sync::Arc<dyn sc_ocr::OcrEngine>],
    names: &[String],
) -> Vec<report::Latency> {
    let mut out = Vec::new();
    for (i, engine) in engines.iter().enumerate() {
        let per_region = Region::ALL
            .into_iter()
            .map(|r| {
                let ms = grid::time_region(
                    &samples.shots,
                    CropMode::Anchored,
                    &Pipeline::RAW,
                    engine.as_ref(),
                    r,
                );
                (r, ms)
            })
            .collect();
        out.push(report::Latency {
            label: format!("{}, raw anchored crops", names[i]),
            per_region,
        });
    }
    for (label, choice) in selections {
        let per_region = choice
            .iter()
            .map(|(&r, c)| {
                // Full-frame configs are timed as anchored crops (same engine work per region).
                let crop_mode = if c.crop == CropMode::Full {
                    CropMode::Anchored
                } else {
                    c.crop
                };
                let ms = grid::time_region(
                    &samples.shots,
                    crop_mode,
                    &c.pipeline,
                    engines[c.engine].as_ref(),
                    r,
                );
                (r, ms)
            })
            .collect();
        out.push(report::Latency {
            label: label.clone(),
            per_region,
        });
    }
    out
}

/// Words for Tesseract's `--user-words`: commodity and place names, split into words.
fn write_user_words(path: &Path, universe: Option<&sc_universe::Universe>) -> Result<()> {
    let mut words = BTreeSet::new();
    let mut add = |s: &str| {
        for w in s.split_whitespace() {
            let w = w.trim_matches(|c: char| !c.is_alphanumeric());
            if w.len() > 1 {
                words.insert(w.to_owned());
            }
        }
    };
    for c in sc_contracts::snap::commodities() {
        add(c);
    }
    if let Some(u) = universe {
        for loc in u.locations() {
            add(&loc.name);
            for a in &loc.aliases {
                add(&a.name);
            }
        }
    }
    for w in [
        "Deliver",
        "Collect",
        "SCU",
        "Reputation",
        "Awarded",
        "Freight",
        "elevator",
    ] {
        add(w);
    }
    let text: Vec<String> = words.into_iter().collect();
    std::fs::write(path, text.join("\n") + "\n")?;
    Ok(())
}

fn run(cli: &Cli, model_dir: &ModelDir, args: &RunArgs) -> Result<()> {
    if let Some(t) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(t)
            .build_global()
            .ok();
    }
    let threads = rayon::current_num_threads();
    let compiled = engines::compiled();
    if compiled.is_empty() {
        bail!(
            "no engine compiled in: build with `--features all-engines` (or paddle, ocrs, winocr, tesseract)"
        );
    }
    let out = cli.out.join(&args.name);
    std::fs::create_dir_all(&out)?;

    let resize = args
        .resize
        .as_deref()
        .map(|s| -> Result<(u32, u32)> {
            let (w, h) = s.split_once('x').context("--resize expects WxH")?;
            Ok((w.parse()?, h.parse()?))
        })
        .transpose()?;
    let samples = samples::load(&cli.samples, args.sample.as_deref(), resize)?;
    eprintln!(
        "{} shots, {} cases from {}",
        samples.shots.len(),
        samples.cases.len(),
        cli.samples.display()
    );

    let universe = if args.universe {
        Some(load_universe()?)
    } else {
        None
    };
    let user_words = out.join("user-words.txt");
    write_user_words(&user_words, universe.as_ref())?;

    let set = engines::build(
        &args.engines,
        model_dir,
        args.preset,
        threads,
        Some(&user_words),
    );
    for (name, why) in &set.skipped {
        eprintln!("skipping {name}: {why}");
    }
    if set.engines.is_empty() {
        bail!("no engine available (compiled: {compiled:?}); run `fetch-models` first?");
    }
    let engine_names: Vec<String> = set.engines.iter().map(|e| e.name()).collect();
    eprintln!("engines: {engine_names:?}");

    let crops: Vec<CropMode> = args
        .crops
        .iter()
        .map(|c| CropMode::from_name(c).with_context(|| format!("unknown crop mode `{c}`")))
        .collect::<Result<_>>()?;
    let plan: Vec<(CropMode, Vec<Pipeline>)> = args
        .preset
        .plan()
        .into_iter()
        .filter(|(c, _)| crops.is_empty() || crops.contains(c))
        .map(|(c, ps)| {
            let ps = ps
                .into_iter()
                .filter(|p| args.pipelines.iter().all(|f| p.id().contains(f.as_str())))
                .collect();
            (c, ps)
        })
        .collect();

    let units = grid::units(samples.shots.len(), &plan, set.engines.len());
    let cache = grid::Cache::open(&cli.out.join("cache.jsonl"))?;
    let results = grid::run(&samples.shots, &units, &set.engines, &cache);

    eprintln!("scoring...");
    let scored = report::score_all(&samples, &units, &results);
    std::fs::write(
        out.join("results.csv"),
        report::results_csv(&samples, &scored, &engine_names),
    )?;
    eprintln!("choosing configs...");
    let snapper = report::Snapper::new(universe.as_ref());
    let selections = report::selections(&samples, &scored, &engine_names, &snapper);
    let latency = if args.no_timing {
        Vec::new()
    } else {
        eprintln!("timing...");
        measure_latency(&samples, &selections, &set.engines, &engine_names)
    };
    let title = format!(
        "{} ({:?} preset){}",
        args.name,
        args.preset,
        resize.map_or(String::new(), |(w, h)| format!(" @ {w}x{h}"))
    );
    let md = report::summary(&report::SummaryInput {
        latency: &latency,
        samples: &samples,
        scored: &scored,
        engines: &engine_names,
        skipped: &set.skipped,
        snapper: &snapper,
        selections: &selections,
        units: units.len(),
        title: &title,
    });
    std::fs::write(out.join("summary.md"), &md)?;
    println!("wrote {}", out.join("summary.md").display());
    Ok(())
}
