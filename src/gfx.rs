//! Software 2D renderer: tiny-skia for shapes, ab_glyph for text. Everything is addressed in
//! logical pixels; `Canvas::s` (the DPI scale) is applied here and nowhere else.

use crate::layout::Rect;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use tiny_skia::{
    Color, FillRule, FilterQuality, IntSize, LineCap, Mask, Paint, Path, PathBuilder, Pattern, Pixmap, SpreadMode,
    Stroke, Transform,
};

const KAPPA: f32 = 0.552_284_75;

// ------------------------------------------------------------------ colours
pub fn rgba(c: (u8, u8, u8), a: f32) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

// ------------------------------------------------------------------ shapes
/// Rounded rectangle with individual corner radii `[tl, tr, br, bl]`.
pub fn rrect4(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Option<Path> {
    let m = (w.min(h)) * 0.5;
    let [tl, tr, br, bl] = r.map(|v| v.clamp(0.0, m));
    let k = KAPPA;
    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    pb.cubic_to(x + w - tr + k * tr, y, x + w, y + tr - k * tr, x + w, y + tr);
    pb.line_to(x + w, y + h - br);
    pb.cubic_to(x + w, y + h - br + k * br, x + w - br + k * br, y + h, x + w - br, y + h);
    pb.line_to(x + bl, y + h);
    pb.cubic_to(x + bl - k * bl, y + h, x, y + h - bl + k * bl, x, y + h - bl);
    pb.line_to(x, y + tl);
    pb.cubic_to(x, y + tl - k * tl, x + tl - k * tl, y, x + tl, y);
    pb.close();
    pb.finish()
}

pub fn rrect(r: Rect, rad: f32) -> Option<Path> {
    rrect4(r.x, r.y, r.w, r.h, [rad; 4])
}

/// Hardware-notch silhouette: flush with the top edge, concave "ears" at the top corners,
/// rounded bottom corners.
pub fn notch_path(r: Rect, r_bot: f32, ear: f32) -> Option<Path> {
    let rb = r_bot.clamp(0.0, (r.w.min(r.h)) * 0.5);
    let ear = ear.clamp(0.0, (r.h - rb).max(0.0));
    let (x, y, w, h, k) = (r.x, r.y, r.w, r.h, KAPPA);
    let mut pb = PathBuilder::new();
    pb.move_to(x - ear, y);
    pb.quad_to(x, y, x, y + ear);
    pb.line_to(x, y + h - rb);
    pb.cubic_to(x, y + h - rb + k * rb, x + rb - k * rb, y + h, x + rb, y + h);
    pb.line_to(x + w - rb, y + h);
    pb.cubic_to(x + w - rb + k * rb, y + h, x + w, y + h - rb + k * rb, x + w, y + h - rb);
    pb.line_to(x + w, y + ear);
    pb.quad_to(x + w, y, x + w + ear, y);
    pb.close();
    pb.finish()
}

/// Circular arc as cubic Béziers. Angles in radians, 0 = 3 o'clock, positive = clockwise on screen.
pub fn arc_path(cx: f32, cy: f32, r: f32, start: f32, sweep: f32) -> Option<Path> {
    if sweep == 0.0 {
        return None;
    }
    let segs = ((sweep.abs() / std::f32::consts::FRAC_PI_2).ceil() as usize).max(1);
    let d = sweep / segs as f32;
    let k = 4.0 / 3.0 * (d / 4.0).tan();
    let mut pb = PathBuilder::new();
    let mut a = start;
    pb.move_to(cx + r * a.cos(), cy + r * a.sin());
    for _ in 0..segs {
        let (c0, s0) = (a.cos(), a.sin());
        let a1 = a + d;
        let (c1, s1) = (a1.cos(), a1.sin());
        pb.cubic_to(
            cx + r * (c0 - k * s0),
            cy + r * (s0 + k * c0),
            cx + r * (c1 + k * s1),
            cy + r * (s1 - k * c1),
            cx + r * c1,
            cy + r * s1,
        );
        a = a1;
    }
    pb.finish()
}

pub fn circle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    PathBuilder::from_circle(cx, cy, r)
}

/// Closed polygon through the given points.
pub fn poly(pts: &[(f32, f32)]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for (i, &(x, y)) in pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
    pb.finish()
}

// ------------------------------------------------------------------ images
/// Area-average resample of premultiplied RGBA. Good quality for the big downscales we do on album art.
pub fn resample_box(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (dw, dh) = (dw.max(1), dh.max(1));
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    if sw == 0 || sh == 0 || src.len() < (sw * sh * 4) as usize {
        return out;
    }
    for dy in 0..dh {
        let y0 = dy as f32 * sh as f32 / dh as f32;
        let y1 = (dy + 1) as f32 * sh as f32 / dh as f32;
        for dx in 0..dw {
            let x0 = dx as f32 * sw as f32 / dw as f32;
            let x1 = (dx + 1) as f32 * sw as f32 / dw as f32;
            let (mut acc, mut wsum) = ([0f32; 4], 0f32);
            let mut sy = y0.floor() as u32;
            while (sy as f32) < y1 && sy < sh {
                let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                let mut sx = x0.floor() as u32;
                while (sx as f32) < x1 && sx < sw {
                    let wx = (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    let w = wx * wy;
                    let i = ((sy * sw + sx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += src[i + c] as f32 * w;
                    }
                    wsum += w;
                    sx += 1;
                }
                sy += 1;
            }
            if wsum > 0.0 {
                let o = ((dy * dw + dx) * 4) as usize;
                for c in 0..4 {
                    out[o + c] = (acc[c] / wsum + 0.5) as u8;
                }
            }
        }
    }
    out
}

pub fn pixmap_from_rgba(data: Vec<u8>, w: u32, h: u32) -> Option<Pixmap> {
    Pixmap::from_vec(data, IntSize::from_wh(w, h)?)
}

// ------------------------------------------------------------------ text
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    Regular,
    Semibold,
}

struct Glyph {
    w: u32,
    h: u32,
    left: i32,
    top: i32,
    adv: f32,
    cov: Vec<u8>,
}

/// Face 0 = regular, 1 = semibold, 2.. = lazily loaded fallbacks (symbols, CJK).
const FACE_FILES: [&str; 7] =
    ["segoeui.ttf", "seguisb.ttf", "seguisym.ttf", "msyh.ttc", "YuGothM.ttc", "malgun.ttf", "simsun.ttc"];
const FALLBACK_ORDER: [usize; 6] = [0, 2, 3, 4, 5, 6];

pub struct Fonts {
    faces: Vec<Option<FontVec>>,
    tried: Vec<bool>,
    cache: HashMap<(u8, char, u32), Glyph>,
    fallback: HashMap<char, u8>,
}

impl Fonts {
    fn new() -> Self {
        Self {
            faces: (0..FACE_FILES.len()).map(|_| None).collect(),
            tried: vec![false; FACE_FILES.len()],
            cache: HashMap::new(),
            fallback: HashMap::new(),
        }
    }

    /// Loads face `i` on first use. Fonts come from the Windows font folder, so they cost no binary size.
    fn load(&mut self, i: usize) -> bool {
        if !self.tried[i] {
            self.tried[i] = true;
            let dir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
            let path = std::path::PathBuf::from(dir).join("Fonts").join(FACE_FILES[i]);
            if let Ok(bytes) = std::fs::read(path) {
                self.faces[i] = FontVec::try_from_vec_and_index(bytes, 0).ok();
            }
        }
        self.faces[i].is_some()
    }

    fn has(&self, i: usize, ch: char) -> bool {
        self.faces[i].as_ref().map_or(false, |f| f.glyph_id(ch).0 != 0)
    }

    fn resolve(&mut self, ch: char, weight: Weight) -> Option<u8> {
        let primary = if weight == Weight::Semibold && self.load(1) { 1 } else if self.load(0) { 0 } else { return None };
        if self.has(primary, ch) {
            return Some(primary as u8);
        }
        if let Some(&f) = self.fallback.get(&ch) {
            return if f == 255 { None } else { Some(f) };
        }
        for &i in FALLBACK_ORDER.iter() {
            if i != primary && self.load(i) && self.has(i, ch) {
                self.fallback.insert(ch, i as u8);
                return Some(i as u8);
            }
        }
        self.fallback.insert(ch, 255);
        None
    }

    fn glyph(&mut self, ch: char, weight: Weight, px: f32) -> Option<&Glyph> {
        let face = self.resolve(ch, weight)?;
        let key = (face, ch, (px * 4.0).round() as u32);
        if !self.cache.contains_key(&key) {
            if self.cache.len() > 4096 {
                self.cache.clear();
            }
            let font = self.faces[face as usize].as_ref()?;
            let scale = PxScale::from(px);
            let gid = font.glyph_id(ch);
            let adv = font.as_scaled(scale).h_advance(gid);
            let g = match font.outline_glyph(gid.with_scale(scale)) {
                Some(og) => {
                    let b = og.px_bounds();
                    let (left, top) = (b.min.x.floor() as i32, b.min.y.floor() as i32);
                    let w = (b.max.x.ceil() as i32 - left).max(0) as u32;
                    let h = (b.max.y.ceil() as i32 - top).max(0) as u32;
                    let mut cov = vec![0u8; (w * h) as usize];
                    og.draw(|x, y, c| {
                        if x < w && y < h {
                            cov[(y * w + x) as usize] = (c * 255.0 + 0.5) as u8;
                        }
                    });
                    Glyph { w, h, left, top, adv, cov }
                }
                None => Glyph { w: 0, h: 0, left: 0, top: 0, adv, cov: Vec::new() },
            };
            self.cache.insert(key, g);
        }
        self.cache.get(&key)
    }
}

/// Alpha-blend a coverage bitmap onto the premultiplied RGBA pixmap, honouring the clip mask.
fn blit(pm: &mut Pixmap, mask: Option<&[u8]>, g: &Glyph, ox: i32, oy: i32, c: (u8, u8, u8), a: u32) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for gy in 0..g.h as i32 {
        let y = oy + gy;
        if y < 0 || y >= ph {
            continue;
        }
        for gx in 0..g.w as i32 {
            let x = ox + gx;
            if x < 0 || x >= pw {
                continue;
            }
            let cov = g.cov[(gy as u32 * g.w + gx as u32) as usize] as u32;
            if cov == 0 {
                continue;
            }
            let mut sa = cov * a / 255;
            if let Some(m) = mask {
                sa = sa * m[(y * pw + x) as usize] as u32 / 255;
            }
            if sa == 0 {
                continue;
            }
            let i = ((y * pw + x) * 4) as usize;
            let inv = 255 - sa;
            data[i] = ((c.0 as u32 * sa + data[i] as u32 * inv + 127) / 255) as u8;
            data[i + 1] = ((c.1 as u32 * sa + data[i + 1] as u32 * inv + 127) / 255) as u8;
            data[i + 2] = ((c.2 as u32 * sa + data[i + 2] as u32 * inv + 127) / 255) as u8;
            data[i + 3] = (sa + (data[i + 3] as u32 * inv + 127) / 255).min(255) as u8;
        }
    }
}

// ------------------------------------------------------------------ canvas
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipSel {
    None,
    Pill,
    Tmp,
}

pub struct Canvas {
    pub pm: Pixmap,
    /// DPI scale: physical pixels per logical pixel.
    pub s: f32,
    pill_path: Option<Path>,
    pill_mask: Option<Mask>,
    tmp_mask: Option<Mask>,
    sel: ClipSel,
    pub fonts: Fonts,
}

impl Canvas {
    pub fn new(w_px: u32, h_px: u32, scale: f32) -> Self {
        Self {
            pm: Pixmap::new(w_px.max(1), h_px.max(1)).expect("pixmap allocation"),
            s: scale,
            pill_path: None,
            pill_mask: None,
            tmp_mask: None,
            sel: ClipSel::None,
            fonts: Fonts::new(),
        }
    }

    /// Resize for a new DPI scale (the glyph cache is keyed by pixel size, so it stays valid).
    pub fn resize(&mut self, w_px: u32, h_px: u32, scale: f32) {
        self.pm = Pixmap::new(w_px.max(1), h_px.max(1)).expect("pixmap allocation");
        self.s = scale;
        self.pill_mask = None;
        self.tmp_mask = None;
        self.sel = ClipSel::None;
    }

    pub fn begin(&mut self) {
        self.pm.fill(Color::TRANSPARENT);
        self.sel = ClipSel::None;
        self.pill_path = None;
        self.pill_mask = None;
    }

    fn tf(&self) -> Transform {
        Transform::from_scale(self.s, self.s)
    }

    fn new_mask(&self, path: &Path) -> Option<Mask> {
        let mut m = Mask::new(self.pm.width(), self.pm.height())?;
        m.fill_path(path, FillRule::Winding, true, self.tf());
        Some(m)
    }

    /// Clip everything drawn afterwards to `path` (the pill silhouette).
    pub fn set_clip(&mut self, path: Path) {
        self.pill_mask = self.new_mask(&path);
        self.pill_path = Some(path);
        self.sel = if self.pill_mask.is_some() { ClipSel::Pill } else { ClipSel::None };
    }

    /// Additionally clip to a rectangle (intersected with the pill).
    pub fn clip_rect(&mut self, r: Rect) {
        let Some(rp) = rrect(r, 0.0) else { return };
        let Some(mut m) = Mask::new(self.pm.width(), self.pm.height()) else { return };
        m.fill_path(&rp, FillRule::Winding, false, self.tf());
        if let Some(pp) = &self.pill_path {
            m.intersect_path(pp, FillRule::Winding, true, self.tf());
        }
        self.tmp_mask = Some(m);
        self.sel = ClipSel::Tmp;
    }

    pub fn unclip_rect(&mut self) {
        self.sel = if self.pill_mask.is_some() { ClipSel::Pill } else { ClipSel::None };
    }

    pub fn fill(&mut self, path: &Path, color: Color) {
        let mut p = Paint::default();
        p.set_color(color);
        p.anti_alias = true;
        let tf = self.tf();
        let mask = match self.sel {
            ClipSel::None => None,
            ClipSel::Pill => self.pill_mask.as_ref(),
            ClipSel::Tmp => self.tmp_mask.as_ref(),
        };
        self.pm.fill_path(path, &p, FillRule::Winding, tf, mask);
    }

    pub fn stroke(&mut self, path: &Path, width: f32, color: Color, round_caps: bool) {
        let mut p = Paint::default();
        p.set_color(color);
        p.anti_alias = true;
        let st = Stroke { width, line_cap: if round_caps { LineCap::Round } else { LineCap::Butt }, ..Stroke::default() };
        let tf = self.tf();
        let mask = match self.sel {
            ClipSel::None => None,
            ClipSel::Pill => self.pill_mask.as_ref(),
            ClipSel::Tmp => self.tmp_mask.as_ref(),
        };
        self.pm.stroke_path(path, &p, &st, tf, mask);
    }

    pub fn fill_rrect(&mut self, r: Rect, rad: f32, color: Color) {
        if let Some(p) = rrect(r, rad) {
            self.fill(&p, color);
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        if let Some(p) = circle(cx, cy, r) {
            self.fill(&p, color);
        }
    }

    /// Draw a pre-sized image (physical pixels 1:1) into `dst` with rounded corners.
    pub fn draw_image(&mut self, img: &Pixmap, dst: Rect, radius: f32, opacity: f32) {
        let Some(path) = rrect(dst, radius) else { return };
        let pat_tf = Transform::from_translate(dst.x, dst.y)
            .pre_scale(dst.w / img.width() as f32, dst.h / img.height() as f32);
        let mut p = Paint::default();
        p.shader = Pattern::new(img.as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, opacity.clamp(0.0, 1.0), pat_tf);
        p.anti_alias = true;
        let tf = self.tf();
        let mask = match self.sel {
            ClipSel::None => None,
            ClipSel::Pill => self.pill_mask.as_ref(),
            ClipSel::Tmp => self.tmp_mask.as_ref(),
        };
        self.pm.fill_path(&path, &p, FillRule::Winding, tf, mask);
    }

    // ---- text (sizes and positions are logical; `baseline` is the y of the text baseline)
    pub fn text(&mut self, s: &str, x: f32, baseline: f32, size: f32, wt: Weight, color: (u8, u8, u8), alpha: f32) {
        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u32;
        if a == 0 || s.is_empty() {
            return;
        }
        let px = size * self.s;
        let mut pen = x * self.s;
        let by = (baseline * self.s).round() as i32;
        let mask: Option<&[u8]> = match self.sel {
            ClipSel::None => None,
            ClipSel::Pill => self.pill_mask.as_ref().map(|m| m.data()),
            ClipSel::Tmp => self.tmp_mask.as_ref().map(|m| m.data()),
        };
        for ch in s.chars() {
            match self.fonts.glyph(ch, wt, px) {
                Some(g) => {
                    if g.w > 0 {
                        blit(&mut self.pm, mask, g, pen.round() as i32 + g.left, by + g.top, color, a);
                    }
                    pen += g.adv;
                }
                None => pen += px * 0.5,
            }
        }
    }

    pub fn text_width(&mut self, s: &str, size: f32, wt: Weight) -> f32 {
        let px = size * self.s;
        let mut w = 0.0;
        for ch in s.chars() {
            w += self.fonts.glyph(ch, wt, px).map_or(px * 0.5, |g| g.adv);
        }
        w / self.s
    }

    pub fn text_centered(&mut self, s: &str, cx: f32, baseline: f32, size: f32, wt: Weight, color: (u8, u8, u8), alpha: f32) {
        let w = self.text_width(s, size, wt);
        self.text(s, cx - w * 0.5, baseline, size, wt, color, alpha);
    }

    /// Truncate with an ellipsis so the text is at most `max_w` logical pixels wide.
    pub fn fit(&mut self, s: &str, size: f32, wt: Weight, max_w: f32) -> String {
        if self.text_width(s, size, wt) <= max_w {
            return s.to_string();
        }
        let ell = self.text_width("\u{2026}", size, wt);
        let px = size * self.s;
        let mut out = String::new();
        let mut w = 0.0;
        for ch in s.chars() {
            let cw = self.fonts.glyph(ch, wt, px).map_or(px * 0.5, |g| g.adv) / self.s;
            if w + cw + ell > max_w {
                break;
            }
            w += cw;
            out.push(ch);
        }
        out.push('\u{2026}');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_resample_preserves_flat_colour() {
        let src = vec![200u8; 8 * 8 * 4];
        let out = resample_box(&src, 8, 8, 3, 3);
        assert!(out.iter().all(|&v| v == 200));
    }

    #[test]
    fn box_resample_averages() {
        // 2x1 image: black | white  ->  1x1 grey
        let src = [0, 0, 0, 255, 255, 255, 255, 255];
        let out = resample_box(&src, 2, 1, 1, 1);
        assert!((out[0] as i32 - 128).abs() <= 1 && out[3] == 255);
    }

    #[test]
    fn shapes_build() {
        assert!(rrect(Rect::new(0.0, 0.0, 100.0, 40.0), 20.0).is_some());
        assert!(notch_path(Rect::new(10.0, 0.0, 184.0, 32.0), 16.0, 8.0).is_some());
        assert!(arc_path(50.0, 50.0, 20.0, -1.57, 3.0).is_some());
        assert!(arc_path(50.0, 50.0, 20.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn canvas_draws_and_clips() {
        let mut c = Canvas::new(120, 60, 1.0);
        c.begin();
        c.set_clip(rrect(Rect::new(0.0, 0.0, 60.0, 60.0), 0.0).unwrap());
        c.fill_rrect(Rect::new(0.0, 0.0, 120.0, 60.0), 0.0, rgba((255, 0, 0), 1.0));
        let px = |x: usize, y: usize| c.pm.data()[(y * 120 + x) * 4 + 3];
        assert_eq!(px(30, 30), 255); // inside clip
        assert_eq!(px(90, 30), 0); // outside clip stays transparent
    }
}
