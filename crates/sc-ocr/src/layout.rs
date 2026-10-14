//! Where things are on the mobiGlas Contracts screen.
//!
//! Regions are defined in a 1920x1080 reference frame (measured on the samples) and mapped
//! to the screenshot by scaling with its height; the 16:9 UI is assumed centred
//! horizontally. The panel moves by a few pixels between screenshots (head bob, window
//! position), so [`Layout::anchored`] measures the green `OFFERS` tab and shifts the
//! regions by the difference.

use image::RgbImage;

/// Axis-aligned rectangle in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> u32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> u32 {
        self.y + self.h
    }

    pub fn centre(&self) -> (f64, f64) {
        (
            f64::from(self.x) + f64::from(self.w) / 2.0,
            f64::from(self.y) + f64::from(self.h) / 2.0,
        )
    }

    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x)
            && x < f64::from(self.right())
            && y >= f64::from(self.y)
            && y < f64::from(self.bottom())
    }

    /// Clamps to an image of `w` x `h`; `None` if nothing is left.
    pub fn clamp(&self, w: u32, h: u32) -> Option<Self> {
        let x = self.x.min(w);
        let y = self.y.min(h);
        let r = self.right().min(w);
        let b = self.bottom().min(h);
        (r > x && b > y).then(|| Self::new(x, y, r - x, b - y))
    }
}

/// The parts of the screen we read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Region {
    /// Contract title box (two lines).
    Title,
    /// Values column of the reward box: reward, availability, contractor.
    Reward,
    /// Details text, below the `DETAILS` header.
    Details,
    /// Primary objectives list, below the `PRIMARY OBJECTIVES` header.
    Objectives,
    /// The highlighted entry of the offer list on the left: title, rounded reward,
    /// availability, contractor. Not at a fixed place (the list scrolls): found on each
    /// screenshot by [`selected_offer`], so it is not in [`Region::ALL`]. A fallback for
    /// the reward box, which the game sometimes draws with its lines on top of each other.
    Offer,
}

impl Region {
    /// The regions at fixed places in the details panel.
    pub const ALL: [Self; 4] = [Self::Title, Self::Reward, Self::Details, Self::Objectives];

    pub fn name(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Reward => "reward",
            Self::Details => "details",
            Self::Objectives => "objectives",
            Self::Offer => "offer",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Offer])
            .find(|r| r.name() == name)
    }

    /// Rectangle in the 1920x1080 reference frame, for the panel at [`REF_ANCHOR`].
    /// Tight around the text; [`Layout::rect`] adds a margin. For [`Region::Offer`], the
    /// list column the entry is looked for in.
    pub fn reference_rect(self) -> Rect {
        match self {
            Self::Title => Rect::new(684, 182, 572, 94),
            Self::Reward => Rect::new(1478, 190, 274, 70),
            Self::Details => Rect::new(684, 346, 512, 452),
            Self::Objectives => Rect::new(1234, 344, 512, 452),
            Self::Offer => OFFER_LIST,
        }
    }
}

/// Top-left corner of the `OFFERS` tab in the reference frame (sample 01).
pub const REF_ANCHOR: (f64, f64) = (660.0, 99.0);

/// Search window for the `OFFERS` tab, reference frame.
const ANCHOR_SEARCH: Rect = Rect::new(560, 40, 360, 160);

/// Maps reference coordinates to one screenshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Image size.
    pub width: u32,
    pub height: u32,
    /// Pixels per reference pixel (`height / 1080`).
    pub scale: f64,
    /// Left edge of the 16:9 area, in image pixels.
    pub origin_x: f64,
    /// Measured panel shift in reference pixels (zero for [`Layout::fixed`]).
    pub shift: (f64, f64),
}

impl Layout {
    /// Reference positions scaled to the image, no shift.
    pub fn fixed(width: u32, height: u32) -> Self {
        let scale = f64::from(height) / 1080.0;
        let origin_x = (f64::from(width) - 1920.0 * scale) / 2.0;
        Self {
            width,
            height,
            scale,
            origin_x,
            shift: (0.0, 0.0),
        }
    }

    /// [`Layout::fixed`] corrected by the measured position of the `OFFERS` tab. Falls back
    /// to no shift when the tab is not found or the shift is implausible.
    pub fn anchored(img: &RgbImage) -> Self {
        let mut layout = Self::fixed(img.width(), img.height());
        if let Some((x, y)) = layout.find_anchor(img) {
            let shift = (x - REF_ANCHOR.0, y - REF_ANCHOR.1);
            if shift.0.abs() < 60.0 && shift.1.abs() < 60.0 {
                layout.shift = shift;
            }
        }
        layout
    }

    /// Reference point -> image pixel.
    pub fn to_image(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.origin_x + (x + self.shift.0) * self.scale,
            (y + self.shift.1) * self.scale,
        )
    }

    /// Image pixel -> reference point (without shift).
    fn to_reference(self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.origin_x) / self.scale, y / self.scale)
    }

    /// Reference rect -> clamped image rect, grown by `margin` reference pixels.
    pub fn map(&self, r: Rect, margin: f64) -> Rect {
        let (x0, y0) = self.to_image(f64::from(r.x) - margin, f64::from(r.y) - margin);
        let (x1, y1) = self.to_image(
            f64::from(r.right()) + margin,
            f64::from(r.bottom()) + margin,
        );
        let px = |v: f64| v.max(0.0).round() as u32;
        let rect = Rect::new(
            px(x0),
            px(y0),
            px(x1).saturating_sub(px(x0)),
            px(y1).saturating_sub(px(y0)),
        );
        rect.clamp(self.width, self.height).unwrap_or(Rect::new(
            0,
            0,
            self.width.min(1),
            self.height.min(1),
        ))
    }

    /// Image rect of a region, grown by `margin` reference pixels.
    pub fn rect(&self, region: Region, margin: f64) -> Rect {
        self.map(region.reference_rect(), margin)
    }

    /// Top-left of the `OFFERS` tab in reference coordinates: bounding box of its bright
    /// green border, ignoring rows/columns with only a few stray green pixels.
    fn find_anchor(&self, img: &RgbImage) -> Option<(f64, f64)> {
        let area = self.map(ANCHOR_SEARCH, 0.0);
        let mut rows = vec![0u32; area.h as usize];
        let mut cols = vec![0u32; area.w as usize];
        for y in 0..area.h {
            for x in 0..area.w {
                let p = img.get_pixel(area.x + x, area.y + y).0;
                let [r, g, b] = p.map(i32::from);
                if g >= 140 && g >= r + 50 && g >= b + 15 {
                    rows[y as usize] += 1;
                    cols[x as usize] += 1;
                }
            }
        }
        let min_count = (8.0 * self.scale).max(2.0) as u32;
        let first = |v: &[u32]| v.iter().position(|&c| c >= min_count);
        let (fx, fy) = (first(&cols)?, first(&rows)?);
        let (rx, ry) =
            self.to_reference(f64::from(area.x) + fx as f64, f64::from(area.y) + fy as f64);
        Some((rx, ry))
    }
}

/// Bullet size, which tells the objective type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BulletKind {
    /// Large diamond: `Deliver ...`.
    Deliver,
    /// Small indented diamond: `Collect ...`.
    Collect,
}

/// One yellow diamond bullet in the objectives list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bullet {
    pub kind: BulletKind,
    /// Bounding box in image pixels.
    pub rect: Rect,
}

/// The pale yellow of the bullets (about `#fdf4b9`). The UI white is bluish, so "blue well
/// below red and green" separates them. Faded bullets at scroll edges are too dark to pass.
pub fn is_bullet_yellow(p: [u8; 3]) -> bool {
    let [r, g, b] = p.map(i32::from);
    r >= 160 && g >= 140 && b + 20 <= r.min(g) && r + 25 >= g
}

/// Finds the diamond bullets inside `area` (normally the objectives region), top to bottom.
pub fn bullets(img: &RgbImage, area: Rect, scale: f64) -> Vec<Bullet> {
    let w = area.w as usize;
    let h = area.h as usize;
    let mut mask = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(area.x + x as u32, area.y + y as u32).0;
            mask[y * w + x] = is_bullet_yellow(p);
        }
    }
    let min_side = 5.0 * scale;
    let max_side = 26.0 * scale;
    // Bullets sit in the left gutter; anything further right is text over the sun glow.
    let gutter = 80.0 * scale;
    let mut rects: Vec<Rect> = components(&mask, w, h)
        .into_iter()
        .filter(|c| f64::from(c.x) < gutter)
        .filter(|c| f64::from(c.w.max(c.h)) <= max_side)
        .collect();
    // Anti-aliasing can split a thin diamond outline; merge pieces with close centres.
    rects.sort_by_key(|r| (r.y, r.x));
    let near = 10.0 * scale;
    let mut merged: Vec<Rect> = Vec::new();
    for r in rects {
        let (cx, cy) = r.centre();
        match merged.iter_mut().find(|m| {
            let (mx, my) = m.centre();
            (mx - cx).abs() <= near && (my - cy).abs() <= near
        }) {
            Some(m) => {
                let x = m.x.min(r.x);
                let y = m.y.min(r.y);
                *m = Rect::new(
                    x,
                    y,
                    m.right().max(r.right()) - x,
                    m.bottom().max(r.bottom()) - y,
                );
            }
            None => merged.push(r),
        }
    }
    let mut found: Vec<Bullet> = merged
        .into_iter()
        .filter(|c| {
            let (cw, ch) = (f64::from(c.w), f64::from(c.h));
            let square = (cw / ch) > 0.6 && (cw / ch) < 1.6;
            square && cw >= min_side && ch >= min_side
        })
        .map(|c| Bullet {
            kind: BulletKind::Collect,
            rect: Rect::new(area.x + c.x, area.y + c.y, c.w, c.h),
        })
        .collect();
    found.sort_by_key(|b| b.rect.y);
    // Collect bullets are indented by ~29 px; the panel curvature drifts lines right by up
    // to ~10 px towards the bottom, so the cut is in between.
    let left = found.iter().map(|b| b.rect.x).min().unwrap_or(0);
    for b in &mut found {
        if f64::from(b.rect.x - left) < 16.0 * scale {
            b.kind = BulletKind::Deliver;
        }
    }
    found
}

/// One crop per bullet: from just right of the bullet to the area's right edge, from just
/// above the bullet to just above the next one (so wrapped lines are included).
pub fn bullet_lines(bullets: &[Bullet], area: Rect, scale: f64) -> Vec<(BulletKind, Rect)> {
    let pad = (4.0 * scale).round() as u32;
    let mut out = Vec::with_capacity(bullets.len());
    for (i, b) in bullets.iter().enumerate() {
        let top = b.rect.y.saturating_sub(pad + b.rect.h / 2);
        let bottom = bullets.get(i + 1).map_or(area.bottom(), |n| {
            n.rect.y.saturating_sub(pad + n.rect.h / 2)
        });
        // Wrapped text is never more than 3 lines; avoid swallowing the rest of the list.
        let bottom = bottom
            .min(b.rect.bottom() + (60.0 * scale) as u32)
            .min(area.bottom());
        let left = b.rect.right() + pad;
        if bottom > top + 2 && area.right() > left + 2 {
            out.push((
                b.kind,
                Rect::new(left, top, area.right() - left, bottom - top),
            ));
        }
    }
    out
}

/// The offer list column, reference frame: where the highlighted entry is looked for.
/// Below the list header, left of the scrollbar.
const OFFER_LIST: Rect = Rect::new(150, 160, 462, 740);

/// The accent of the selected offer: bright green (verified contracts) or bright red
/// (unverified ones). The fill varies (green, teal, red), the accent bar on the entry's
/// left edge does not.
pub fn is_offer_accent(p: [u8; 3]) -> bool {
    let [r, g, b] = p.map(i32::from);
    let green = g >= 140 && g >= r + 50 && g >= b + 15;
    let red = r >= 140 && r >= g + 60 && r >= b + 40;
    green || red
}

/// The entry of the offer list that is highlighted, i.e. the contract shown in the details
/// panel, in image pixels. Found by its accent: the bar on its left edge (and the border,
/// when bright enough to connect to the bar). `None` when it is scrolled out of the list.
pub fn selected_offer(img: &RgbImage, layout: &Layout) -> Option<Rect> {
    let area = layout.map(OFFER_LIST, 0.0);
    let (w, h) = (area.w as usize, area.h as usize);
    let mut mask = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(area.x + x as u32, area.y + y as u32).0;
            mask[y * w + x] = is_offer_accent(p);
        }
    }
    let scale = layout.scale;
    // Entries are ~100 px high; the bar spans most of it.
    let accent = components(&mask, w, h)
        .into_iter()
        .filter(|c| f64::from(c.h) >= 40.0 * scale)
        .max_by_key(|c| c.h)?;
    let px = |v: f64| (v * scale).round() as u32;
    let rect = if f64::from(accent.w) >= 200.0 * scale {
        // The whole border: that is the entry.
        let m = px(2.0);
        Rect::new(
            accent.x.saturating_sub(m),
            accent.y.saturating_sub(m),
            accent.w + 2 * m,
            accent.h + 2 * m,
        )
    } else {
        // The bar alone: the entry reaches the list's right edge and a little above and
        // below the bar (less than the gap to the next entry).
        let (m, pad) = (px(4.0), px(14.0));
        let x = accent.x.saturating_sub(m);
        let y = accent.y.saturating_sub(pad);
        Rect::new(x, y, area.w - x, accent.h + 2 * pad)
    };
    let rect = rect.clamp(area.w, area.h)?;
    Some(Rect::new(area.x + rect.x, area.y + rect.y, rect.w, rect.h))
}

/// 8-connected components of `mask`, as bounding boxes.
pub(crate) fn components(mask: &[bool], w: usize, h: usize) -> Vec<Rect> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            for dy in [-1i64, 0, 1] {
                for dx in [-1i64, 0, 1] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        out.push(Rect::new(
            x0 as u32,
            y0 as u32,
            (x1 - x0 + 1) as u32,
            (y1 - y0 + 1) as u32,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_layout_scales_and_centres() {
        let l = Layout::fixed(1920, 1080);
        assert_eq!(l.rect(Region::Title, 0.0), Region::Title.reference_rect());
        let l = Layout::fixed(3840, 2160);
        let r = l.rect(Region::Reward, 0.0);
        assert_eq!((r.x, r.y), (2956, 380));
        // 21:9: UI centred.
        let l = Layout::fixed(2560, 1080);
        assert_eq!(l.rect(Region::Title, 0.0).x, 684 + 320);
    }

    #[test]
    fn rect_clamps() {
        assert_eq!(
            Rect::new(5, 5, 10, 10).clamp(8, 100),
            Some(Rect::new(5, 5, 3, 10))
        );
        assert_eq!(Rect::new(50, 5, 10, 10).clamp(8, 100), None);
    }

    #[test]
    fn finds_the_selected_offer() {
        let mut img = RgbImage::from_pixel(1920, 1080, image::Rgb([20, 25, 35]));
        let layout = Layout::fixed(1920, 1080);
        assert_eq!(selected_offer(&img, &layout), None);
        // Green accent bar of a verified offer, 4 px wide, 72 px high.
        for y in 465..537 {
            for x in 190..194 {
                img.put_pixel(x, y, image::Rgb([60, 230, 140]));
            }
        }
        // A short green speck elsewhere is not an entry.
        for y in 700..710 {
            img.put_pixel(300, y, image::Rgb([60, 230, 140]));
        }
        // From the bar to the list's right edge, a little above and below the bar.
        assert_eq!(
            selected_offer(&img, &layout),
            Some(Rect::new(186, 451, 612 - 186, 72 + 28))
        );
        // A red border all around (unverified offer): the border is the entry.
        let mut img = RgbImage::from_pixel(1920, 1080, image::Rgb([20, 25, 35]));
        for x in 190..600 {
            img.put_pixel(x, 340, image::Rgb([220, 60, 70]));
            img.put_pixel(x, 457, image::Rgb([220, 60, 70]));
        }
        for y in 340..458 {
            img.put_pixel(190, y, image::Rgb([220, 60, 70]));
        }
        assert_eq!(
            selected_offer(&img, &layout),
            Some(Rect::new(188, 338, 414, 122))
        );
    }

    #[test]
    fn finds_components() {
        #[rustfmt::skip]
        let mask = [
            true,  true,  false, false,
            false, false, false, true,
            false, false, true,  true,
        ];
        let mut c = components(&mask, 4, 3);
        c.sort_by_key(|r| r.x);
        assert_eq!(c, vec![Rect::new(0, 0, 2, 1), Rect::new(2, 1, 2, 2)]);
    }
}
