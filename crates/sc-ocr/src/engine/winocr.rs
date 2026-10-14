//! Windows built-in OCR (`Windows.Media.Ocr`), available on Windows 10+ with the language's
//! OCR pack installed (English ships with English Windows; others via Settings > Language).

use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine as WinOcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::core::HSTRING;

use crate::engine::{OcrEngine, OcrLine, OcrOutput};
use crate::{OcrError, OcrImage, Rect};

const NAME: &str = "winocr";

/// Stateless: a WinRT engine is created per call (cheap), so the adapter is `Send + Sync`.
#[derive(Debug, Clone)]
pub struct WinOcr {
    language: String,
}

impl WinOcr {
    /// `language` is a BCP-47 tag such as `en-US`. Fails if no OCR pack is installed for it.
    pub fn new(language: &str) -> Result<Self, OcrError> {
        let me = Self {
            language: language.to_owned(),
        };
        me.engine()?;
        Ok(me)
    }

    fn engine(&self) -> Result<WinOcrEngine, OcrError> {
        let lang = Language::CreateLanguage(&HSTRING::from(self.language.as_str()))
            .map_err(|e| OcrError::unavailable(NAME, e))?;
        WinOcrEngine::TryCreateFromLanguage(&lang).map_err(|e| {
            OcrError::unavailable(NAME, format!("no OCR pack for {}: {e}", self.language))
        })
    }

    /// Largest width/height the engine accepts.
    pub fn max_dimension() -> u32 {
        WinOcrEngine::MaxImageDimension().unwrap_or(2600)
    }
}

impl OcrEngine for WinOcr {
    fn name(&self) -> String {
        format!("{NAME}/{}", self.language)
    }

    fn recognize(&self, img: &OcrImage) -> Result<OcrOutput, OcrError> {
        let err = |e: windows::core::Error| OcrError::engine(NAME, e);
        let rgb = img.to_rgb();
        let (w, h) = rgb.dimensions();
        let max = Self::max_dimension();
        if w > max || h > max {
            return Err(OcrError::engine(
                NAME,
                format!("{w}x{h} exceeds the {max} px limit"),
            ));
        }
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for p in rgb.pixels() {
            bgra.extend_from_slice(&[p.0[2], p.0[1], p.0[0], 255]);
        }
        let writer = DataWriter::new().map_err(err)?;
        writer.WriteBytes(&bgra).map_err(err)?;
        let buffer = writer.DetachBuffer().map_err(err)?;
        let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            w as i32,
            h as i32,
            BitmapAlphaMode::Premultiplied,
        )
        .map_err(err)?;
        let result = self
            .engine()?
            .RecognizeAsync(&bitmap)
            .map_err(err)?
            .join()
            .map_err(err)?;
        let mut out = OcrOutput::default();
        for line in result.Lines().map_err(err)? {
            let text = line.Text().map_err(err)?.to_string();
            let mut rect: Option<(f32, f32, f32, f32)> = None;
            for word in line.Words().map_err(err)? {
                let b = word.BoundingRect().map_err(err)?;
                let (x0, y0, x1, y1) = (b.X, b.Y, b.X + b.Width, b.Y + b.Height);
                rect = Some(match rect {
                    Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
                    None => (x0, y0, x1, y1),
                });
            }
            out.lines.push(OcrLine {
                text,
                rect: rect.map(|(x0, y0, x1, y1)| {
                    Rect::new(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32)
                }),
                confidence: None,
            });
        }
        out.sort_reading_order();
        Ok(out)
    }
}
