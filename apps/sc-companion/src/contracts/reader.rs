//! Screenshot -> [`ShotText`] with the benchmark's recommended setup (`docs/ocr.md`):
//! PaddleOCR `v3+en`, anchored crops, `recommended_pipeline`. Needs the `ocr` feature.

use std::path::Path;

use anyhow::Result;

use super::ShotText;

/// Shown when the app was built without the `ocr` feature.
pub const NO_OCR: &str =
    "This build has no OCR. Rebuild with `cargo run -p sc-companion --features ocr`.";

/// Session pool size given to the engine: one per region read concurrently.
#[cfg(feature = "ocr")]
const ENGINE_THREADS: usize = 4;

/// Reads contract screenshots. The engine (and its models, downloaded if missing) is loaded
/// on the first read, so creating a reader is free.
#[derive(Debug, Default)]
pub struct ContractReader {
    #[cfg(feature = "ocr")]
    engine: Option<sc_ocr::engine::PaddleEngine>,
}

impl ContractReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this build can read screenshots at all.
    pub const fn available() -> bool {
        cfg!(feature = "ocr")
    }

    /// OCRs one screenshot. `status` receives progress messages (model download, engine
    /// loading, reading).
    #[cfg(feature = "ocr")]
    pub fn read(&mut self, path: &Path, status: &mut dyn FnMut(String)) -> Result<ShotText> {
        use anyhow::Context as _;
        use sc_ocr::models::{self, ModelDir};

        let engine = match &mut self.engine {
            Some(engine) => engine,
            slot @ None => {
                let dir = ModelDir::new(models::default_dir());
                models::fetch(&dir, models::PADDLE_RECOMMENDED, |file| {
                    status(format!("Downloading OCR model {}...", file.name));
                })
                .context("downloading the OCR models")?;
                status("Loading the OCR engine...".to_owned());
                let engine = sc_ocr::engine::PaddleEngine::load(
                    "v3+en",
                    &dir.path(&models::PADDLE_DET_V3),
                    &dir.path(&models::PADDLE_REC_EN_V5),
                    &dir.path(&models::PADDLE_DICT_EN_V5),
                    ENGINE_THREADS,
                )?;
                slot.insert(engine)
            }
        };
        status(format!("Reading {}...", file_name(path)));
        let img = sc_ocr::load_image(path)?;
        let pipeline = sc_ocr::recommended_pipeline(img.height());
        let screen = sc_ocr::read_contract_screen(&img, engine, &pipeline)?;
        let source = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
        Ok(ShotText::from_screen(Some(source), screen))
    }

    #[cfg(not(feature = "ocr"))]
    pub fn read(&mut self, _path: &Path, _status: &mut dyn FnMut(String)) -> Result<ShotText> {
        anyhow::bail!(NO_OCR)
    }
}

/// File name for messages, falling back to the whole path.
pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Screenshot formats the reader accepts (by extension, case-insensitive).
pub const EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

pub fn is_screenshot(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_extensions() {
        assert!(is_screenshot(Path::new("a/01-shot.webp")));
        assert!(is_screenshot(Path::new("SHOT.PNG")));
        assert!(is_screenshot(Path::new("x.jpeg")));
        assert!(!is_screenshot(Path::new("notes.txt")));
        assert!(!is_screenshot(Path::new("noext")));
    }

    #[cfg(not(feature = "ocr"))]
    #[test]
    fn without_ocr_reading_fails_cleanly() {
        let err = ContractReader::new()
            .read(Path::new("x.png"), &mut |_| {})
            .err()
            .map(|e| e.to_string());
        assert_eq!(err.as_deref(), Some(NO_OCR));
    }
}
