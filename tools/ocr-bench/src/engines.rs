//! Builds the engine configurations under test from the compiled-in features.

use std::path::Path;
use std::sync::Arc;

use sc_ocr::OcrEngine;
use sc_ocr::models::ModelDir;

use crate::grid::Preset;

/// Engines that loaded, and the ones that could not (with the reason).
pub struct EngineSet {
    pub engines: Vec<Arc<dyn OcrEngine>>,
    pub skipped: Vec<(String, String)>,
}

impl std::fmt::Debug for EngineSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self.engines.iter().map(|e| e.name()).collect();
        f.debug_struct("EngineSet")
            .field("engines", &names)
            .field("skipped", &self.skipped)
            .finish()
    }
}

/// Engine families compiled into this binary.
pub fn compiled() -> Vec<&'static str> {
    let mut v = Vec::new();
    if cfg!(feature = "paddle") {
        v.push("paddle");
    }
    if cfg!(feature = "ocrs") {
        v.push("ocrs");
    }
    if cfg!(all(feature = "winocr", windows)) {
        v.push("winocr");
    }
    if cfg!(feature = "tesseract") {
        v.push("tesseract");
    }
    v
}

/// Loads every configuration whose name contains one of `filters` (all when empty).
/// `preset` picks the configurations: one per family (quick), the ones that differ in kind
/// (reduced), or all (full).
#[allow(unused_variables, unused_mut)]
pub fn build(
    filters: &[String],
    models: &ModelDir,
    preset: Preset,
    threads: usize,
    user_words: Option<&Path>,
) -> EngineSet {
    let wanted =
        |name: &str| filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str()));
    let mut set = EngineSet {
        engines: Vec::new(),
        skipped: Vec::new(),
    };
    let mut add = |name: String, r: Result<Arc<dyn OcrEngine>, sc_ocr::OcrError>| match r {
        Ok(e) => set.engines.push(e),
        Err(e) => set.skipped.push((name, e.to_string())),
    };

    #[cfg(feature = "paddle")]
    {
        use sc_ocr::engine::PaddleEngine;
        use sc_ocr::models::*;
        let mut combos = vec![
            (
                "v5srv+en",
                PADDLE_DET_V5_SERVER,
                PADDLE_REC_EN_V5,
                PADDLE_DICT_EN_V5,
            ),
            ("v3+en", PADDLE_DET_V3, PADDLE_REC_EN_V5, PADDLE_DICT_EN_V5),
            (
                "v5srv+latin",
                PADDLE_DET_V5_SERVER,
                PADDLE_REC_LATIN_V5,
                PADDLE_DICT_LATIN_V5,
            ),
        ];
        match preset {
            Preset::Quick => combos.truncate(1),
            Preset::Reduced => combos.truncate(2),
            Preset::Full => {}
        }
        for (label, det, rec, dict) in combos {
            let name = format!("paddle/{label}");
            if wanted(&name) {
                let r = PaddleEngine::load(
                    label,
                    &models.path(&det),
                    &models.path(&rec),
                    &models.path(&dict),
                    threads,
                )
                .map(|e| Arc::new(e) as Arc<dyn OcrEngine>);
                add(name, r);
            }
        }
    }

    #[cfg(feature = "ocrs")]
    {
        use sc_ocr::engine::OcrsEngine;
        use sc_ocr::models::{OCRS_DET, OCRS_REC};
        if wanted("ocrs") {
            let r = OcrsEngine::load(&models.path(&OCRS_DET), &models.path(&OCRS_REC))
                .map(|e| Arc::new(e) as Arc<dyn OcrEngine>);
            add("ocrs".to_owned(), r);
        }
    }

    #[cfg(all(feature = "winocr", windows))]
    {
        use sc_ocr::engine::WinOcr;
        if wanted("winocr/en-US") {
            let r = WinOcr::new("en-US").map(|e| Arc::new(e) as Arc<dyn OcrEngine>);
            add("winocr/en-US".to_owned(), r);
        }
    }

    #[cfg(feature = "tesseract")]
    {
        use sc_ocr::engine::Tesseract;
        let configs: Vec<(u8, bool)> = match preset {
            Preset::Quick => vec![(6, false)],
            Preset::Reduced => vec![(6, false), (11, false), (6, true)],
            Preset::Full => vec![(6, false), (4, false), (11, false), (6, true), (4, true)],
        };
        for (psm, dict) in configs {
            let words = if dict { user_words } else { None };
            if dict && words.is_none() {
                continue;
            }
            let name = format!("tesseract/psm{psm}{}", if dict { "/dict" } else { "" });
            if wanted(&name) {
                let r = Tesseract::new(psm, words).map(|e| Arc::new(e) as Arc<dyn OcrEngine>);
                add(name, r);
            }
        }
    }

    set
}
