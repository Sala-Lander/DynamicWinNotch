//! Everything that is drawn: the pill silhouette, the compact (collapsed / peek) content and the
//! three tab pages. Pure drawing: it reads a `Scene` snapshot and paints into the `Canvas`.
//! All coordinates are logical pixels; positions of controls come from `layout::Geo`, the same
//! source the hit-testing uses, so drawing and clicking can never disagree.

use crate::config::Config;
use crate::gfx::{self, rgba, Canvas, Weight};
use crate::history::History;
use crate::layout::{Geo, Hit, Rect, Style, Tab, ROW_H, VISIBLE_ROWS};
use crate::media::MediaView;
use crate::sys::Stats;
use crate::util::{fmt_time, lerp, marquee_offset, smoothstep};
use std::f32::consts::{FRAC_PI_2, TAU};
use tiny_skia::Color;

const WHITE: (u8, u8, u8) = (255, 255, 255);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeekKind {
    Track,
    Clip,
}

/// Text shown in the widened "peek" pill (new track / new clipboard text).
#[derive(Clone, Debug)]
pub struct PeekInfo {
    pub kind: PeekKind,
    pub line1: String,
    pub line2: String,
}

pub struct Scene<'a> {
    pub cfg: &'a Config,
    pub geo: &'a Geo,
    /// Current (animated) pill rectangle.
    pub pill: Rect,
    /// 0 = collapsed height, 1 = fully expanded height.
    pub t_open: f32,
    /// 0 = collapsed content, 1 = peek content.
    pub peek_mix: f32,
    /// Fractional tab index while the highlight slides.
    pub tab_pos: f32,
    pub hover: Hit,
    pub pinned: bool,
    /// Seconds since start; drives the equalizer and title marquee phases.
    pub now: f32,
    pub media: &'a MediaView,
    pub media_pos_ms: i64,
    pub clip: &'a History,
    /// Clipboard scroll position in rows.
    pub scroll: f32,
    /// (row index, 0..1 intensity) of the row that was just copied.
    pub flash: Option<(usize, f32)>,
    /// Displayed (spring-smoothed) CPU / RAM / battery fractions.
    pub gauges: [f32; 3],
    pub stats: &'a Stats,
    pub accent: (u8, u8, u8),
    pub peek: Option<&'a PeekInfo>,
}

fn white(a: f32) -> Color {
    rgba(WHITE, a)
}

/// Paints one frame. Returns true when something needs continuous redraws (scrolling title).
pub fn draw(cv: &mut Canvas, sc: &Scene) -> bool {
    cv.begin();
    let p = sc.pill;
    let notch = sc.cfg.style == Style::Notch;
    let r_bot = (p.h * 0.5).min(30.0);
    let ear = if notch { lerp(8.0, 12.0, sc.t_open) } else { 0.0 };
    let silhouette = |r: Rect| {
        if notch {
            gfx::notch_path(r, r_bot, ear)
        } else {
            gfx::rrect(r, r_bot)
        }
    };

    // Soft shadow: a few translucent, slightly larger silhouettes underneath.
    let sh = smoothstep(0.02, 0.5, sc.t_open);
    if sh > 0.01 {
        for i in (1..=5).rev() {
            let d = i as f32 * 2.6;
            if let Some(path) = silhouette(p.inflate_xy(d, d * 0.9)) {
                cv.fill(&path, rgba((0, 0, 0), 0.045 * sh));
            }
        }
    }

    // The matte black body.
    if let Some(path) = silhouette(p) {
        cv.fill(&path, rgba((0, 0, 0), 1.0));
        if !notch {
            cv.stroke(&path, 1.0, white(0.07), false);
        }
    }
    if let Some(path) = silhouette(p) {
        cv.set_clip(path);
    }

    let compact_a = 1.0 - smoothstep(0.12, 0.40, sc.t_open);
    let exp_a = smoothstep(0.50, 0.92, sc.t_open);
    let mut marquee = false;
    if compact_a > 0.01 {
        draw_compact(cv, sc, compact_a);
    }
    if exp_a > 0.01 {
        marquee = draw_expanded(cv, sc, exp_a);
    }
    marquee
}

// ------------------------------------------------------------------ compact
fn draw_compact(cv: &mut Canvas, sc: &Scene, a: f32) {
    let p = sc.pill;
    let m = &sc.media.info;
    let show_media = m.has_session && !(m.title.is_empty() && m.artist.is_empty());
    let ca = a * (1.0 - sc.peek_mix);
    let pa = a * sc.peek_mix;

    if ca > 0.01 && show_media {
        let cy = p.cy();
        if let Some(img) = &sc.media.art {
            cv.draw_image(img, Rect::new(p.x + 11.0, cy - 10.0, 20.0, 20.0), 6.0, ca);
        }
        draw_eq(cv, p.right() - 14.0, cy, m.playing, sc.now, sc.accent, ca);
    }
    if pa > 0.01 {
        if let Some(pk) = sc.peek {
            draw_peek(cv, sc, pk, pa);
        }
    }
}

fn draw_peek(cv: &mut Canvas, sc: &Scene, pk: &PeekInfo, a: f32) {
    let p = sc.pill;
    let cy = p.cy();
    let mut x = p.x + 10.0;
    match pk.kind {
        PeekKind::Track => {
            let r = Rect::new(x, cy - 15.0, 30.0, 30.0);
            match &sc.media.art {
                Some(img) => cv.draw_image(img, r, 8.0, a),
                None => {
                    cv.fill_rrect(r, 8.0, white(0.10 * a));
                    draw_note(cv, r.cx(), r.cy(), 0.55, 0.40 * a);
                }
            }
            x += 40.0;
        }
        PeekKind::Clip => {
            draw_clip_icon(cv, x + 15.0, cy, a, sc.accent);
            x += 40.0;
        }
    }
    let right_pad = if pk.kind == PeekKind::Track { 38.0 } else { 14.0 };
    let w = (p.right() - right_pad - x).max(20.0);
    let l1 = cv.fit(&pk.line1, 13.0, Weight::Semibold, w);
    cv.text(&l1, x, cy - 2.0, 13.0, Weight::Semibold, WHITE, a);
    let l2 = cv.fit(&pk.line2, 11.0, Weight::Regular, w);
    cv.text(&l2, x, cy + 12.0, 11.0, Weight::Regular, WHITE, 0.55 * a);
    if pk.kind == PeekKind::Track {
        draw_eq(cv, p.right() - 14.0, cy, sc.media.info.playing, sc.now, sc.accent, a);
    }
}

/// Four bouncing bars while playing, four resting bars while paused. `right` is the bars' right edge.
fn draw_eq(cv: &mut Canvas, right: f32, cy: f32, playing: bool, t: f32, color: (u8, u8, u8), a: f32) {
    const N: usize = 4;
    const REST: [f32; N] = [0.25, 0.50, 0.35, 0.20];
    let (bw, gap, max_h) = (2.6f32, 2.2f32, 14.0f32);
    let x0 = right - (N as f32 * bw + (N as f32 - 1.0) * gap);
    for i in 0..N {
        let f = if playing {
            0.5 + 0.5 * (t * (5.0 + 1.7 * i as f32) + i as f32 * 1.9).sin()
        } else {
            REST[i]
        };
        let h = (0.22 + 0.78 * f) * max_h;
        let alpha = if playing { a } else { a * 0.55 };
        cv.fill_rrect(Rect::new(x0 + i as f32 * (bw + gap), cy + max_h * 0.5 - h, bw, h), 1.3, rgba(color, alpha));
    }
}

// ----------------------------------------------------------------- expanded
fn draw_expanded(cv: &mut Canvas, sc: &Scene, a: f32) -> bool {
    let g = sc.geo;

    // Sliding tab highlight + labels.
    let step = g.tabs[1].x - g.tabs[0].x;
    let hx = g.tabs[0].x + sc.tab_pos.clamp(0.0, 2.0) * step;
    cv.fill_rrect(Rect::new(hx, g.tabs[0].y, g.tabs[0].w, g.tabs[0].h), 12.0, white(0.13 * a));
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let r = g.tabs[i];
        let act = 1.0 - (sc.tab_pos - i as f32).abs().min(1.0);
        let hov = if matches!(sc.hover, Hit::Tab(j) if j == i) { 1.0 } else { 0.0 };
        let alpha = (0.45 + 0.55 * act + 0.25 * hov * (1.0 - act)).min(1.0) * a;
        cv.text_centered(tab.label(), r.cx(), r.y + 16.5, 12.0, Weight::Semibold, WHITE, alpha);
    }

    draw_pin(cv, g.pin, sc.pinned, sc.hover == Hit::Pin, sc.accent, a);

    let clip_vis = 1.0 - (sc.tab_pos - 1.0).abs().min(1.0);
    if clip_vis > 0.01 && !sc.clip.is_empty() {
        let al = if sc.hover == Hit::Clear { 0.9 } else { 0.5 };
        cv.text("Clear", g.clear.x + 4.0, g.clear.y + 16.0, 11.5, Weight::Semibold, WHITE, al * clip_vis * a);
    }

    // Pages: neighbours slide and cross-fade while the highlight moves.
    let mut marquee = false;
    for i in 0..3 {
        let d = i as f32 - sc.tab_pos;
        if d.abs() >= 1.0 {
            continue;
        }
        let pa = a * (1.0 - d.abs());
        let ox = d * 40.0;
        match i {
            0 => marquee = page_media(cv, sc, ox, pa),
            1 => page_clip(cv, sc, ox, pa),
            _ => page_system(cv, sc, ox, pa),
        }
    }
    marquee
}

// -------------------------------------------------------------------- media
fn page_media(cv: &mut Canvas, sc: &Scene, ox: f32, a: f32) -> bool {
    let g = sc.geo;
    let m = &sc.media.info;

    if !m.has_session {
        let cx = g.content.cx() + ox;
        let cy = g.content.y + 38.0;
        draw_note(cv, cx, cy, 1.0, 0.28 * a);
        cv.text_centered("Nothing playing", cx, cy + 40.0, 14.0, Weight::Semibold, WHITE, 0.85 * a);
        cv.text_centered("Play something in any app", cx, cy + 58.0, 11.5, Weight::Regular, WHITE, 0.45 * a);
        return false;
    }

    // Cover art.
    let art = Rect::new(g.art.x + ox, g.art.y, g.art.w, g.art.h);
    match &sc.media.art {
        Some(img) => cv.draw_image(img, art, 14.0, a),
        None => {
            cv.fill_rrect(art, 14.0, white(0.08 * a));
            draw_note(cv, art.cx(), art.cy(), 1.1, 0.30 * a);
        }
    }

    // Title (scrolls when it does not fit) and subtitle.
    let tx = g.text_x + ox;
    let title = if m.title.is_empty() { "Unknown title" } else { m.title.as_str() };
    let tw = cv.text_width(title, 16.0, Weight::Semibold);
    let mut marquee = false;
    if tw <= g.text_w {
        cv.text(title, tx, g.title_y, 16.0, Weight::Semibold, WHITE, a);
    } else {
        marquee = true;
        let off = marquee_offset(tw - g.text_w, sc.now);
        cv.clip_rect(Rect::new(tx, g.title_y - 18.0, g.text_w, 24.0));
        cv.text(title, tx - off, g.title_y, 16.0, Weight::Semibold, WHITE, a);
        cv.unclip_rect();
    }
    let sub = match (m.artist.is_empty(), m.album.is_empty()) {
        (false, false) => format!("{} \u{b7} {}", m.artist, m.album),
        (false, true) => m.artist.clone(),
        (true, false) => m.album.clone(),
        (true, true) => m.app.clone(),
    };
    let sub = cv.fit(&sub, 12.5, Weight::Regular, g.text_w);
    cv.text(&sub, tx, g.artist_y, 12.5, Weight::Regular, WHITE, 0.62 * a);

    // Progress bar and times.
    let bar = Rect::new(g.bar.x + ox, g.bar.y, g.bar.w, g.bar.h);
    cv.fill_rrect(bar, 2.5, white(0.14 * a));
    if m.dur_ms > 0 {
        let f = (sc.media_pos_ms as f32 / m.dur_ms as f32).clamp(0.0, 1.0);
        if f > 0.002 {
            cv.fill_rrect(Rect::new(bar.x, bar.y, (bar.w * f).max(bar.h), bar.h), 2.5, rgba(sc.accent, a));
        }
        if m.can_seek && matches!(sc.hover, Hit::Seek(_)) {
            cv.fill_circle(bar.x + bar.w * f, bar.cy(), 5.0, white(a));
        }
        let left = fmt_time(sc.media_pos_ms);
        let right = fmt_time(m.dur_ms);
        let rw = cv.text_width(&right, 10.5, Weight::Regular);
        cv.text(&left, tx, g.time_y, 10.5, Weight::Regular, WHITE, 0.5 * a);
        cv.text(&right, tx + g.text_w - rw, g.time_y, 10.5, Weight::Regular, WHITE, 0.5 * a);
    }
    if !m.app.is_empty() {
        cv.text_centered(&m.app, tx + g.text_w * 0.5, g.time_y, 10.5, Weight::Regular, WHITE, 0.28 * a);
    }

    // Transport controls.
    let (pr, pl, nx) = (g.prev, g.play, g.next);
    if sc.hover == Hit::Prev && m.can_prev {
        cv.fill_circle(pr.cx() + ox, pr.cy(), 15.0, white(0.10 * a));
    }
    draw_prev(cv, pr.cx() + ox, pr.cy(), 12.0, white(if m.can_prev { 0.95 } else { 0.28 } * a));
    if sc.hover == Hit::Next && m.can_next {
        cv.fill_circle(nx.cx() + ox, nx.cy(), 15.0, white(0.10 * a));
    }
    draw_next(cv, nx.cx() + ox, nx.cy(), 12.0, white(if m.can_next { 0.95 } else { 0.28 } * a));
    let radius = pl.w * 0.5 + if sc.hover == Hit::Play { 1.5 } else { 0.0 };
    let pa = if m.can_toggle { a } else { a * 0.5 };
    cv.fill_circle(pl.cx() + ox, pl.cy(), radius, white(pa));
    draw_play_pause(cv, pl.cx() + ox, pl.cy(), 13.0, m.playing, rgba((0, 0, 0), pa));

    marquee
}

// ---------------------------------------------------------------- clipboard
fn page_clip(cv: &mut Canvas, sc: &Scene, ox: f32, a: f32) {
    let g = sc.geo;
    let c = g.content;
    let items = sc.clip.items();

    if items.is_empty() {
        let cx = c.cx() + ox;
        draw_clip_icon(cv, cx, c.y + 34.0, 0.30 * a, sc.accent);
        cv.text_centered("Nothing copied yet", cx, c.y + 72.0, 14.0, Weight::Semibold, WHITE, 0.85 * a);
        cv.text_centered("Text you copy shows up here", cx, c.y + 90.0, 11.5, Weight::Regular, WHITE, 0.45 * a);
        return;
    }

    cv.clip_rect(Rect::new(c.x - 8.0, c.y, c.w + 16.0, c.h));
    let first = sc.scroll.floor().max(0.0) as usize;
    let last = (first + VISIBLE_ROWS + 1).min(items.len());
    for i in first..last {
        let e = &items[i];
        let y = c.y + (i as f32 - sc.scroll) * ROW_H;
        let row = Rect::new(c.x + ox, y + 2.0, c.w, ROW_H - 4.0);
        let hov = matches!(sc.hover, Hit::Row(j) if j == i);
        if hov {
            cv.fill_rrect(row, 9.0, white(0.08 * a));
        }
        let flash = match sc.flash {
            Some((fi, v)) if fi == i => v,
            _ => 0.0,
        };
        if flash > 0.0 {
            cv.fill_rrect(row, 9.0, rgba(sc.accent, 0.28 * flash * a));
        }
        let mut tx = row.x + 12.0;
        if e.pinned {
            cv.fill_circle(row.x + 11.0, row.cy(), 3.0, rgba(sc.accent, a));
            tx += 11.0;
        }
        let txt = cv.fit(&e.preview, 12.5, Weight::Regular, row.w - (tx - row.x) - 64.0);
        cv.text(&txt, tx, row.y + 16.5, 12.5, Weight::Regular, WHITE, 0.88 * a);

        let label: Option<(&str, (u8, u8, u8), f32)> = if flash > 0.05 {
            Some(("Copied", sc.accent, flash.min(1.0) * a))
        } else if hov {
            Some(("Copy", WHITE, 0.5 * a))
        } else {
            None
        };
        if let Some((s, rgb, al)) = label {
            let w = cv.text_width(s, 10.5, Weight::Semibold);
            cv.text(s, row.right() - 10.0 - w, row.y + 16.0, 10.5, Weight::Semibold, rgb, al);
        }
    }
    cv.unclip_rect();

    // Scroll thumb.
    if items.len() > VISIBLE_ROWS {
        let max_scroll = (items.len() - VISIBLE_ROWS) as f32;
        let th = (c.h * VISIBLE_ROWS as f32 / items.len() as f32).max(16.0);
        let ty = c.y + (c.h - th) * (sc.scroll / max_scroll).clamp(0.0, 1.0);
        cv.fill_rrect(Rect::new(c.right() + 6.0 + ox, ty, 3.0, th), 1.5, white(0.22 * a));
    }
}

// ------------------------------------------------------------------- system
fn load_color(base: (u8, u8, u8), pct: f32) -> (u8, u8, u8) {
    if pct >= 90.0 {
        (255, 69, 58)
    } else if pct >= 75.0 {
        (255, 159, 10)
    } else {
        base
    }
}

fn battery_color(pct: u8, charging: bool) -> (u8, u8, u8) {
    if charging || pct > 30 {
        (48, 209, 88)
    } else if pct > 15 {
        (255, 159, 10)
    } else {
        (255, 69, 58)
    }
}

fn battery_sub(st: &Stats) -> String {
    if st.charging {
        "Charging".to_string()
    } else if st.on_ac {
        "Plugged in".to_string()
    } else if let Some(s) = st.secs_left {
        format!("{}h {:02}m left", s / 3600, (s % 3600) / 60)
    } else {
        "On battery".to_string()
    }
}

fn page_system(cv: &mut Canvas, sc: &Scene, ox: f32, a: f32) {
    let g = sc.geo;
    let st = sc.stats;
    let (cy, r) = (g.ring_y, g.ring_r);

    let (batt_center, batt_sub, batt_col) = match st.battery {
        Some(b) => (format!("{b}%"), battery_sub(st), battery_color(b, st.charging)),
        None => ("\u{2014}".to_string(), "No battery".to_string(), (120, 120, 128)),
    };
    let cols: [(&str, String, String, (u8, u8, u8), f32); 3] = [
        ("CPU", format!("{:.0}%", st.cpu), format!("{} threads", st.threads), load_color((90, 200, 250), st.cpu), sc.gauges[0]),
        (
            "Memory",
            format!("{:.0}%", st.ram_pct),
            format!("{:.1} / {:.0} GB", st.ram_used_gb, st.ram_total_gb),
            load_color((191, 90, 242), st.ram_pct),
            sc.gauges[1],
        ),
        ("Battery", batt_center, batt_sub, batt_col, sc.gauges[2]),
    ];

    for (i, (label, center, sub, color, frac)) in cols.iter().enumerate() {
        let cx = g.content.x + g.col_w * (i as f32 + 0.5) + ox;
        if let Some(track) = gfx::circle(cx, cy, r) {
            cv.stroke(&track, 6.0, white(0.10 * a), false);
        }
        let f = frac.clamp(0.0, 1.0);
        if f > 0.004 {
            if let Some(arc) = gfx::arc_path(cx, cy, r, -FRAC_PI_2, TAU * f.min(0.9999)) {
                cv.stroke(&arc, 6.0, rgba(*color, a), true);
            }
        }
        cv.text_centered(center, cx, cy + 5.5, 15.0, Weight::Semibold, WHITE, a);
        cv.text_centered(label, cx, cy + r + 21.0, 12.0, Weight::Semibold, WHITE, 0.9 * a);
        cv.text_centered(sub, cx, cy + r + 36.0, 10.5, Weight::Regular, WHITE, 0.45 * a);
    }
}

// -------------------------------------------------------------------- icons
fn tri(cv: &mut Canvas, pts: &[(f32, f32)], col: Color) {
    if let Some(p) = gfx::poly(pts) {
        cv.fill(&p, col);
    }
}

fn draw_next(cv: &mut Canvas, cx: f32, cy: f32, s: f32, col: Color) {
    tri(cv, &[(cx - 0.62 * s, cy - 0.5 * s), (cx + 0.2 * s, cy), (cx - 0.62 * s, cy + 0.5 * s)], col);
    cv.fill_rrect(Rect::new(cx + 0.32 * s, cy - 0.5 * s, 0.2 * s, s), 0.08 * s, col);
}

fn draw_prev(cv: &mut Canvas, cx: f32, cy: f32, s: f32, col: Color) {
    tri(cv, &[(cx + 0.62 * s, cy - 0.5 * s), (cx - 0.2 * s, cy), (cx + 0.62 * s, cy + 0.5 * s)], col);
    cv.fill_rrect(Rect::new(cx - 0.52 * s, cy - 0.5 * s, 0.2 * s, s), 0.08 * s, col);
}

/// Shows the *action*: a pause glyph while playing, a play triangle while paused.
fn draw_play_pause(cv: &mut Canvas, cx: f32, cy: f32, s: f32, playing: bool, col: Color) {
    if playing {
        cv.fill_rrect(Rect::new(cx - 0.5 * s, cy - 0.5 * s, 0.3 * s, s), 0.1 * s, col);
        cv.fill_rrect(Rect::new(cx + 0.2 * s, cy - 0.5 * s, 0.3 * s, s), 0.1 * s, col);
    } else {
        tri(cv, &[(cx - 0.4 * s, cy - 0.58 * s), (cx + 0.62 * s, cy), (cx - 0.4 * s, cy + 0.58 * s)], col);
    }
}

fn draw_note(cv: &mut Canvas, cx: f32, cy: f32, s: f32, a: f32) {
    cv.fill_circle(cx - 4.5 * s, cy + 9.0 * s, 5.0 * s, white(a));
    cv.fill_rrect(Rect::new(cx - 0.4 * s, cy - 13.0 * s, 2.6 * s, 22.0 * s), 1.2 * s, white(a));
    tri(
        cv,
        &[(cx + 2.2 * s, cy - 13.0 * s), (cx + 11.0 * s, cy - 8.0 * s), (cx + 11.0 * s, cy - 2.5 * s), (cx + 2.2 * s, cy - 6.5 * s)],
        white(a),
    );
}

fn draw_clip_icon(cv: &mut Canvas, cx: f32, cy: f32, a: f32, accent: (u8, u8, u8)) {
    cv.fill_rrect(Rect::new(cx - 7.0, cy - 8.5, 14.0, 17.0), 3.0, white(0.92 * a));
    cv.fill_rrect(Rect::new(cx - 3.5, cy - 10.5, 7.0, 4.5), 1.8, rgba(accent, a));
    for k in 0..3 {
        cv.fill_rrect(Rect::new(cx - 4.0, cy - 2.5 + k as f32 * 4.2, 8.0, 1.5), 0.7, rgba((0, 0, 0), 0.55 * a));
    }
}

/// Thumbtack: lit with the accent colour while the island is pinned open.
fn draw_pin(cv: &mut Canvas, r: Rect, on: bool, hover: bool, accent: (u8, u8, u8), a: f32) {
    let (cx, cy) = (r.cx(), r.cy());
    if hover {
        cv.fill_circle(cx, cy, 12.0, white(0.10 * a));
    }
    let (col, al) = if on { (accent, 1.0) } else { (WHITE, if hover { 0.85 } else { 0.5 }) };
    cv.fill_circle(cx, cy - 2.5, 3.6, rgba(col, al * a));
    cv.fill_rrect(Rect::new(cx - 0.8, cy + 0.5, 1.6, 7.0), 0.8, rgba(col, al * a));
}
