//! Image preprocessing that makes the OCR engines' job easier.
//!
//! A [`Pipeline`] is a point in the benchmark grid. Steps run in a fixed order:
//!
//! ```text
//! colour -> background -> scale -> binarise -> despeckle -> polarity -> padding
//! ```
//!
//! Internally text is kept **bright on dark** (as on screen) until the polarity step.

use std::fmt;

use image::imageops::{self, FilterType};
use image::{GrayImage, Luma, RgbImage};

use crate::layout::{components, is_bullet_yellow};

/// What a pipeline produces: engines accept either.
#[derive(Debug, Clone)]
pub enum OcrImage {
    Rgb(RgbImage),
    Gray(GrayImage),
}

impl OcrImage {
    pub fn width(&self) -> u32 {
        match self {
            Self::Rgb(i) => i.width(),
            Self::Gray(i) => i.width(),
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Self::Rgb(i) => i.height(),
            Self::Gray(i) => i.height(),
        }
    }

    pub fn to_rgb(&self) -> RgbImage {
        match self {
            Self::Rgb(i) => i.clone(),
            Self::Gray(i) => image::DynamicImage::ImageLuma8(i.clone()).to_rgb8(),
        }
    }

    pub fn to_gray(&self) -> GrayImage {
        match self {
            Self::Rgb(i) => image::DynamicImage::ImageRgb8(i.clone()).to_luma8(),
            Self::Gray(i) => i.clone(),
        }
    }

    /// PNG bytes, for engines that read files and for debugging dumps.
    pub fn to_png(&self) -> Result<Vec<u8>, image::ImageError> {
        let mut out = std::io::Cursor::new(Vec::new());
        match self {
            Self::Rgb(i) => i.write_to(&mut out, image::ImageFormat::Png)?,
            Self::Gray(i) => i.write_to(&mut out, image::ImageFormat::Png)?,
        }
        Ok(out.into_inner())
    }
}

/// How colour is turned into a single text-intensity channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ColorMode {
    /// Keep colour (only without background removal and binarisation).
    Rgb,
    /// Luminance.
    Gray,
    /// `max(r, g, b)`: keeps the blue link text as bright as the white text.
    MaxChannel,
    /// Luminance of near-white pixels only, everything else black.
    WhiteMask,
    /// Like [`ColorMode::WhiteMask`] plus the link/label blue.
    WhiteBlueMask,
}

/// Removal of the glow and light rays behind the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Background {
    None,
    /// White top-hat: subtract a morphological opening wider than a text stroke.
    TopHat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Binarize {
    None,
    /// Global threshold from the histogram.
    Otsu,
    /// Local threshold (mean and deviation over a window).
    Sauvola,
}

/// One preprocessing configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pipeline {
    pub color: ColorMode,
    pub background: Background,
    /// Integer upscale factor (Lanczos).
    pub scale: u8,
    pub binarize: Binarize,
    /// Drop specks: small connected components on a black background, else a 3x3 median.
    pub despeckle: bool,
    /// Output dark text on white (what Tesseract is trained on).
    pub invert: bool,
}

/// Border added around every crop, in output pixels per scale unit.
const PAD: u32 = 12;

impl Default for Pipeline {
    fn default() -> Self {
        Self::RAW
    }
}

impl Pipeline {
    /// No processing at all: the crop as captured.
    pub const RAW: Self = Self {
        color: ColorMode::Rgb,
        background: Background::None,
        scale: 1,
        binarize: Binarize::None,
        despeckle: false,
        invert: false,
    };

    /// Every distinct combination of the axes. Combinations that would produce the same
    /// image as another one are left out (colour is only kept without background removal
    /// or binarisation).
    pub fn grid() -> Vec<Self> {
        let mut out = Vec::new();
        for color in [
            ColorMode::Rgb,
            ColorMode::Gray,
            ColorMode::MaxChannel,
            ColorMode::WhiteMask,
            ColorMode::WhiteBlueMask,
        ] {
            for background in [Background::None, Background::TopHat] {
                for binarize in [Binarize::None, Binarize::Otsu, Binarize::Sauvola] {
                    if color == ColorMode::Rgb
                        && (background != Background::None || binarize != Binarize::None)
                    {
                        continue;
                    }
                    for scale in [1, 2, 3] {
                        for despeckle in [false, true] {
                            for invert in [false, true] {
                                out.push(Self {
                                    color,
                                    background,
                                    scale,
                                    binarize,
                                    despeckle,
                                    invert,
                                });
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Border added around the output, in output pixels.
    pub fn padding(&self) -> u32 {
        PAD * u32::from(self.scale)
    }

    /// Output pixel -> pixel of the crop given to [`Pipeline::apply`].
    pub fn to_source(&self, x: f64, y: f64) -> (f64, f64) {
        let pad = f64::from(self.padding());
        let s = f64::from(self.scale.max(1));
        ((x - pad) / s, (y - pad) / s)
    }

    /// Short stable id, used in reports and cache keys.
    pub fn id(&self) -> String {
        self.to_string()
    }

    pub fn apply(&self, crop: &RgbImage) -> OcrImage {
        if self.color == ColorMode::Rgb
            && self.background == Background::None
            && self.binarize == Binarize::None
        {
            let mut img = upscale_rgb(crop, self.scale);
            if self.despeckle {
                img = median3_rgb(&img);
            }
            if self.invert {
                imageops::invert(&mut img);
            }
            return OcrImage::Rgb(pad_rgb(&img, PAD * u32::from(self.scale), self.invert));
        }

        let masked = matches!(self.color, ColorMode::WhiteMask | ColorMode::WhiteBlueMask);
        let mut g = to_intensity(crop, self.color);
        if self.background == Background::TopHat {
            g = top_hat(&g, 9);
        }
        g = upscale_gray(&g, self.scale);
        let mut zero_background = masked;
        match self.binarize {
            Binarize::None => {}
            Binarize::Otsu => {
                g = threshold(&g, otsu(&g, masked));
                zero_background = true;
            }
            Binarize::Sauvola => {
                let window = (15 * u32::from(self.scale)) | 1;
                g = sauvola(&g, window, 0.2);
                zero_background = true;
            }
        }
        if self.despeckle {
            g = if zero_background {
                let s = u32::from(self.scale);
                drop_small_components(&g, 3 * s * s)
            } else {
                median3(&g)
            };
        }
        if self.invert {
            imageops::invert(&mut g);
        }
        OcrImage::Gray(pad_gray(&g, PAD * u32::from(self.scale), self.invert))
    }
}

impl fmt::Display for Pipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let color = match self.color {
            ColorMode::Rgb => "rgb",
            ColorMode::Gray => "gray",
            ColorMode::MaxChannel => "max",
            ColorMode::WhiteMask => "white",
            ColorMode::WhiteBlueMask => "whiteblue",
        };
        let bg = match self.background {
            Background::None => "-",
            Background::TopHat => "tophat",
        };
        let bin = match self.binarize {
            Binarize::None => "-",
            Binarize::Otsu => "otsu",
            Binarize::Sauvola => "sauvola",
        };
        write!(
            f,
            "{color}/{bg}/x{}/{bin}/{}/{}",
            self.scale,
            if self.despeckle { "despeckle" } else { "-" },
            if self.invert { "inv" } else { "native" }
        )
    }
}

/// Near-white: bright and unsaturated (the UI white has a slight blue tint).
pub fn is_text_white(p: [u8; 3]) -> bool {
    let max = i32::from(p[0].max(p[1]).max(p[2]));
    let min = i32::from(p[0].min(p[1]).min(p[2]));
    max >= 120 && (max - min) * 100 <= max * 38
}

/// The blue of links and labels (`#4a9cff`-ish): blue dominant and bright.
pub fn is_text_blue(p: [u8; 3]) -> bool {
    let [r, g, b] = p.map(i32::from);
    b >= 140 && b >= r + 50 && b + 10 >= g
}

fn luminance(p: [u8; 3]) -> u8 {
    let [r, g, b] = p.map(u32::from);
    ((299 * r + 587 * g + 114 * b) / 1000) as u8
}

fn to_intensity(crop: &RgbImage, mode: ColorMode) -> GrayImage {
    GrayImage::from_fn(crop.width(), crop.height(), |x, y| {
        let p = crop.get_pixel(x, y).0;
        let v = match mode {
            ColorMode::Rgb | ColorMode::Gray => luminance(p),
            ColorMode::MaxChannel => p[0].max(p[1]).max(p[2]),
            ColorMode::WhiteMask => {
                if is_text_white(p) && !is_bullet_yellow(p) {
                    luminance(p)
                } else {
                    0
                }
            }
            ColorMode::WhiteBlueMask => {
                if is_bullet_yellow(p) {
                    0
                } else if is_text_white(p) {
                    luminance(p)
                } else if is_text_blue(p) {
                    // Blue is darker in luminance than white; use its brightest channel.
                    p[2]
                } else {
                    0
                }
            }
        };
        Luma([v])
    })
}

fn upscale_rgb(img: &RgbImage, scale: u8) -> RgbImage {
    if scale <= 1 {
        return img.clone();
    }
    let s = u32::from(scale);
    imageops::resize(img, img.width() * s, img.height() * s, FilterType::Lanczos3)
}

fn upscale_gray(img: &GrayImage, scale: u8) -> GrayImage {
    if scale <= 1 {
        return img.clone();
    }
    let s = u32::from(scale);
    imageops::resize(img, img.width() * s, img.height() * s, FilterType::Lanczos3)
}

/// Separable min (`max == false`) or max filter with a square window of side `size`.
fn rank_filter(img: &GrayImage, size: u32, max: bool) -> GrayImage {
    let (w, h) = img.dimensions();
    let r = size / 2;
    let pick = |a: u8, b: u8| if max { a.max(b) } else { a.min(b) };
    let mut tmp = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r).min(w - 1));
            let mut v = img.get_pixel(x0, y).0[0];
            for xx in x0 + 1..=x1 {
                v = pick(v, img.get_pixel(xx, y).0[0]);
            }
            tmp.put_pixel(x, y, Luma([v]));
        }
    }
    let mut out = GrayImage::new(w, h);
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            let mut v = tmp.get_pixel(x, y0).0[0];
            for yy in y0 + 1..=y1 {
                v = pick(v, tmp.get_pixel(x, yy).0[0]);
            }
            out.put_pixel(x, y, Luma([v]));
        }
    }
    out
}

/// `img - opening(img)`: keeps thin bright strokes, removes smooth bright areas.
fn top_hat(img: &GrayImage, size: u32) -> GrayImage {
    let opened = rank_filter(&rank_filter(img, size, false), size, true);
    let mut out = img.clone();
    for (o, b) in out.pixels_mut().zip(opened.pixels()) {
        o.0[0] = o.0[0].saturating_sub(b.0[0]);
    }
    // Stretch so the brightest strokes are white again.
    let max = out.pixels().map(|p| p.0[0]).max().unwrap_or(0);
    if max > 0 && max < 255 {
        for p in out.pixels_mut() {
            p.0[0] = (u32::from(p.0[0]) * 255 / u32::from(max)) as u8;
        }
    }
    out
}

/// Otsu threshold. With `ignore_zero`, pure black (masked out) pixels are not counted.
fn otsu(img: &GrayImage, ignore_zero: bool) -> u8 {
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[usize::from(p.0[0])] += 1;
    }
    if ignore_zero {
        hist[0] = 0;
    }
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return 128;
    }
    let sum: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut sum_b, mut w_b, mut best, mut best_t) = (0.0, 0u64, 0.0, 0u8);
    for (t, &c) in hist.iter().enumerate() {
        w_b += c;
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += t as f64 * c as f64;
        let m_b = sum_b / w_b as f64;
        let m_f = (sum - sum_b) / w_f as f64;
        let between = w_b as f64 * w_f as f64 * (m_b - m_f).powi(2);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    best_t
}

fn threshold(img: &GrayImage, t: u8) -> GrayImage {
    GrayImage::from_fn(img.width(), img.height(), |x, y| {
        Luma([if img.get_pixel(x, y).0[0] > t { 255 } else { 0 }])
    })
}

/// Sauvola on bright-on-dark text: a pixel is text when it is brighter than
/// `m * (1 + k * (s / 128 - 1))` computed on the inverted image, i.e. locally bright with
/// enough contrast. Integral images make it O(1) per pixel.
fn sauvola(img: &GrayImage, window: u32, k: f64) -> GrayImage {
    let (w, h) = img.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let mut sum = vec![0f64; (wu + 1) * (hu + 1)];
    let mut sq = vec![0f64; (wu + 1) * (hu + 1)];
    for y in 0..hu {
        let (mut rs, mut rq) = (0.0, 0.0);
        for x in 0..wu {
            // Inverted: dark text on light background, as Sauvola expects.
            let v = 255.0 - f64::from(img.get_pixel(x as u32, y as u32).0[0]);
            rs += v;
            rq += v * v;
            let i = (y + 1) * (wu + 1) + x + 1;
            sum[i] = sum[i - wu - 1] + rs;
            sq[i] = sq[i - wu - 1] + rq;
        }
    }
    let r = (window / 2) as usize;
    GrayImage::from_fn(w, h, |x, y| {
        let (x, y) = (x as usize, y as usize);
        let (x0, y0) = (x.saturating_sub(r), y.saturating_sub(r));
        let (x1, y1) = ((x + r + 1).min(wu), (y + r + 1).min(hu));
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        let at = |xx: usize, yy: usize| yy * (wu + 1) + xx;
        let s = sum[at(x1, y1)] - sum[at(x0, y1)] - sum[at(x1, y0)] + sum[at(x0, y0)];
        let q = sq[at(x1, y1)] - sq[at(x0, y1)] - sq[at(x1, y0)] + sq[at(x0, y0)];
        let mean = s / n;
        let dev = (q / n - mean * mean).max(0.0).sqrt();
        let t = mean * (1.0 + k * (dev / 128.0 - 1.0));
        let v = 255.0 - f64::from(img.get_pixel(x as u32, y as u32).0[0]);
        // Flat dark areas (dev ~ 0) would pass with any small noise; require contrast.
        Luma([if v < t && dev > 12.0 { 255 } else { 0 }])
    })
}

fn median3(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    GrayImage::from_fn(w, h, |x, y| {
        let mut v = [0u8; 9];
        let mut n = 0;
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let xx = (i64::from(x) + dx).clamp(0, i64::from(w) - 1) as u32;
                let yy = (i64::from(y) + dy).clamp(0, i64::from(h) - 1) as u32;
                v[n] = img.get_pixel(xx, yy).0[0];
                n += 1;
            }
        }
        v.sort_unstable();
        Luma([v[4]])
    })
}

fn median3_rgb(img: &RgbImage) -> RgbImage {
    let channels: Vec<GrayImage> = (0..3)
        .map(|c| {
            median3(&GrayImage::from_fn(img.width(), img.height(), |x, y| {
                Luma([img.get_pixel(x, y).0[c]])
            }))
        })
        .collect();
    RgbImage::from_fn(img.width(), img.height(), |x, y| {
        image::Rgb([0, 1, 2].map(|c| channels[c].get_pixel(x, y).0[0]))
    })
}

/// Zeroes connected groups of non-zero pixels smaller than `min_area` (bounding box area,
/// so thin but long strokes survive).
fn drop_small_components(img: &GrayImage, min_area: u32) -> GrayImage {
    let (w, h) = img.dimensions();
    let mask: Vec<bool> = img.pixels().map(|p| p.0[0] > 0).collect();
    let mut out = img.clone();
    for c in components(&mask, w as usize, h as usize) {
        if c.w * c.h < min_area {
            for y in c.y..c.bottom() {
                for x in c.x..c.right() {
                    out.put_pixel(x, y, Luma([0]));
                }
            }
        }
    }
    out
}

fn pad_gray(img: &GrayImage, pad: u32, light: bool) -> GrayImage {
    let fill = if light { 255 } else { 0 };
    let mut out =
        GrayImage::from_pixel(img.width() + 2 * pad, img.height() + 2 * pad, Luma([fill]));
    imageops::replace(&mut out, img, i64::from(pad), i64::from(pad));
    out
}

fn pad_rgb(img: &RgbImage, pad: u32, light: bool) -> RgbImage {
    // Native screenshots have a dark navy background; pad with black, or white if inverted.
    let fill = if light { 255 } else { 0 };
    let mut out = RgbImage::from_pixel(
        img.width() + 2 * pad,
        img.height() + 2 * pad,
        image::Rgb([fill; 3]),
    );
    imageops::replace(&mut out, img, i64::from(pad), i64::from(pad));
    out
}

/// Horizontal bands holding text in a single-column crop, as `(top, bottom)` (bottom
/// exclusive). Works on either polarity: a pixel is "ink" when it is far from the most
/// common (background) value.
pub fn text_rows(img: &GrayImage) -> Vec<(u32, u32)> {
    let (w, h) = img.dimensions();
    let bg = background_value(img);
    let min_ink = (w / 200).max(2);
    let inked: Vec<bool> = (0..h)
        .map(|y| {
            (0..w)
                .filter(|&x| (i32::from(img.get_pixel(x, y).0[0]) - bg).abs() > 70)
                .count() as u32
                >= min_ink
        })
        .collect();
    let mut rows = Vec::new();
    let mut start = None;
    for (y, &ink) in inked.iter().enumerate() {
        match (ink, start) {
            (true, None) => start = Some(y as u32),
            (false, Some(s)) => {
                rows.push((s, y as u32));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        rows.push((s, h));
    }
    // Text rows are at least ~40% as tall as the tallest; smaller bands are specks or
    // dots split off from letters.
    let tallest = rows.iter().map(|(a, b)| b - a).max().unwrap_or(0);
    rows.retain(|(a, b)| (b - a) * 5 >= tallest * 2);
    rows
}

/// Horizontal extent `(left, right)` of the text in rows `y0..y1`: the widest run of inked
/// columns where gaps are shorter than ~1.5 line heights. Isolated marks further away (the
/// panel border, a bullet remnant) are left out.
pub fn text_span(img: &GrayImage, y0: u32, y1: u32) -> Option<(u32, u32)> {
    let (w, _) = img.dimensions();
    let bg = background_value(img);
    let inked: Vec<bool> = (0..w)
        .map(|x| (y0..y1).any(|y| (i32::from(img.get_pixel(x, y).0[0]) - bg).abs() > 70))
        .collect();
    let max_gap = ((y1 - y0) * 3 / 2).max(4);
    let mut spans: Vec<(u32, u32, u32)> = Vec::new(); // (start, end, inked columns)
    let mut gap = 0;
    for (x, &ink) in inked.iter().enumerate() {
        let x = x as u32;
        if ink {
            match spans.last_mut() {
                Some(s) if gap <= max_gap => {
                    s.1 = x + 1;
                    s.2 += 1;
                }
                _ => spans.push((x, x + 1, 1)),
            }
            gap = 0;
        } else {
            gap += 1;
        }
    }
    spans.into_iter().max_by_key(|s| s.2).map(|s| (s.0, s.1))
}

/// Most common grey level (the background of a crop).
fn background_value(img: &GrayImage) -> i32 {
    let mut hist = [0u32; 256];
    for p in img.pixels() {
        hist[usize::from(p.0[0])] += 1;
    }
    hist.iter()
        .enumerate()
        .max_by_key(|(_, c)| **c)
        .map_or(0, |(v, _)| v as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_crop() -> RgbImage {
        // Dark background with a white bar, a blue bar and a yellow square.
        RgbImage::from_fn(40, 20, |x, y| {
            if (5..8).contains(&y) && (5..35).contains(&x) {
                image::Rgb([235, 240, 250])
            } else if (12..15).contains(&y) && (5..20).contains(&x) {
                image::Rgb([70, 150, 255])
            } else if (12..17).contains(&y) && (25..30).contains(&x) {
                image::Rgb([253, 244, 185])
            } else {
                image::Rgb([20, 30, 45])
            }
        })
    }

    #[test]
    fn grid_is_deduplicated() {
        let g = Pipeline::grid();
        // rgb: 3 scales x 2 x 2; 4 other colours x 2 bg x 3 bin x 3 x 2 x 2.
        assert_eq!(g.len(), 12 + 4 * 2 * 3 * 12);
        let ids: std::collections::HashSet<String> = g.iter().map(Pipeline::id).collect();
        assert_eq!(ids.len(), g.len());
    }

    #[test]
    fn masks_select_colours() {
        let crop = test_crop();
        let white = to_intensity(&crop, ColorMode::WhiteMask);
        assert!(white.get_pixel(10, 6).0[0] > 200);
        assert_eq!(white.get_pixel(10, 13).0[0], 0, "blue dropped");
        assert_eq!(white.get_pixel(27, 14).0[0], 0, "yellow dropped");
        assert_eq!(white.get_pixel(0, 0).0[0], 0, "background dropped");
        let wb = to_intensity(&crop, ColorMode::WhiteBlueMask);
        assert!(wb.get_pixel(10, 13).0[0] > 200, "blue kept");
        assert_eq!(wb.get_pixel(27, 14).0[0], 0, "yellow dropped");
    }

    #[test]
    fn pipeline_output_shape_and_polarity() {
        let crop = test_crop();
        let p = Pipeline {
            color: ColorMode::WhiteBlueMask,
            background: Background::None,
            scale: 2,
            binarize: Binarize::Otsu,
            despeckle: true,
            invert: true,
        };
        let out = p.apply(&crop).to_gray();
        let pad = PAD * 2;
        assert_eq!(out.dimensions(), (80 + 2 * pad, 40 + 2 * pad));
        assert_eq!(out.get_pixel(0, 0).0[0], 255, "white background");
        assert_eq!(out.get_pixel(pad + 20, pad + 12).0[0], 0, "dark text");
    }

    #[test]
    fn otsu_splits_bimodal() {
        let img = GrayImage::from_fn(10, 10, |x, _| Luma([if x < 5 { 20 } else { 220 }]));
        let t = otsu(&img, false);
        assert!((20..220).contains(&t));
    }

    #[test]
    fn top_hat_removes_flat_glow() {
        let img = GrayImage::from_fn(40, 40, |x, y| {
            Luma([if x == 20 && y > 5 && y < 35 { 250 } else { 120 }])
        });
        let out = top_hat(&img, 9);
        assert_eq!(out.get_pixel(5, 5).0[0], 0);
        assert!(out.get_pixel(20, 20).0[0] > 200);
    }

    #[test]
    fn finds_text_rows() {
        let img = GrayImage::from_fn(50, 40, |x, y| {
            let ink = (5..15).contains(&y) || ((25..34).contains(&y) && x < 30);
            Luma([if ink && x % 3 != 0 { 240 } else { 10 }])
        });
        assert_eq!(text_rows(&img), vec![(5, 15), (25, 34)]);
    }

    #[test]
    fn text_span_skips_far_marks() {
        let img = GrayImage::from_fn(100, 10, |x, _| {
            let ink = (10..50).contains(&x) && x % 4 != 0 || x == 95;
            Luma([if ink { 240 } else { 10 }])
        });
        assert_eq!(text_span(&img, 0, 10), Some((10, 50)));
    }

    #[test]
    fn despeckle_drops_dots() {
        let mut img = GrayImage::new(20, 20);
        img.put_pixel(3, 3, Luma([255]));
        for x in 5..15 {
            img.put_pixel(x, 10, Luma([255]));
        }
        let out = drop_small_components(&img, 4);
        assert_eq!(out.get_pixel(3, 3).0[0], 0);
        assert_eq!(out.get_pixel(8, 10).0[0], 255);
    }
}
