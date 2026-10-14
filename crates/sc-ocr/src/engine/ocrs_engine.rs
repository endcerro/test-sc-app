//! ocrs: pure-Rust OCR (detection + recognition models run by rten).

use std::path::Path;

use ocrs::{ImageSource, TextItem};
use rten::Model;

use crate::engine::{OcrEngine, OcrLine, OcrOutput};
use crate::{OcrError, OcrImage, Rect};

const NAME: &str = "ocrs";

pub struct OcrsEngine {
    engine: ocrs::OcrEngine,
}

impl std::fmt::Debug for OcrsEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OcrsEngine")
    }
}

impl OcrsEngine {
    pub fn load(det_model: &Path, rec_model: &Path) -> Result<Self, OcrError> {
        let load = |p: &Path| {
            Model::load_file(p)
                .map_err(|e| OcrError::unavailable(NAME, format!("{}: {e}", p.display())))
        };
        let engine = ocrs::OcrEngine::new(ocrs::OcrEngineParams {
            detection_model: Some(load(det_model)?),
            recognition_model: Some(load(rec_model)?),
            ..Default::default()
        })
        .map_err(|e| OcrError::engine(NAME, e))?;
        Ok(Self { engine })
    }
}

impl OcrEngine for OcrsEngine {
    fn name(&self) -> String {
        NAME.to_owned()
    }

    fn recognize(&self, img: &OcrImage) -> Result<OcrOutput, OcrError> {
        let rgb = img.to_rgb();
        let err = |e: &dyn std::fmt::Display| OcrError::engine(NAME, e);
        let source =
            ImageSource::from_bytes(rgb.as_raw(), rgb.dimensions()).map_err(|e| err(&e))?;
        let input = self.engine.prepare_input(source).map_err(|e| err(&e))?;
        let words = self.engine.detect_words(&input).map_err(|e| err(&e))?;
        let lines = self.engine.find_text_lines(&input, &words);
        let texts = self
            .engine
            .recognize_text(&input, &lines)
            .map_err(|e| err(&e))?;
        let mut out = OcrOutput::default();
        for line in texts.iter().flatten() {
            let text = line.to_string().trim().to_owned();
            if text.is_empty() {
                continue;
            }
            let r = line.bounding_rect();
            let rect = Rect::new(
                r.left().max(0) as u32,
                r.top().max(0) as u32,
                r.width().max(0) as u32,
                r.height().max(0) as u32,
            );
            out.lines.push(OcrLine {
                text,
                rect: Some(rect),
                confidence: None,
            });
        }
        out.sort_reading_order();
        Ok(out)
    }
}
