//! PaddleOCR (PP-OCR) models on ONNX Runtime.
//!
//! Detection is DB (differentiable binarisation): a text-probability map that is
//! thresholded into boxes. Recognition is CRNN/SVTR with greedy CTC decoding. Text on these
//! screens is horizontal, so boxes are kept axis-aligned.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use image::RgbImage;
use image::imageops::{self, FilterType};
use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;

use crate::engine::{OcrEngine, OcrLine, OcrOutput};
use crate::preprocess::{text_rows, text_span};
use crate::{OcrError, OcrImage, Rect};

const NAME: &str = "paddle";

/// DB post-processing (PaddleOCR defaults).
const DB_THRESH: f32 = 0.3;
const DB_BOX_THRESH: f32 = 0.6;
const DB_UNCLIP: f32 = 1.5;
/// Largest side fed to the detector.
const DET_MAX_SIDE: u32 = 4000;

/// Models bigger than this (the 88 MB server detector) get fewer, multi-threaded sessions:
/// their activations on large crops take hundreds of MB per concurrent run.
const HEAVY_MODEL_BYTES: u64 = 20 * 1024 * 1024;

/// Sessions are not shareable (`run` takes `&mut`), so keep several; callers beyond the
/// pool size wait, which also bounds memory.
struct Pool(Vec<Mutex<Session>>);

impl Pool {
    /// `threads` is the parallelism budget: light models get one single-threaded session
    /// per thread, heavy ones a quarter of that with 4 threads each.
    fn load(env: &Environment, path: &Path, threads: usize) -> Result<Self, OcrError> {
        let heavy = std::fs::metadata(path).is_ok_and(|m| m.len() > HEAVY_MODEL_BYTES);
        let (size, intra) = if heavy {
            ((threads / 4).max(1), 4)
        } else {
            (threads.max(1), 1)
        };
        let mut sessions = Vec::with_capacity(size);
        for _ in 0..size {
            let err = |e: ort::Error<_>| OcrError::engine(NAME, e);
            // The CPU arena grows to the largest input seen and never shrinks; crops vary a
            // lot in size, so plain allocations keep memory bounded.
            let cpu = ort::ep::CPU::default().with_arena_allocator(false).build();
            let session = Session::builder(env)
                .map_err(|e| OcrError::engine(NAME, e))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(err)?
                .with_intra_threads(intra)
                .map_err(err)?
                .with_memory_pattern(false)
                .map_err(err)?
                .with_execution_providers([cpu])
                .map_err(err)?
                .commit_from_file(path)
                .map_err(|e| OcrError::unavailable(NAME, format!("{}: {e}", path.display())))?;
            sessions.push(Mutex::new(session));
        }
        Ok(Self(sessions))
    }

    fn with<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        for m in &self.0 {
            if let Ok(mut s) = m.try_lock() {
                return f(&mut s);
            }
        }
        // All busy: wait on one, spread by thread id.
        let i = thread_index() % self.0.len();
        let mut s = match self.0[i].lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut s)
    }

    fn input_height(&self) -> Option<i64> {
        self.0.first().and_then(|m| {
            let s = m.lock().ok()?;
            let shape = s.inputs().first()?.dtype().tensor_shape()?.to_vec();
            shape.get(2).copied().filter(|h| *h > 0)
        })
    }
}

/// ONNX Runtime allows a single environment per process; every engine shares it.
fn environment() -> Result<Environment, OcrError> {
    static ENV: OnceLock<Result<Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| {
        ort::init()
            .with_name("sc-ocr")
            .build()
            .map_err(|e| e.to_string())
    })
    .clone()
    .map_err(|e| OcrError::unavailable(NAME, e))
}

fn thread_index() -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut h);
    h.finish() as usize
}

/// PP-OCR detector + recogniser.
pub struct PaddleEngine {
    label: String,
    det: Pool,
    rec: Pool,
    rec_height: u32,
    /// Class index -> text (index 0 is the CTC blank).
    charset: Vec<String>,
}

impl std::fmt::Debug for PaddleEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaddleEngine")
            .field("label", &self.label)
            .finish()
    }
}

impl PaddleEngine {
    /// `label` names the model combination in reports (e.g. `v5srv+en`). `threads` is the
    /// parallelism budget per model (see [`Pool::load`]).
    pub fn load(
        label: &str,
        det_model: &Path,
        rec_model: &Path,
        dict: &Path,
        threads: usize,
    ) -> Result<Self, OcrError> {
        let dict_text = std::fs::read_to_string(dict).map_err(|source| OcrError::Io {
            path: dict.to_owned(),
            source,
        })?;
        let mut charset = vec![String::new()];
        charset.extend(dict_text.lines().map(str::to_owned));
        // PP-OCR appends a space class after the dictionary.
        charset.push(" ".to_owned());
        let env = environment()?;
        let rec = Pool::load(&env, rec_model, threads)?;
        let rec_height = rec
            .input_height()
            .and_then(|h| u32::try_from(h).ok())
            .unwrap_or(48);
        Ok(Self {
            label: label.to_owned(),
            det: Pool::load(&env, det_model, threads)?,
            rec,
            rec_height,
            charset,
        })
    }

    fn detect(&self, img: &RgbImage) -> Result<Vec<Rect>, OcrError> {
        let (w, h) = img.dimensions();
        let ratio = (f64::from(DET_MAX_SIDE) / f64::from(w.max(h))).min(1.0);
        let round32 = |v: f64| ((v / 32.0).round() as u32).max(1) * 32;
        let (dw, dh) = (round32(f64::from(w) * ratio), round32(f64::from(h) * ratio));
        let resized = imageops::resize(img, dw, dh, FilterType::Triangle);
        // BGR order, ImageNet mean/std, as PaddleOCR's NormalizeImage on cv2 images.
        let mean = [0.406f32, 0.456, 0.485];
        let std = [0.225f32, 0.224, 0.229];
        let plane = (dw * dh) as usize;
        let mut data = vec![0f32; 3 * plane];
        for (i, p) in resized.pixels().enumerate() {
            let bgr = [p.0[2], p.0[1], p.0[0]];
            for c in 0..3 {
                data[c * plane + i] = (f32::from(bgr[c]) / 255.0 - mean[c]) / std[c];
            }
        }
        let prob: Vec<f32> = self.det.with(|s| -> Result<Vec<f32>, OcrError> {
            let input = TensorRef::from_array_view(([1usize, 3, dh as usize, dw as usize], &*data))
                .map_err(|e| OcrError::engine(NAME, e))?;
            let out = s
                .run(ort::inputs![input])
                .map_err(|e| OcrError::engine(NAME, e))?;
            let (_, values) = out[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| OcrError::engine(NAME, e))?;
            Ok(values.to_vec())
        })?;
        if prob.len() != plane {
            return Err(OcrError::engine(NAME, "unexpected detection output size"));
        }
        let (sx, sy) = (f64::from(w) / f64::from(dw), f64::from(h) / f64::from(dh));
        Ok(db_boxes(&prob, dw as usize, dh as usize)
            .into_iter()
            .filter_map(|(x0, y0, x1, y1)| {
                Rect::new(
                    (x0 * sx).max(0.0) as u32,
                    (y0 * sy).max(0.0) as u32,
                    ((x1 - x0) * sx).ceil() as u32,
                    ((y1 - y0) * sy).ceil() as u32,
                )
                .clamp(w, h)
            })
            .collect())
    }

    fn recognize_crop(&self, crop: &RgbImage) -> Result<(String, f32), OcrError> {
        let (w, h) = crop.dimensions();
        if w < 2 || h < 2 {
            return Ok((String::new(), 0.0));
        }
        let rh = self.rec_height;
        let rw = ((f64::from(w) * f64::from(rh) / f64::from(h)).ceil() as u32).clamp(16, 3200);
        let resized = imageops::resize(crop, rw, rh, FilterType::Triangle);
        let plane = (rw * rh) as usize;
        let mut data = vec![0f32; 3 * plane];
        for (i, p) in resized.pixels().enumerate() {
            let bgr = [p.0[2], p.0[1], p.0[0]];
            for c in 0..3 {
                data[c * plane + i] = (f32::from(bgr[c]) / 255.0 - 0.5) / 0.5;
            }
        }
        let (shape, probs) = self
            .rec
            .with(|s| -> Result<(Vec<i64>, Vec<f32>), OcrError> {
                let input =
                    TensorRef::from_array_view(([1usize, 3, rh as usize, rw as usize], &*data))
                        .map_err(|e| OcrError::engine(NAME, e))?;
                let out = s
                    .run(ort::inputs![input])
                    .map_err(|e| OcrError::engine(NAME, e))?;
                let (shape, values) = out[0]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| OcrError::engine(NAME, e))?;
                Ok((shape.to_vec(), values.to_vec()))
            })?;
        let [_, steps, classes] = shape[..] else {
            return Err(OcrError::engine(NAME, "unexpected recognition output rank"));
        };
        Ok(ctc_decode(
            &probs,
            steps as usize,
            classes as usize,
            &self.charset,
        ))
    }

    fn read_boxes(&self, rgb: &RgbImage, boxes: &[Rect]) -> Result<OcrOutput, OcrError> {
        let mut out = OcrOutput::default();
        for r in boxes {
            let crop = imageops::crop_imm(rgb, r.x, r.y, r.w, r.h).to_image();
            let (text, conf) = self.recognize_crop(&crop)?;
            let text = text.trim().to_owned();
            if !text.is_empty() {
                out.lines.push(OcrLine {
                    text,
                    rect: Some(*r),
                    confidence: Some(conf),
                });
            }
        }
        out.sort_reading_order();
        Ok(out)
    }
}

impl OcrEngine for PaddleEngine {
    fn name(&self) -> String {
        format!("paddle/{}", self.label)
    }

    fn recognize(&self, img: &OcrImage) -> Result<OcrOutput, OcrError> {
        let rgb = img.to_rgb();
        let boxes = self.detect(&rgb)?;
        self.read_boxes(&rgb, &boxes)
    }

    /// Skips detection: rows of text are found by projection and recognised directly.
    fn recognize_lines(&self, img: &OcrImage) -> Result<OcrOutput, OcrError> {
        let rgb = img.to_rgb();
        let gray = img.to_gray();
        let rows = text_rows(&gray);
        if rows.is_empty() {
            return self.recognize(img);
        }
        let boxes: Vec<Rect> = rows
            .into_iter()
            .filter_map(|(y0, y1)| {
                let pad = ((y1 - y0) / 5).max(2);
                let top = y0.saturating_sub(pad);
                let bottom = (y1 + pad).min(rgb.height());
                let (x0, x1) = text_span(&gray, y0, y1)?;
                let x0 = x0.saturating_sub(pad);
                let x1 = (x1 + pad).min(rgb.width());
                Some(Rect::new(x0, top, x1 - x0, bottom - top))
            })
            .collect();
        self.read_boxes(&rgb, &boxes)
    }
}

/// Probability map -> boxes `(x0, y0, x1, y1)` in map pixels, unclipped.
fn db_boxes(prob: &[f32], w: usize, h: usize) -> Vec<(f64, f64, f64, f64)> {
    let mut seen = vec![false; prob.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..prob.len() {
        if seen[start] || prob[start] <= DB_THRESH {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
        let (mut sum, mut n) = (0f64, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            sum += f64::from(prob[i]);
            n += 1;
            let neighbours = [
                (x > 0).then(|| i - 1),
                (x + 1 < w).then(|| i + 1),
                (y > 0).then(|| i - w),
                (y + 1 < h).then(|| i + w),
            ];
            for j in neighbours.into_iter().flatten() {
                if !seen[j] && prob[j] > DB_THRESH {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64);
        if bw.min(bh) < 3.0 || sum / (n as f64) < f64::from(DB_BOX_THRESH) {
            continue;
        }
        // Grow by area * ratio / perimeter on each side (DB "unclip" for a rectangle).
        let d = bw * bh * f64::from(DB_UNCLIP) / (2.0 * (bw + bh));
        out.push((
            x0 as f64 - d,
            y0 as f64 - d,
            x1 as f64 + 1.0 + d,
            y1 as f64 + 1.0 + d,
        ));
    }
    out
}

/// Greedy CTC: best class per step, repeats collapsed, blanks dropped. Returns the text
/// and the mean probability of the kept characters.
fn ctc_decode(probs: &[f32], steps: usize, classes: usize, charset: &[String]) -> (String, f32) {
    let mut text = String::new();
    let (mut conf, mut kept) = (0f32, 0usize);
    let mut prev = 0usize;
    for t in 0..steps {
        let row = &probs[t * classes..(t + 1) * classes];
        let (best, p) =
            row.iter().enumerate().fold(
                (0usize, f32::MIN),
                |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc },
            );
        if best != 0
            && best != prev
            && let Some(ch) = charset.get(best)
        {
            text.push_str(ch);
            conf += p;
            kept += 1;
        }
        prev = best;
    }
    (text, if kept > 0 { conf / kept as f32 } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ctc() {
        let charset: Vec<String> = ["", "a", "b", " "]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        // a a _ a b
        let steps = [1usize, 1, 0, 1, 2];
        let mut probs = vec![0f32; steps.len() * 4];
        for (t, &c) in steps.iter().enumerate() {
            probs[t * 4 + c] = 0.9;
        }
        let (text, conf) = ctc_decode(&probs, steps.len(), 4, &charset);
        assert_eq!(text, "aab");
        assert!((conf - 0.9).abs() < 1e-6);
    }

    #[test]
    fn db_finds_blob() {
        let (w, h) = (40, 20);
        let mut prob = vec![0f32; w * h];
        for y in 8..12 {
            for x in 5..30 {
                prob[y * w + x] = 0.9;
            }
        }
        let boxes = db_boxes(&prob, w, h);
        assert_eq!(boxes.len(), 1);
        let (x0, y0, x1, y1) = boxes[0];
        assert!(x0 < 5.0 && y0 < 8.0 && x1 > 30.0 && y1 > 12.0);
    }
}
