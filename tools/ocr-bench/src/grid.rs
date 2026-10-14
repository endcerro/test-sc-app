//! The benchmark grid: (shot x crop mode x pipeline x engine) units, run in parallel with an
//! on-disk result cache.

use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::Result;
use rayon::prelude::*;
use sc_ocr::preprocess::{Background, Binarize, ColorMode};
use sc_ocr::{Layout, OcrEngine, OcrOutput, Pipeline, Region, crop, layout};
use serde::{Deserialize, Serialize};

use crate::samples::Shot;

/// Bump when the meaning of a cached result changes (layout, pipelines, adapters).
const CACHE_VERSION: &str = "v3";

pub use sc_ocr::screen::REGION_MARGIN;

/// How the screenshot is cut before OCR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CropMode {
    /// Whole screenshot in one call; lines are assigned to regions by their box centre.
    Full,
    /// Regions at their reference position (no anchor search).
    Fixed,
    /// Regions shifted by the measured position of the OFFERS tab.
    Anchored,
    /// Objectives only: one crop per yellow bullet (anchored), read without detection
    /// where the engine supports it.
    Lines,
}

impl CropMode {
    pub const ALL: [Self; 4] = [Self::Full, Self::Fixed, Self::Anchored, Self::Lines];

    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Fixed => "fixed",
            Self::Anchored => "anchored",
            Self::Lines => "lines",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    /// Regions this mode produces text for.
    pub fn regions(self) -> &'static [Region] {
        match self {
            Self::Lines => &[Region::Objectives],
            _ => &Region::ALL,
        }
    }
}

/// One OCR job: everything needed to produce text for the regions of one shot.
#[derive(Debug, Clone, Copy)]
pub struct Unit {
    pub shot: usize,
    pub crop: CropMode,
    pub pipeline: Pipeline,
    pub engine: usize,
}

/// Text per region (by name) and wall time of the whole unit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnitResult {
    pub regions: BTreeMap<String, String>,
    pub ms: f64,
    pub error: Option<String>,
}

/// How much of the grid to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Preset {
    /// A coarse subset for iterating on code (a few minutes).
    Quick,
    /// Every axis tested at a fraction of the cost (~20-30 min): the cross-product of the
    /// axes that matter most, one-at-a-time checks for the costly or minor values, and
    /// fixed/full crops on baselines only.
    Reduced,
    /// Every permutation of every axis, crop mode and engine config (many hours; cached
    /// and resumable).
    Full,
}

fn pipeline(id: &str) -> Option<Pipeline> {
    Pipeline::grid().into_iter().find(|p| p.id() == id)
}

/// Baselines that the reduced preset varies one axis at a time.
const ABLATION_BASES: [&str; 4] = [
    "rgb/-/x2/-/-/native",
    "gray/-/x2/-/-/inv",
    "max/-/x2/-/-/native",
    "whiteblue/-/x2/otsu/-/inv",
];

/// Pipelines run on fixed and full-frame crops in the reduced preset.
const CROP_BASELINES: [&str; 3] = [
    "rgb/-/x1/-/-/native",
    "gray/-/x1/-/-/inv",
    "whiteblue/-/x1/otsu/-/inv",
];

impl Preset {
    /// Which pipelines each crop mode is run with.
    pub fn plan(self) -> Vec<(CropMode, Vec<Pipeline>)> {
        match self {
            Self::Quick => {
                let p: Vec<Pipeline> = Pipeline::grid()
                    .into_iter()
                    .filter(|p| {
                        matches!(
                            p.color,
                            ColorMode::Rgb | ColorMode::Gray | ColorMode::WhiteBlueMask
                        ) && p.background == Background::None
                            && matches!(p.binarize, Binarize::None | Binarize::Otsu)
                            && p.scale <= 2
                            && !p.despeckle
                    })
                    .collect();
                vec![(CropMode::Anchored, p.clone()), (CropMode::Lines, p)]
            }
            Self::Reduced => {
                let mut p: Vec<Pipeline> = Pipeline::grid()
                    .into_iter()
                    .filter(|p| {
                        matches!(p.binarize, Binarize::None | Binarize::Otsu)
                            && p.scale <= 2
                            && !p.despeckle
                    })
                    .collect();
                for base in ABLATION_BASES.iter().filter_map(|id| pipeline(id)) {
                    let variants = [
                        Pipeline { scale: 3, ..base },
                        Pipeline {
                            binarize: Binarize::Sauvola,
                            ..base
                        },
                        Pipeline {
                            despeckle: true,
                            ..base
                        },
                    ];
                    p.extend(variants.into_iter().filter(|v| pipeline(&v.id()).is_some()));
                }
                p.sort();
                p.dedup();
                let baselines: Vec<Pipeline> = CROP_BASELINES
                    .iter()
                    .filter_map(|id| pipeline(id))
                    .collect();
                vec![
                    (CropMode::Anchored, p.clone()),
                    (CropMode::Lines, p),
                    (CropMode::Fixed, baselines.clone()),
                    (CropMode::Full, baselines),
                ]
            }
            Self::Full => CropMode::ALL
                .into_iter()
                .map(|c| (c, Pipeline::grid()))
                .collect(),
        }
    }
}

pub fn units(shots: usize, plan: &[(CropMode, Vec<Pipeline>)], engines: usize) -> Vec<Unit> {
    let mut out = Vec::new();
    for shot in 0..shots {
        for (crop, pipelines) in plan {
            let crop = *crop;
            for pipeline in pipelines {
                // Full-frame at 2x/3x is 4-9x the pixels of a 1080p screenshot: beyond some
                // engines' limits and not a realistic option, so only 1x is measured.
                if crop == CropMode::Full && pipeline.scale > 1 {
                    continue;
                }
                for engine in 0..engines {
                    out.push(Unit {
                        shot,
                        crop,
                        pipeline: *pipeline,
                        engine,
                    });
                }
            }
        }
    }
    out
}

/// Append-only JSON-lines cache keyed by unit identity.
pub struct Cache {
    map: HashMap<String, UnitResult>,
    file: Mutex<BufWriter<File>>,
}

#[derive(Serialize, Deserialize)]
struct CacheLine {
    key: String,
    #[serde(flatten)]
    result: UnitResult,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        let mut map = HashMap::new();
        if let Ok(f) = File::open(path) {
            for line in BufReader::new(f).lines() {
                let line = line?;
                if let Ok(c) = serde_json::from_str::<CacheLine>(&line) {
                    map.insert(c.key, c.result);
                }
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            map,
            file: Mutex::new(BufWriter::new(file)),
        })
    }

    pub fn key(shot: &Shot, unit: &Unit, engine: &dyn OcrEngine) -> String {
        format!(
            "{CACHE_VERSION}|{}|{}|{}|{}",
            shot.hash,
            unit.crop.name(),
            unit.pipeline.id(),
            engine.name()
        )
    }

    pub fn get(&self, key: &str) -> Option<&UnitResult> {
        self.map.get(key)
    }

    fn put(&self, key: String, result: &UnitResult) {
        let line = CacheLine {
            key,
            result: result.clone(),
        };
        if let (Ok(json), Ok(mut f)) = (serde_json::to_string(&line), self.file.lock()) {
            let _ = writeln!(f, "{json}");
        }
    }

    pub fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

/// Reads one region with a region-level crop mode (not [`CropMode::Full`]).
pub fn read_region(
    img: &image::RgbImage,
    anchored: &Layout,
    crop_mode: CropMode,
    pipeline: &Pipeline,
    engine: &dyn OcrEngine,
    region: Region,
) -> Result<String, sc_ocr::OcrError> {
    match crop_mode {
        CropMode::Lines if region == Region::Objectives => {
            let area = anchored.rect(Region::Objectives, REGION_MARGIN);
            let bullets = layout::bullets(img, area, anchored.scale);
            let mut text = Vec::new();
            for (_, r) in layout::bullet_lines(&bullets, area, anchored.scale) {
                let out: OcrOutput = engine.recognize_lines(&pipeline.apply(&crop(img, r)))?;
                text.push(out.text());
            }
            Ok(text.join(
                "
",
            ))
        }
        CropMode::Fixed => {
            let l = Layout::fixed(img.width(), img.height());
            let processed = pipeline.apply(&crop(img, l.rect(region, REGION_MARGIN)));
            Ok(engine.recognize(&processed)?.text())
        }
        _ => sc_ocr::screen::read_region(img, anchored, pipeline, engine, region),
    }
}

/// Runs one unit.
pub fn run_unit(shot: &Shot, unit: &Unit, engine: &dyn OcrEngine) -> UnitResult {
    let start = Instant::now();
    let mut result = UnitResult::default();
    let img = &shot.image;
    let anchored = Layout::anchored(img);
    let outcome: Result<(), sc_ocr::OcrError> = (|| {
        match unit.crop {
            CropMode::Full => {
                let full = sc_ocr::Rect::new(0, 0, img.width(), img.height());
                let processed = unit.pipeline.apply(&crop(img, full));
                let out = engine.recognize(&processed)?;
                for region in Region::ALL {
                    let r = anchored.rect(region, REGION_MARGIN);
                    let lines: Vec<&str> = out
                        .lines
                        .iter()
                        .filter(|l| {
                            l.rect.is_some_and(|b| {
                                let (cx, cy) = b.centre();
                                let (sx, sy) = unit.pipeline.to_source(cx, cy);
                                r.contains_point(sx, sy)
                            })
                        })
                        .map(|l| l.text.as_str())
                        .collect();
                    result
                        .regions
                        .insert(region.name().to_owned(), lines.join("\n"));
                }
            }
            crop_mode => {
                for &region in crop_mode.regions() {
                    let text =
                        read_region(img, &anchored, crop_mode, &unit.pipeline, engine, region)?;
                    result.regions.insert(region.name().to_owned(), text);
                }
            }
        }
        Ok(())
    })();
    if let Err(e) = outcome {
        result.error = Some(e.to_string());
    }
    result.ms = start.elapsed().as_secs_f64() * 1000.0;
    result
}

/// Runs all units (cached ones are reused) and returns results in unit order.
pub fn run(
    shots: &[Shot],
    units: &[Unit],
    engines: &[std::sync::Arc<dyn OcrEngine>],
    cache: &Cache,
) -> Vec<UnitResult> {
    let total = units.len();
    let todo = units
        .iter()
        .filter(|u| {
            cache
                .get(&Cache::key(&shots[u.shot], u, engines[u.engine].as_ref()))
                .is_none()
        })
        .count();
    eprintln!("{total} units, {} cached, {todo} to run", total - todo);
    let done = AtomicUsize::new(0);
    let start = Instant::now();
    let mut results: Vec<Option<UnitResult>> = vec![None; units.len()];
    // One engine at a time: engines with their own internal parallelism (rten) do not
    // compete with each other, and progress is easier to follow.
    for (e, engine) in engines.iter().enumerate() {
        let idx: Vec<usize> = (0..units.len()).filter(|&i| units[i].engine == e).collect();
        let batch: Vec<(usize, UnitResult)> = idx
            .par_iter()
            .map(|&i| {
                let u = &units[i];
                let shot = &shots[u.shot];
                let key = Cache::key(shot, u, engine.as_ref());
                if let Some(r) = cache.get(&key) {
                    return (i, r.clone());
                }
                let r = run_unit(shot, u, engine.as_ref());
                cache.put(key, &r);
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(500) || n == todo {
                    let secs = start.elapsed().as_secs_f64();
                    let eta = secs / n as f64 * (todo - n) as f64;
                    eprintln!(
                        "  {n}/{todo} ({:.0}%), {:.0}s elapsed, ~{:.0}s left [{}]",
                        100.0 * n as f64 / todo as f64,
                        secs,
                        eta,
                        engine.name()
                    );
                    cache.flush();
                }
                (i, r)
            })
            .collect();
        for (i, r) in batch {
            results[i] = Some(r);
        }
        cache.flush();
    }
    let results: Vec<UnitResult> = results.into_iter().map(Option::unwrap_or_default).collect();
    results
}

/// Wall time to read one region of every shot with `config`, one shot at a time (engines
/// may still parallelise internally, as they would in the app). Mean ms per shot, after
/// one warm-up call.
pub fn time_region(
    shots: &[Shot],
    crop_mode: CropMode,
    pipeline: &Pipeline,
    engine: &dyn OcrEngine,
    region: Region,
) -> f64 {
    let read = |shot: &Shot| {
        let anchored = Layout::anchored(&shot.image);
        let _ = read_region(&shot.image, &anchored, crop_mode, pipeline, engine, region);
    };
    if let Some(first) = shots.first() {
        read(first);
    }
    let start = Instant::now();
    for shot in shots {
        read(shot);
    }
    start.elapsed().as_secs_f64() * 1000.0 / shots.len().max(1) as f64
}
