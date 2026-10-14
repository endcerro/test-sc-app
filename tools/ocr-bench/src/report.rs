//! Scoring of grid results and the CSV / Markdown reports.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::Mutex;

use rayon::prelude::*;

use sc_contracts::{HaulingContract, RegionText, parse, score, snap};
use sc_ocr::{Pipeline, Region};
use sc_universe::Universe;

use crate::grid::{CropMode, Unit, UnitResult};
use crate::metrics;
use crate::samples::Samples;

/// A full OCR configuration (everything but the shot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Config {
    pub crop: CropMode,
    pub pipeline: Pipeline,
    pub engine: usize,
}

impl Config {
    pub fn of(u: &Unit) -> Self {
        Self {
            crop: u.crop,
            pipeline: u.pipeline,
            engine: u.engine,
        }
    }

    pub fn label(&self, engines: &[String]) -> String {
        format!(
            "{} | {} | {}",
            engines[self.engine],
            self.crop.name(),
            self.pipeline.id()
        )
    }
}

/// One scored (shot, region, config) row.
#[derive(Debug, Clone)]
pub struct Row {
    pub shot: usize,
    pub region: Region,
    pub config: Config,
    pub cer: f64,
    pub wer: f64,
    pub ms: f64,
    pub error: bool,
}

/// Aggregate of one config on one region over all shots.
#[derive(Debug, Clone, Copy)]
pub struct Agg {
    pub config: Config,
    pub mean_cer: f64,
    pub worst_cer: f64,
    pub mean_wer: f64,
    pub mean_ms: f64,
    pub n: usize,
    pub errors: usize,
}

pub struct Scored {
    pub rows: Vec<Row>,
    /// region -> aggregates sorted best first (mean CER, then worst).
    pub aggs: BTreeMap<Region, Vec<Agg>>,
    /// (shot, config) -> unit result, to rebuild texts for end-to-end scoring.
    pub texts: HashMap<(usize, Config), UnitResult>,
}

pub fn score_all(samples: &Samples, units: &[Unit], results: &[UnitResult]) -> Scored {
    let mut rows = Vec::new();
    let mut texts = HashMap::new();
    for (u, r) in units.iter().zip(results) {
        let shot = &samples.shots[u.shot];
        for &region in u.crop.regions() {
            let Some(truth) = shot.region_truth.get(&region) else {
                continue;
            };
            let pred = r.regions.get(region.name()).map_or("", String::as_str);
            rows.push(Row {
                shot: u.shot,
                region,
                config: Config::of(u),
                cer: metrics::cer(pred, truth),
                wer: metrics::wer(pred, truth),
                ms: r.ms,
                error: r.error.is_some(),
            });
        }
        texts.insert((u.shot, Config::of(u)), r.clone());
    }

    let mut groups: HashMap<(Region, Config), Vec<&Row>> = HashMap::new();
    for row in &rows {
        groups
            .entry((row.region, row.config))
            .or_default()
            .push(row);
    }
    let mut aggs: BTreeMap<Region, Vec<Agg>> = BTreeMap::new();
    for ((region, config), rs) in groups {
        let n = rs.len();
        let mean = |f: fn(&Row) -> f64| rs.iter().map(|r| f(r)).sum::<f64>() / n as f64;
        aggs.entry(region).or_default().push(Agg {
            config,
            mean_cer: mean(|r| r.cer),
            worst_cer: rs.iter().map(|r| r.cer).fold(0.0, f64::max),
            mean_wer: mean(|r| r.wer),
            mean_ms: mean(|r| r.ms),
            n,
            errors: rs.iter().filter(|r| r.error).count(),
        });
    }
    for v in aggs.values_mut() {
        v.sort_by(|a, b| {
            a.mean_cer
                .total_cmp(&b.mean_cer)
                .then(a.worst_cer.total_cmp(&b.worst_cer))
                .then(a.mean_ms.total_cmp(&b.mean_ms))
        });
    }
    Scored { rows, aggs, texts }
}

/// Which config to read each region with.
pub type Choice = BTreeMap<Region, Config>;

/// Result of parsing every case with one [`Choice`].
#[derive(Debug, Clone)]
pub struct E2e {
    pub accuracy: f64,
    pub worst_accuracy: f64,
    pub cargo_f1: f64,
    pub perfect: usize,
    pub cases: usize,
    /// (case name, report) for imperfect cases.
    pub misses: Vec<(String, String)>,
}

pub fn region_text(scored: &Scored, shot: usize, choice: &Choice) -> RegionText {
    let get = |region: Region| -> Option<String> {
        let config = choice.get(&region)?;
        scored
            .texts
            .get(&(shot, *config))
            .and_then(|r| r.regions.get(region.name()).cloned())
    };
    RegionText {
        title: get(Region::Title),
        reward: get(Region::Reward),
        details: get(Region::Details),
        objectives: get(Region::Objectives)
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default(),
    }
}

/// Memoised name snapping (names repeat across thousands of evaluations).
pub struct Snapper<'a> {
    universe: Option<&'a Universe>,
    memo: Mutex<HashMap<(bool, String), String>>,
}

impl<'a> Snapper<'a> {
    pub fn new(universe: Option<&'a Universe>) -> Self {
        Self {
            universe,
            memo: Mutex::new(HashMap::new()),
        }
    }

    pub fn has_universe(&self) -> bool {
        self.universe.is_some()
    }

    fn cached(&self, is_place: bool, name: &str, f: impl FnOnce() -> String) -> String {
        let key = (is_place, name.to_owned());
        if let Some(v) = self.memo.lock().ok().and_then(|m| m.get(&key).cloned()) {
            return v;
        }
        let v = f();
        if let Ok(mut m) = self.memo.lock() {
            m.insert(key, v.clone());
        }
        v
    }

    /// Same as [`snap::contract`], memoised.
    pub fn contract(&self, c: &mut HaulingContract) {
        for cargo in &mut c.cargo {
            cargo.commodity = self.cached(false, &cargo.commodity, || {
                snap::commodity(&cargo.commodity)
                    .map_or_else(|| cargo.commodity.clone(), str::to_owned)
            });
            if let Some(u) = self.universe {
                for p in cargo
                    .pickups
                    .iter_mut()
                    .chain(std::iter::once(&mut cargo.dropoff))
                {
                    if !p.name.is_empty() {
                        p.name = self.cached(true, &p.name, || snap::place(u, &p.name));
                    }
                }
            }
        }
    }
}

impl std::fmt::Debug for Snapper<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapper")
            .field("universe", &self.universe.is_some())
            .finish()
    }
}

/// Parses every case with `choice`; snaps names when `snapper` is given.
pub fn end_to_end(
    samples: &Samples,
    scored: &Scored,
    choice: &Choice,
    snapper: Option<&Snapper<'_>>,
) -> E2e {
    let mut accs = Vec::new();
    let mut f1s = Vec::new();
    let mut misses = Vec::new();
    for case in &samples.cases {
        let shots: Vec<RegionText> = case
            .shots
            .iter()
            .map(|&s| region_text(scored, s, choice))
            .collect();
        let mut pred: HaulingContract = parse::contract_from_shots(&shots);
        if let Some(snapper) = snapper {
            snapper.contract(&mut pred);
        }
        let s = score(&pred, &case.truth);
        accs.push(s.accuracy());
        f1s.push(s.cargo_f1());
        if !s.is_perfect() {
            misses.push((case.name.clone(), score::report(&s)));
        }
    }
    let n = accs.len().max(1) as f64;
    E2e {
        accuracy: accs.iter().sum::<f64>() / n,
        worst_accuracy: accs.iter().copied().fold(1.0, f64::min),
        cargo_f1: f1s.iter().sum::<f64>() / n,
        perfect: samples.cases.len() - misses.len(),
        cases: samples.cases.len(),
        misses,
    }
}

/// Same config for every region; `Lines` configs read the other regions anchored.
pub fn uniform_choice(config: Config) -> Choice {
    Region::ALL
        .into_iter()
        .map(|r| {
            let c = if config.crop == CropMode::Lines && r != Region::Objectives {
                Config {
                    crop: CropMode::Anchored,
                    ..config
                }
            } else {
                config
            };
            (r, c)
        })
        .collect()
}

/// Best config per region by CER, optionally restricted to one engine.
pub fn best_choice(scored: &Scored, engine: Option<usize>) -> Option<Choice> {
    let mut choice = Choice::new();
    for region in Region::ALL {
        let best = scored
            .aggs
            .get(&region)?
            .iter()
            .find(|a| engine.is_none_or(|e| a.config.engine == e) && a.errors == 0)?;
        choice.insert(region, best.config);
    }
    Some(choice)
}

/// Starting from `start`, swaps each region's config for the one with the best end-to-end
/// accuracy (snapped; ties keep the lower CER), two passes. Fitted on the samples, so it is
/// reported next to the CER-based choice rather than instead of it.
pub fn optimise_choice(
    samples: &Samples,
    scored: &Scored,
    start: &Choice,
    engine: Option<usize>,
    snapper: &Snapper<'_>,
) -> Choice {
    let mut choice = start.clone();
    let mut best = end_to_end(samples, scored, &choice, Some(snapper)).accuracy;
    for _ in 0..2 {
        for region in Region::ALL {
            let Some(aggs) = scored.aggs.get(&region) else {
                continue;
            };
            let candidates: Vec<Config> = aggs
                .iter()
                .filter(|a| a.errors == 0 && engine.is_none_or(|e| a.config.engine == e))
                .map(|a| a.config)
                .collect();
            let scores: Vec<f64> = candidates
                .par_iter()
                .map(|c| {
                    let mut candidate = choice.clone();
                    candidate.insert(region, *c);
                    end_to_end(samples, scored, &candidate, Some(snapper)).accuracy
                })
                .collect();
            // First (lowest CER) among the best.
            if let Some((i, &acc)) =
                scores
                    .iter()
                    .enumerate()
                    .fold(None, |acc: Option<(usize, &f64)>, (i, v)| match acc {
                        Some((_, b)) if *b >= *v => acc,
                        _ => Some((i, v)),
                    })
                && acc > best + 1e-9
            {
                best = acc;
                choice.insert(region, candidates[i]);
            }
        }
    }
    choice
}

/// The selections compared end to end: best CER per region and field-optimised, overall
/// and per engine.
pub fn selections(
    samples: &Samples,
    scored: &Scored,
    engines: &[String],
    snapper: &Snapper<'_>,
) -> Vec<(String, Choice)> {
    let mut out = Vec::new();
    let mut add = |label: String, engine: Option<usize>| {
        if let Some(c) = best_choice(scored, engine) {
            let tuned = optimise_choice(samples, scored, &c, engine, snapper);
            out.push((format!("best CER per region, {label}"), c));
            out.push((format!("field-optimised, {label}"), tuned));
        }
    };
    add("any engine".to_owned(), None);
    for (i, name) in engines.iter().enumerate() {
        add(name.clone(), Some(i));
    }
    out
}

pub fn results_csv(samples: &Samples, scored: &Scored, engines: &[String]) -> String {
    let mut out = String::from("shot,region,engine,crop,pipeline,cer,wer,ms,error\n");
    for r in &scored.rows {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{:.4},{:.4},{:.1},{}",
            samples.shots[r.shot].stem,
            r.region.name(),
            engines[r.config.engine],
            r.config.crop.name(),
            r.config.pipeline.id(),
            r.cer,
            r.wer,
            r.ms,
            r.error
        );
    }
    out
}

/// One axis of the grid, for the "effect of each axis" tables.
struct Axis {
    name: &'static str,
    value: fn(&Config, &[String]) -> String,
}

const AXES: &[Axis] = &[
    Axis {
        name: "engine",
        value: |c, e| e[c.engine].clone(),
    },
    Axis {
        name: "crop",
        value: |c, _| c.crop.name().to_owned(),
    },
    Axis {
        name: "colour",
        value: |c, _| format!("{:?}", c.pipeline.color),
    },
    Axis {
        name: "background",
        value: |c, _| format!("{:?}", c.pipeline.background),
    },
    Axis {
        name: "binarise",
        value: |c, _| format!("{:?}", c.pipeline.binarize),
    },
    Axis {
        name: "scale",
        value: |c, _| format!("x{}", c.pipeline.scale),
    },
    Axis {
        name: "despeckle",
        value: |c, _| c.pipeline.despeckle.to_string(),
    },
    Axis {
        name: "invert",
        value: |c, _| c.pipeline.invert.to_string(),
    },
];

/// Measured latency of one selection.
#[derive(Debug, Clone)]
pub struct Latency {
    pub label: String,
    pub per_region: Vec<(Region, f64)>,
}

impl Latency {
    pub fn total(&self) -> f64 {
        self.per_region.iter().map(|(_, ms)| ms).sum()
    }
}

pub struct SummaryInput<'a> {
    pub latency: &'a [Latency],
    pub samples: &'a Samples,
    pub scored: &'a Scored,
    pub engines: &'a [String],
    pub skipped: &'a [(String, String)],
    pub snapper: &'a Snapper<'a>,
    pub selections: &'a [(String, Choice)],
    pub units: usize,
    pub title: &'a str,
}

fn pct(v: f64) -> String {
    format!("{:.1}%", v * 100.0)
}

pub fn summary(input: &SummaryInput<'_>) -> String {
    let SummaryInput {
        latency,
        samples,
        scored,
        engines,
        skipped,
        snapper,
        selections,
        units,
        title,
    } = input;
    let mut md = String::new();
    let _ = writeln!(md, "# OCR benchmark: {title}\n");
    let _ = writeln!(
        md,
        "{} shots, {} cases, {} engine configs, {} units, {} scored rows. Universe for place \
         snapping: {}.\n",
        samples.shots.len(),
        samples.cases.len(),
        engines.len(),
        units,
        scored.rows.len(),
        if snapper.has_universe() { "yes" } else { "no" }
    );
    if !skipped.is_empty() {
        let _ = writeln!(md, "Skipped engines:\n");
        for (name, why) in *skipped {
            let _ = writeln!(md, "- `{name}`: {why}");
        }
        md.push('\n');
    }

    // ---- end to end -------------------------------------------------------------------
    let _ = writeln!(md, "## End to end (parsed contract vs `#HAULING` truth)\n");
    let _ = writeln!(
        md,
        "Accuracy = share of scalar fields and cargo columns right; F1 over exact cargo lines. \
         \"best CER per region\" picks the lowest-CER config for each region independently; \
         \"field-optimised\" then swaps each region's config to maximise end-to-end accuracy \
         on these samples (fitted on them, so optimistic).\n"
    );
    let _ = writeln!(
        md,
        "| Selection | Snap | Accuracy | Worst case | Cargo F1 | Perfect |\n|---|---|---|---|---|---|"
    );
    let mut winner: Option<(f64, String, Choice, bool)> = None;
    for (name, choice) in *selections {
        for snap_names in [false, true] {
            let e = end_to_end(samples, scored, choice, snap_names.then_some(*snapper));
            let _ = writeln!(
                md,
                "| {name} | {} | {} | {} | {:.3} | {}/{} |",
                if snap_names { "yes" } else { "no" },
                pct(e.accuracy),
                pct(e.worst_accuracy),
                e.cargo_f1,
                e.perfect,
                e.cases
            );
            if winner.as_ref().is_none_or(|w| e.accuracy > w.0) {
                winner = Some((e.accuracy, name.clone(), choice.clone(), snap_names));
            }
        }
    }
    md.push('\n');

    let _ = writeln!(
        md,
        "<details><summary>Configs of each selection</summary>\n"
    );
    for (name, choice) in *selections {
        let _ = writeln!(md, "- **{name}**");
        for (region, c) in choice {
            let _ = writeln!(md, "  - {}: `{}`", region.name(), c.label(engines));
        }
    }
    let _ = writeln!(md, "\n</details>\n");

    // Uniform configurations.
    let mut configs: Vec<Config> = scored
        .aggs
        .values()
        .flat_map(|v| v.iter().map(|a| a.config))
        .collect();
    configs.sort();
    configs.dedup();
    let mut uniform: Vec<(Config, E2e)> = configs
        .par_iter()
        .map(|c| {
            (
                *c,
                end_to_end(samples, scored, &uniform_choice(*c), Some(*snapper)),
            )
        })
        .collect();
    uniform.sort_by(|a, b| {
        b.1.accuracy
            .total_cmp(&a.1.accuracy)
            .then(b.1.worst_accuracy.total_cmp(&a.1.worst_accuracy))
    });
    let _ = writeln!(
        md,
        "### Single configuration for all regions (snapped), top 15\n\n| Config | Accuracy | Worst | Cargo F1 | Perfect |\n|---|---|---|---|---|"
    );
    for (c, e) in uniform.iter().take(15) {
        let _ = writeln!(
            md,
            "| `{}` | {} | {} | {:.3} | {}/{} |",
            c.label(engines),
            pct(e.accuracy),
            pct(e.worst_accuracy),
            e.cargo_f1,
            e.perfect,
            e.cases
        );
    }
    md.push('\n');

    if let Some((acc, name, choice, snap_names)) = &winner {
        let _ = writeln!(
            md,
            "### Winner: {name} (snap: {snap_names}), accuracy {}\n",
            pct(*acc)
        );
        for (region, c) in choice {
            let _ = writeln!(md, "- {}: `{}`", region.name(), c.label(engines));
        }
        let e = end_to_end(samples, scored, choice, snap_names.then_some(*snapper));
        if e.misses.is_empty() {
            let _ = writeln!(md, "\nAll cases parse exactly to the truth.");
        } else {
            let _ = writeln!(md, "\nRemaining differences:\n\n```text");
            for (case, report) in &e.misses {
                let _ = writeln!(md, "{case}: {report}");
            }
            let _ = writeln!(md, "```");
        }
        md.push('\n');
    }

    // ---- per region ---------------------------------------------------------------------
    for (region, aggs) in &scored.aggs {
        let _ = writeln!(md, "## Region `{}`\n", region.name());
        let _ = writeln!(
            md,
            "| # | Config | Mean CER | Worst CER | Mean WER | Shots |\n|---|---|---|---|---|---|"
        );
        for (i, a) in aggs.iter().take(15).enumerate() {
            let _ = writeln!(
                md,
                "| {} | `{}` | {} | {} | {} | {} |",
                i + 1,
                a.config.label(engines),
                pct(a.mean_cer),
                pct(a.worst_cer),
                pct(a.mean_wer),
                a.n
            );
        }
        md.push('\n');

        let _ = writeln!(
            md,
            "Best per engine:\n\n| Engine | Config | Mean CER | Worst CER |\n|---|---|---|---|"
        );
        for (i, name) in engines.iter().enumerate() {
            if let Some(a) = aggs.iter().find(|a| a.config.engine == i) {
                let _ = writeln!(
                    md,
                    "| {name} | `{} | {}` | {} | {} |",
                    a.config.crop.name(),
                    a.config.pipeline.id(),
                    pct(a.mean_cer),
                    pct(a.worst_cer)
                );
            }
        }
        md.push('\n');

        let _ = writeln!(
            md,
            "Effect of each axis (mean CER over all configs with that value / best config with it):\n"
        );
        for axis in AXES {
            let mut by: BTreeMap<String, (f64, usize, f64)> = BTreeMap::new();
            for a in aggs {
                let e = by
                    .entry((axis.value)(&a.config, engines))
                    .or_insert((0.0, 0, f64::MAX));
                e.0 += a.mean_cer;
                e.1 += 1;
                e.2 = e.2.min(a.mean_cer);
            }
            let cells: Vec<String> = by
                .iter()
                .map(|(v, (sum, n, best))| format!("{v} {} / {}", pct(sum / *n as f64), pct(*best)))
                .collect();
            let _ = writeln!(md, "- **{}**: {}", axis.name, cells.join(", "));
        }
        md.push('\n');
    }

    // ---- latency -------------------------------------------------------------------------
    let _ = writeln!(
        md,
        "## Latency (ms per screenshot, sequential, warm)\n\n\
         Each region read one screenshot at a time; engines may use several cores internally \
         (Paddle sessions are single-threaded except the server detector).\n\n\
         | Selection | Title | Reward | Details | Objectives | Total |\n|---|---|---|---|---|---|"
    );
    for row in *latency {
        let cell = |r: Region| {
            row.per_region
                .iter()
                .find(|(region, _)| *region == r)
                .map_or("-".to_owned(), |(_, ms)| format!("{ms:.0}"))
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {:.0} |",
            row.label,
            cell(Region::Title),
            cell(Region::Reward),
            cell(Region::Details),
            cell(Region::Objectives),
            row.total()
        );
    }
    md
}
