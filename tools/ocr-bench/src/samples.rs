//! Sample discovery: screenshots, their region transcripts and contract truth.
//!
//! ```text
//! samples/<stem>.{png,webp,jpg}               screenshot
//! samples/truth/<stem>.txt                    #HAULING truth for that screenshot
//! samples/truth/<base>.txt                    truth for shots <base>-1, <base>-2, ... merged
//! samples/truth/regions/<stem>/<region>.txt   exact text of each region
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::RgbImage;
use image::imageops::{self, FilterType};
use sc_contracts::{HaulingContract, truth};
use sc_ocr::{Region, load_image};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct Shot {
    pub stem: String,
    pub image: RgbImage,
    /// Content hash (file bytes + resize), for the result cache.
    pub hash: String,
    pub region_truth: BTreeMap<Region, String>,
}

/// A contract to score: one or more shots and the truth they should parse to.
#[derive(Debug)]
pub struct Case {
    pub name: String,
    /// Indices into [`Samples::shots`], top of the list first.
    pub shots: Vec<usize>,
    pub truth: HaulingContract,
}

#[derive(Debug, Default)]
pub struct Samples {
    pub shots: Vec<Shot>,
    pub cases: Vec<Case>,
}

fn is_image(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "webp" | "jpg" | "jpeg"
        )
    })
}

pub fn image_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| is_image(p))
        .collect();
    out.sort();
    Ok(out)
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_owned()
}

/// `09-redwind-yela-scroll-2` -> (`09-redwind-yela-scroll`, 2).
fn split_shot_number(stem: &str) -> Option<(&str, u32)> {
    let (base, n) = stem.rsplit_once('-')?;
    n.parse().ok().map(|n| (base, n))
}

/// Loads samples whose stem contains `filter` (if any), optionally resized to `resize`
/// (simulating another game resolution).
pub fn load(dir: &Path, filter: Option<&str>, resize: Option<(u32, u32)>) -> Result<Samples> {
    let truth_dir = dir.join("truth");
    let mut samples = Samples::default();
    for path in image_paths(dir)? {
        let stem = stem(&path);
        if filter.is_some_and(|f| !stem.contains(f)) {
            continue;
        }
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut image = load_image(&path)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        if let Some((w, h)) = resize {
            image = imageops::resize(&image, w, h, FilterType::Lanczos3);
            hasher.update(format!("resize {w}x{h}").as_bytes());
        }
        let hash: String = hasher
            .finalize()
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut region_truth = BTreeMap::new();
        for region in Region::ALL {
            let p = truth_dir
                .join("regions")
                .join(&stem)
                .join(format!("{}.txt", region.name()));
            if let Ok(text) = std::fs::read_to_string(&p) {
                region_truth.insert(region, text);
            }
        }
        samples.shots.push(Shot {
            stem,
            image,
            hash,
            region_truth,
        });
    }

    let read_truth = |name: &str| -> Result<Option<HaulingContract>> {
        let p = truth_dir.join(format!("{name}.txt"));
        match std::fs::read_to_string(&p) {
            Ok(text) => Ok(Some(
                truth::parse(&text).with_context(|| format!("parsing {}", p.display()))?,
            )),
            Err(_) => Ok(None),
        }
    };

    // Single shots with their own truth.
    for (i, shot) in samples.shots.iter().enumerate() {
        if let Some(t) = read_truth(&shot.stem)? {
            samples.cases.push(Case {
                name: shot.stem.clone(),
                shots: vec![i],
                truth: t,
            });
        }
    }
    // Groups `<base>-N` with truth for `<base>`.
    let mut groups: BTreeMap<String, Vec<(u32, usize)>> = BTreeMap::new();
    for (i, shot) in samples.shots.iter().enumerate() {
        if let Some((base, n)) = split_shot_number(&shot.stem) {
            groups.entry(base.to_owned()).or_default().push((n, i));
        }
    }
    for (base, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        if let Some(t) = read_truth(&base)? {
            members.sort();
            samples.cases.push(Case {
                name: base,
                shots: members.into_iter().map(|(_, i)| i).collect(),
                truth: t,
            });
        }
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_numbers() {
        assert_eq!(
            split_shot_number("09-redwind-yela-scroll-2"),
            Some(("09-redwind-yela-scroll", 2))
        );
        assert_eq!(split_shot_number("01-dead-saints-riker"), None);
    }
}
