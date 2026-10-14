//! Reading a whole Contracts screenshot with the settings the benchmark recommends
//! (`docs/ocr.md`): anchored region crops, max-channel grey, upscaled to ~2160 px of screen
//! height, 3x3 median, light-on-dark kept.

use image::RgbImage;

use crate::preprocess::{Background, Binarize, ColorMode};
use crate::{Layout, OcrEngine, OcrError, Pipeline, Region, crop};

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

/// Text of each region of one screenshot, lines separated by `\n`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenText {
    pub title: String,
    pub reward: String,
    pub details: String,
    pub objectives: String,
}

/// Reads one region from its crop in `layout`.
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

/// Reads the four regions of a Contracts screenshot, crops anchored on the OFFERS tab. The
/// regions are read concurrently (engines with a session pool run them in parallel).
pub fn read_contract_screen(
    img: &RgbImage,
    engine: &dyn OcrEngine,
    pipeline: &Pipeline,
) -> Result<ScreenText, OcrError> {
    let layout = Layout::anchored(img);
    let read = |region| read_region(img, &layout, pipeline, engine, region);
    let [title, reward, details, objectives] = std::thread::scope(|s| {
        let handles = Region::ALL.map(|region| s.spawn(move || read(region)));
        handles.map(|h| {
            h.join()
                .unwrap_or_else(|_| Err(OcrError::engine(engine.name(), "OCR thread panicked")))
        })
    });
    Ok(ScreenText {
        title: title?,
        reward: reward?,
        details: details?,
        objectives: objectives?,
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
}
