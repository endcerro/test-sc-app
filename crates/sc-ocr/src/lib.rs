//! OCR of in-game screens.
//!
//! - [`layout`]: where the regions of the Contracts screen are, and the objective bullets.
//! - [`preprocess`]: image pipelines that make the OCR engines' job easier.
//! - [`engine`]: the [`OcrEngine`] trait and the adapters (each behind a cargo feature).
//! - [`screen`]: a whole Contracts screenshot read with the recommended settings.
//! - [`models`]: model files (pinned URLs, download behind the `download` feature).
//!
//! Text interpretation (parsing contracts) lives in `sc-contracts`; this crate only turns
//! pixels into lines of text.

pub mod engine;
pub mod layout;
pub mod models;
pub mod preprocess;
pub mod screen;

use std::path::{Path, PathBuf};

use image::RgbImage;
use thiserror::Error;

pub use engine::{OcrEngine, OcrLine, OcrOutput};
pub use layout::{Bullet, BulletKind, Layout, Rect, Region};
pub use preprocess::{OcrImage, Pipeline};
pub use screen::{ScreenText, read_contract_screen, recommended_pipeline};

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("cannot read image {path}: {source}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The engine is not usable here (missing binary, model or OS support).
    #[error("{engine} unavailable: {message}")]
    Unavailable { engine: String, message: String },
    #[error("{engine} failed: {message}")]
    Engine { engine: String, message: String },
    #[error("cannot download {file}: {message}")]
    Download { file: String, message: String },
}

impl OcrError {
    pub fn engine(engine: impl Into<String>, message: impl ToString) -> Self {
        Self::Engine {
            engine: engine.into(),
            message: message.to_string(),
        }
    }

    pub fn unavailable(engine: impl Into<String>, message: impl ToString) -> Self {
        Self::Unavailable {
            engine: engine.into(),
            message: message.to_string(),
        }
    }
}

/// Loads a screenshot (PNG, WebP, JPEG) as RGB, dropping alpha.
pub fn load_image(path: &Path) -> Result<RgbImage, OcrError> {
    image::open(path)
        .map(|i| i.to_rgb8())
        .map_err(|source| OcrError::Image {
            path: path.to_owned(),
            source,
        })
}

/// Copies a rectangle out of an image.
pub fn crop(img: &RgbImage, r: Rect) -> RgbImage {
    image::imageops::crop_imm(img, r.x, r.y, r.w, r.h).to_image()
}
