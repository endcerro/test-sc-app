//! Screen capture for the in-game hotkey: grabs the monitor the game is on and saves it as
//! `shots/<unix ms>.png` in the contracts directory, so the shot can be OCR'd like a dropped
//! file and its pixels shown later. Capturing needs Windows. No egui here.

// Outside Windows `grab` always fails, so the saving helpers are only used by tests there.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use image::RgbaImage;

/// Where captured screenshots are kept: `<contracts dir>/shots`.
pub fn shots_dir(store_dir: &Path) -> PathBuf {
    store_dir.join("shots")
}

/// Whether `path` is a screenshot this app captured into `store_dir` (and may delete).
/// Compared as absolute paths: the reader records shots with absolute paths.
pub fn is_captured(store_dir: &Path, path: &Path) -> bool {
    let abs = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_owned());
    path.parent().map(abs) == Some(abs(&shots_dir(store_dir)))
}

/// A free `<unix ms>.png` path in `dir`: the current time, bumped past existing files.
fn shot_path(dir: &Path, now_ms: u64) -> PathBuf {
    (now_ms..)
        .map(|ms| dir.join(format!("{ms}.png")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{now_ms}.png")))
}

/// Whether a capture came out black, as an exclusive-fullscreen game does: no pixel is
/// brighter than a near-black level.
pub fn is_blank(img: &RgbaImage) -> bool {
    img.pixels().all(|p| p.0[..3].iter().all(|&c| c <= 8))
}

/// Saves `img` as a new PNG in `dir` (written then renamed, so the OCR thread never sees a
/// half-written file) and returns its path.
pub fn save(dir: &Path, img: &RgbaImage) -> Result<PathBuf> {
    use anyhow::Context as _;

    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let path = shot_path(dir, now);
    let tmp = path.with_extension("png.tmp");
    img.save_with_format(&tmp, image::ImageFormat::Png)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Captures the monitor of the focused window (the game), else the primary one, and saves
/// it in `dir`.
#[cfg(windows)]
pub fn grab(dir: &Path) -> Result<PathBuf> {
    use anyhow::{Context as _, bail};

    let focused = xcap::Window::all()
        .ok()
        .and_then(|ws| ws.into_iter().find(|w| w.is_focused().unwrap_or(false)))
        .and_then(|w| w.current_monitor().ok());
    let monitor = match focused {
        Some(m) => m,
        None => xcap::Monitor::all()
            .context("listing the monitors")?
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .context("no monitor found")?,
    };
    let img = monitor.capture_image().context("capturing the screen")?;
    if is_blank(&img) {
        bail!("captured a black screen: set Star Citizen to Borderless window mode");
    }
    save(dir, &img)
}

#[cfg(not(windows))]
pub fn grab(_dir: &Path) -> Result<PathBuf> {
    anyhow::bail!("Screen capture is only available on Windows")
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    #[test]
    fn black_frames_are_blank() {
        let mut img = RgbaImage::from_pixel(8, 4, Rgba([3, 5, 8, 255]));
        assert!(is_blank(&img));
        img.put_pixel(7, 3, Rgba([200, 180, 40, 255]));
        assert!(!is_blank(&img));
    }

    #[test]
    fn saved_shots_get_free_names_in_the_shots_dir() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let shots = shots_dir(dir.path());
        assert_eq!(shot_path(&shots, 42), shots.join("42.png"));
        let img = RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 255]));
        let a = save(&shots, &img).unwrap_or_else(|e| panic!("{e}"));
        let b = save(&shots, &img).unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(a, b);
        assert!(a.is_file() && b.is_file());
        assert!(is_captured(dir.path(), &a));
        assert!(!is_captured(dir.path(), &dir.path().join("x.png")));
        std::fs::write(shots.join("100.png"), "").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(shot_path(&shots, 100), shots.join("101.png"));
    }
}
