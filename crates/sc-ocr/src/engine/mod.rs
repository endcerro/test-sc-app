//! The OCR engine abstraction and its adapters.
//!
//! Each adapter sits behind a cargo feature so the default build (and CI) needs no model
//! files, native libraries or OS-specific APIs.

use crate::{OcrError, OcrImage, Rect};

#[cfg(feature = "ocrs")]
mod ocrs_engine;
#[cfg(feature = "paddle")]
mod paddle;
#[cfg(feature = "tesseract")]
mod tesseract;
#[cfg(all(feature = "winocr", windows))]
mod winocr;

#[cfg(feature = "ocrs")]
pub use ocrs_engine::OcrsEngine;
#[cfg(feature = "paddle")]
pub use paddle::PaddleEngine;
#[cfg(feature = "tesseract")]
pub use tesseract::Tesseract;
#[cfg(all(feature = "winocr", windows))]
pub use winocr::WinOcr;

/// One recognised line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    /// Bounding box in the input image's pixels, when the engine reports it.
    pub rect: Option<Rect>,
    /// Engine confidence in `0.0..=1.0`, when reported.
    pub confidence: Option<f32>,
}

/// Lines in reading order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OcrOutput {
    pub lines: Vec<OcrLine>,
}

impl OcrOutput {
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn from_text(text: &str) -> Self {
        Self {
            lines: text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| OcrLine {
                    text: l.to_owned(),
                    rect: None,
                    confidence: None,
                })
                .collect(),
        }
    }

    /// Sorts lines top to bottom, then left to right, using their boxes. Lines are grouped
    /// into rows first (a line joins the current row when its vertical centre is within half
    /// a line height of the row's first line), so the order is well defined. Lines without a
    /// box keep their relative order at the end.
    pub fn sort_reading_order(&mut self) {
        let (mut boxed, unboxed): (Vec<OcrLine>, Vec<OcrLine>) = std::mem::take(&mut self.lines)
            .into_iter()
            .partition(|l| l.rect.is_some());
        let centre = |l: &OcrLine| l.rect.map_or((0.0, 0.0), |r| r.centre());
        boxed.sort_by(|a, b| centre(a).1.total_cmp(&centre(b).1));
        let mut rows: Vec<Vec<OcrLine>> = Vec::new();
        for line in boxed {
            let (_, y) = centre(&line);
            let h = line.rect.map_or(0, |r| r.h);
            match rows.last_mut() {
                Some(row)
                    if row.first().is_some_and(|f| {
                        let tol = f64::from(h.min(f.rect.map_or(0, |r| r.h))) / 2.0;
                        (centre(f).1 - y).abs() <= tol
                    }) =>
                {
                    row.push(line);
                }
                _ => rows.push(vec![line]),
            }
        }
        for row in &mut rows {
            row.sort_by(|a, b| centre(a).0.total_cmp(&centre(b).0));
        }
        self.lines = rows.into_iter().flatten().chain(unboxed).collect();
    }
}

/// An OCR engine with a fixed configuration.
pub trait OcrEngine: Send + Sync {
    /// Engine and configuration, e.g. `tesseract/psm6/dict`. Used in reports and cache keys,
    /// so it must change when the configuration changes.
    fn name(&self) -> String;

    /// Finds and reads all text in the image.
    fn recognize(&self, img: &OcrImage) -> Result<OcrOutput, OcrError>;

    /// Reads an image known to hold a single text block of 1-3 lines (an objective crop).
    /// Engines with a separate recognition stage can skip detection here.
    fn recognize_lines(&self, img: &OcrImage) -> Result<OcrOutput, OcrError> {
        self.recognize(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, x: u32, y: u32) -> OcrLine {
        OcrLine {
            text: text.into(),
            rect: Some(Rect::new(x, y, 50, 20)),
            confidence: None,
        }
    }

    #[test]
    fn reading_order() {
        let mut out = OcrOutput {
            lines: vec![line("c", 0, 50), line("b", 100, 3), line("a", 0, 0)],
        };
        out.sort_reading_order();
        assert_eq!(out.text(), "a\nb\nc");
    }
}
