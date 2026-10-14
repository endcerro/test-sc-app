//! Reading a whole Contracts screenshot with the settings the benchmark recommends
//! (`docs/ocr.md`): anchored region crops, max-channel grey, upscaled to ~2160 px of screen
//! height, 3x3 median, light-on-dark kept. The highlighted entry of the offer list is read
//! too, as a fallback for the reward box.

use image::RgbImage;

use crate::layout::selected_offer;
use crate::preprocess::{Background, Binarize, ColorMode};
use crate::{Layout, OcrEngine, OcrError, Pipeline, Rect, Region, crop};

/// Margin around region rectangles, in reference pixels.
pub const REGION_MARGIN: f64 = 8.0;

/// Screen height the text should be scaled to, in pixels.
const TARGET_HEIGHT: f64 = 2160.0;

/// `max/-/xN/-/despeckle/native`, with `N = round(2160 / height)` clamped to 1..=3.
pub fn recommended_pipeline(height: u32) -> Pipeline {
    let scale = (TARGET_HEIGHT / f64::from(height.max(1)))
        .round()
        .clamp(1.0, 3.0) as u8;
    Pipeline {
        color: ColorMode::MaxChannel,
        background: Background::None,
        scale,
        binarize: Binarize::None,
        despeckle: true,
        invert: false,
    }
}

/// One line read on the screenshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenLine {
    pub text: String,
    /// Where the line is, in screenshot pixels, when the engine reports boxes.
    pub rect: Option<Rect>,
    /// Engine confidence in `0.0..=1.0`, when reported.
    pub confidence: Option<f32>,
}

/// The lines of each region of one screenshot, in reading order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScreenText {
    pub title: Vec<ScreenLine>,
    pub reward: Vec<ScreenLine>,
    pub details: Vec<ScreenLine>,
    pub objectives: Vec<ScreenLine>,
    /// The highlighted entry of the offer list; empty when it was not found.
    pub offer: Vec<ScreenLine>,
}

/// Lines joined with `\n`.
pub fn text_of(lines: &[ScreenLine]) -> String {
    lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Reads one region from its crop in `layout`; line boxes are mapped back to the
/// screenshot.
pub fn read_region_lines(
    img: &RgbImage,
    layout: &Layout,
    pipeline: &Pipeline,
    engine: &dyn OcrEngine,
    region: Region,
) -> Result<Vec<ScreenLine>, OcrError> {
    read_area_lines(img, layout.rect(region, REGION_MARGIN), pipeline, engine)
}

/// Reads one crop of the screenshot; line boxes are mapped back to the screenshot.
pub fn read_area_lines(
    img: &RgbImage,
    area: Rect,
    pipeline: &Pipeline,
    engine: &dyn OcrEngine,
) -> Result<Vec<ScreenLine>, OcrError> {
    let out = engine.recognize(&pipeline.apply(&crop(img, area)))?;
    Ok(out
        .lines
        .into_iter()
        .map(|l| ScreenLine {
            rect: l.rect.map(|r| to_screenshot(r, area, pipeline)),
            text: l.text,
            confidence: l.confidence,
        })
        .collect())
}

/// Box in the preprocessed crop -> box in the screenshot.
fn to_screenshot(r: Rect, area: Rect, pipeline: &Pipeline) -> Rect {
    let (x0, y0) = pipeline.to_source(f64::from(r.x), f64::from(r.y));
    let (x1, y1) = pipeline.to_source(f64::from(r.right()), f64::from(r.bottom()));
    let px = |v: f64, max: u32| (v.max(0.0).round() as u32).min(max);
    let (x0, x1) = (px(x0, area.w), px(x1, area.w));
    let (y0, y1) = (px(y0, area.h), px(y1, area.h));
    Rect::new(area.x + x0, area.y + y0, x1 - x0.min(x1), y1 - y0.min(y1))
}

/// Reads one region's text from its crop in `layout`.
pub fn read_region(
    img: &RgbImage,
    layout: &Layout,
    pipeline: &Pipeline,
    engine: &dyn OcrEngine,
    region: Region,
) -> Result<String, OcrError> {
    let processed = pipeline.apply(&crop(img, layout.rect(region, REGION_MARGIN)));
    Ok(engine.recognize(&processed)?.text())
}

/// Reads the four regions of a Contracts screenshot, crops anchored on the OFFERS tab, and
/// the highlighted offer list entry. They are read concurrently (engines with a session
/// pool run them in parallel).
pub fn read_contract_screen(
    img: &RgbImage,
    engine: &dyn OcrEngine,
    pipeline: &Pipeline,
) -> Result<ScreenText, OcrError> {
    let layout = Layout::anchored(img);
    let read = |region| read_region_lines(img, &layout, pipeline, engine, region);
    let read_offer = || match selected_offer(img, &layout) {
        Some(area) => read_area_lines(img, area, pipeline, engine),
        None => Ok(Vec::new()),
    };
    let panicked = || Err(OcrError::engine(engine.name(), "OCR thread panicked"));
    let ([title, reward, details, objectives], offer) = std::thread::scope(|s| {
        let offer = s.spawn(read_offer);
        let handles = Region::ALL.map(|region| s.spawn(move || read(region)));
        (
            handles.map(|h| h.join().unwrap_or_else(|_| panicked())),
            offer.join().unwrap_or_else(|_| panicked()),
        )
    });
    Ok(ScreenText {
        title: title?,
        reward: reward?,
        details: details?,
        objectives: objectives?,
        offer: offer?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_follows_height() {
        let scale = |h| recommended_pipeline(h).scale;
        assert_eq!(scale(720), 3);
        assert_eq!(scale(1080), 2);
        assert_eq!(scale(1440), 2);
        assert_eq!(scale(2160), 1);
        assert_eq!(scale(4320), 1);
        assert_eq!(
            recommended_pipeline(1080).id(),
            "max/-/x2/-/despeckle/native"
        );
    }

    #[test]
    fn boxes_map_back_to_the_screenshot() {
        let pipeline = recommended_pipeline(1080);
        let pad = pipeline.padding();
        let area = Rect::new(1000, 300, 500, 400);
        // A line at (10, 20) of the crop, 100x30, is at pad + 2x in the processed image.
        let processed = Rect::new(pad + 20, pad + 40, 200, 60);
        assert_eq!(
            to_screenshot(processed, area, &pipeline),
            Rect::new(1010, 320, 100, 30)
        );
        // Boxes reaching into the padding are clamped to the crop.
        let edge = Rect::new(0, 0, pad + 2, pad + 2);
        assert_eq!(
            to_screenshot(edge, area, &pipeline),
            Rect::new(1000, 300, 1, 1)
        );
    }
}
