//! Everything that is drawn: the pill silhouette, the compact (collapsed / peek) content and the
//! three tab pages. Pure drawing: it reads a `Scene` snapshot and paints into the `Canvas`.
//! All coordinates are logical pixels; positions of controls come from `layout::Geo`, the same
//! source the hit-testing uses, so drawing and clicking can never disagree.

use crate::config::Config;
use crate::gfx::{self, pixmap_from_rgba, rgba, Canvas, Weight};
use crate::history::{History, Kind};
use crate::layout::{
    game_bar_rect, settings_ctl_right, Geo, Hit, Rect, Style, Tab, CLIP_LIST_OFFSET, CLIP_VISIBLE_ROWS, ROW_H, TAB_COUNT,
    TIMER_PRESETS, VISIBLE_ROWS,
};
use crate::media::MediaView;
use crate::settings::{self, Ctl};
use crate::sys::Stats;
use crate::timer::fmt_clock;
use crate::util::{fmt_rate, fmt_time, lerp, marquee_offset, smoothstep};
use std::f32::consts::{FRAC_PI_2, TAU};
use tiny_skia::{Color, Pixmap};

const WHITE: (u8, u8, u8) = (255, 255, 255);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeekKind {
    Track,
    Clip,
    Timer,
    Power(PowerIcon),
    /// A toast notification (app name is drawn as the line1/line2 already carry the text).
    Notification,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerIcon {
    Plugged,
    Unplugged,
    Low,
    Full,
}

/// Text shown in the widened "peek" pill (new track / new clipboard item / timer / power).
#[derive(Clone)]
pub struct PeekInfo {
    pub kind: PeekKind,
    pub line1: String,
    pub line2: String,
    /// Clipboard image thumbnail.
    pub thumb: Option<Pixmap>,
    /// Clipboard colour swatch.
    pub color: Option<(u8, u8, u8)>,
    /// Battery level 0..=1 for power peeks.
    pub level: Option<f32>,
}

impl PeekInfo {
    pub fn new(kind: PeekKind, line1: String, line2: String) -> Self {
        Self { kind, line1, line2, thumb: None, color: None, level: None }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimerMode {
    #[default]
    Countdown,
    Stopwatch,
    Reminders,
}

/// What the Timer tab looks like right now (computed by the app from `timer::{Timer,Stopwatch}`
/// and the reminder list).
#[derive(Clone, Debug, Default)]
pub struct TimerView {
    pub mode: TimerMode,
    pub running: bool,
    pub finished: bool,
    pub secs: u64,
    /// 1.0 = full, 0.0 = done. Always 1.0 in Stopwatch mode (nothing to count down from).
    pub frac: f32,
    pub total_min: u32,
    /// Remaining seconds of each pending reminder, soonest first.
    pub reminders: Vec<u64>,
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
    /// Settings scroll position in rows.
    pub sscroll: f32,
    pub autostart: bool,
    /// 1 -> 0 fade of the "Saved" note after a settings change.
    pub saved: f32,
    pub timer: TimerView,
    /// (row index, 0..1 intensity) of the row that was just copied.
    pub flash: Option<(usize, f32)>,
    /// Displayed (spring-smoothed) CPU / RAM / battery fractions.
    pub gauges: [f32; 3],
    pub stats: &'a Stats,
    pub accent: (u8, u8, u8),
    pub peek: Option<&'a PeekInfo>,
    /// Current clipboard search text (empty = no filter).
    pub search: &'a str,
    /// The search box has keyboard focus (used only for the cursor-blink affordance).
    pub search_active: bool,
    /// Draw the tiny always-click-through game bar instead of the normal pill.
    pub game_bar: bool,
}

fn white(a: f32) -> Color {
    rgba(WHITE, a)
}

/// Paints one frame. Returns true when something needs continuous redraws (scrolling title).
pub fn draw(cv: &mut Canvas, sc: &Scene) -> bool {
    cv.begin();
    if sc.game_bar {
        draw_game_bar(cv, sc);
        return false;
    }
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

    if ca > 0.01 {
        let cy = p.cy();
        // Running timer: a small ring and the remaining time. It sits on the right next to the
        // equalizer, or in the middle when nothing is playing.
        let mut eq_right = p.right() - 14.0;
        if sc.timer.running {
            let txt = fmt_clock(sc.timer.secs);
            let tw = cv.text_width(&txt, 12.0, Weight::Semibold);
            let (ring_cx, text_x) = if show_media {
                let tx = p.right() - 12.0 - tw;
                eq_right = tx - 18.0 - 12.0;
                (tx - 11.0, tx)
            } else {
                let gw = 12.0 + 6.0 + tw;
                let x0 = p.cx() - gw * 0.5;
                (x0 + 6.0, x0 + 18.0)
            };
            draw_ring(cv, ring_cx, cy, 5.5, 2.2, sc.timer.frac, sc.accent, ca);
            cv.text(&txt, text_x, cy + 4.3, 12.0, Weight::Semibold, WHITE, ca);
        }
        if show_media {
            if let Some(img) = &sc.media.art {
                cv.draw_image(img, Rect::new(p.x + 11.0, cy - 10.0, 20.0, 20.0), 6.0, ca);
            }
            draw_eq(cv, eq_right, cy, m.playing, sc.now, sc.accent, ca);
        }
    }
    if pa > 0.01 {
        if let Some(pk) = sc.peek {
            draw_peek(cv, sc, pk, pa);
        }
    }
}

/// Draw a pre-sized image inside `area`, keeping its aspect ratio.
fn draw_thumb(cv: &mut Canvas, px: &Pixmap, area: Rect, radius: f32, a: f32) {
    let (iw, ih) = (px.width() as f32, px.height() as f32);
    let s = (area.w / iw).min(area.h / ih);
    let (w, h) = (iw * s, ih * s);
    let r = Rect::new(area.cx() - w * 0.5, area.cy() - h * 0.5, w, h);
    cv.draw_image(px, r, radius.min(w * 0.5).min(h * 0.5), a);
}

/// Circular progress: a faint full track plus an arc of `frac` of the circle from 12 o'clock.
fn draw_ring(cv: &mut Canvas, cx: f32, cy: f32, r: f32, width: f32, frac: f32, color: (u8, u8, u8), a: f32) {
    if let Some(track) = gfx::circle(cx, cy, r) {
        cv.stroke(&track, width, white(0.14 * a), false);
    }
    let f = frac.clamp(0.0, 1.0);
    if f > 0.004 {
        if let Some(arc) = gfx::arc_path(cx, cy, r, -FRAC_PI_2, TAU * f.min(0.9999)) {
            cv.stroke(&arc, width, rgba(color, a), true);
        }
    }
}

fn draw_peek(cv: &mut Canvas, sc: &Scene, pk: &PeekInfo, a: f32) {
    let p = sc.pill;
    let cy = p.cy();
    let mut x = p.x + 10.0;
    let cell = Rect::new(x, cy - 15.0, 30.0, 30.0);
    match pk.kind {
        PeekKind::Track => match &sc.media.art {
            Some(img) => cv.draw_image(img, cell, 8.0, a),
            None => {
                cv.fill_rrect(cell, 8.0, white(0.10 * a));
                draw_note(cv, cell.cx(), cell.cy(), 0.55, 0.40 * a);
            }
        },
        PeekKind::Clip => {
            if let Some(px) = &pk.thumb {
                cv.fill_rrect(cell, 8.0, white(0.08 * a));
                draw_thumb(cv, px, cell, 6.0, a);
            } else if let Some(c) = pk.color {
                cv.fill_rrect(cell.inflate(0.8), 9.0, white(0.18 * a));
                cv.fill_rrect(cell, 8.0, rgba(c, a));
            } else {
                draw_clip_icon(cv, cell.cx(), cy, a, sc.accent);
            }
        }
        PeekKind::Timer => {
            draw_ring(cv, cell.cx(), cy, 11.0, 3.0, 1.0, sc.accent, a);
            cv.fill_circle(cell.cx(), cy, 3.2, white(a));
        }
        PeekKind::Power(icon) => draw_power_icon(cv, cell.cx(), cy, icon, pk.level.unwrap_or(1.0), a),
        PeekKind::Notification => {
            cv.fill_rrect(cell, 8.0, white(0.10 * a));
            draw_bell(cv, cell.cx(), cell.cy(), 0.55, 0.85 * a);
        }
    }
    x += 40.0;
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

/// Battery outline with a level bar; a bolt for "plugged in".
fn draw_power_icon(cv: &mut Canvas, cx: f32, cy: f32, icon: PowerIcon, level: f32, a: f32) {
    let pct = (level.clamp(0.0, 1.0) * 100.0).round() as u8;
    let charging = matches!(icon, PowerIcon::Plugged | PowerIcon::Full);
    let col = battery_color(pct, charging);
    let body = Rect::new(cx - 13.0, cy - 7.0, 24.0, 14.0);
    if let Some(path) = gfx::rrect(body, 4.0) {
        cv.stroke(&path, 1.5, white(0.75 * a), false);
    }
    cv.fill_rrect(Rect::new(body.right() + 1.5, cy - 2.5, 2.2, 5.0), 1.0, white(0.6 * a));
    let fill_w = ((body.w - 5.0) * level.clamp(0.0, 1.0)).max(2.0);
    cv.fill_rrect(Rect::new(body.x + 2.5, body.y + 2.5, fill_w, body.h - 5.0), 2.0, rgba(col, a));
    if charging {
        let s = 5.5;
        tri(
            cv,
            &[
                (cx - 1.0 + 0.4 * s, cy - 1.0 * s),
                (cx - 1.0 - 0.6 * s, cy + 0.15 * s),
                (cx - 1.0 - 0.05 * s, cy + 0.15 * s),
                (cx - 1.0 - 0.35 * s, cy + 1.0 * s),
                (cx - 1.0 + 0.65 * s, cy - 0.25 * s),
                (cx - 1.0 + 0.1 * s, cy - 0.25 * s),
            ],
            rgba((0, 0, 0), 0.85 * a),
        );
    }
}

/// A small notification bell: body + clapper, for the notification peek.
fn draw_bell(cv: &mut Canvas, cx: f32, cy: f32, scale: f32, a: f32) {
    let col = white(a);
    if let Some(p) = gfx::arc_path(cx, cy - 1.5 * scale, 7.0 * scale, std::f32::consts::PI, std::f32::consts::PI) {
        cv.stroke(&p, 1.8 * scale, col, false);
    }
    cv.fill_rrect(Rect::new(cx - 7.5 * scale, cy - 1.5 * scale, 15.0 * scale, 2.0 * scale), 1.0 * scale, col);
    cv.fill_circle(cx, cy + 3.3 * scale, 1.8 * scale, col);
}

/// The whole overlay in "game bar" mode: a tiny, always-click-through strip with no hover or
/// expand state at all, just three mini readouts. Never animated (there is nothing to spring
/// towards - it is either on screen like this, or not on screen).
fn draw_game_bar(cv: &mut Canvas, sc: &Scene) {
    let r = game_bar_rect();
    if let Some(path) = gfx::rrect(r, r.h * 0.5) {
        cv.fill(&path, rgba((0, 0, 0), 0.82));
        cv.stroke(&path, 1.0, white(0.10), false);
        cv.set_clip(path);
    }
    let st = sc.stats;
    let cols: [(&str, f32, (u8, u8, u8)); 3] = [
        ("C", st.cpu / 100.0, load_color((90, 200, 250), st.cpu)),
        ("G", st.gpu_pct.unwrap_or(0.0) / 100.0, load_color((255, 159, 10), st.gpu_pct.unwrap_or(0.0))),
        ("R", st.ram_pct / 100.0, load_color((191, 90, 242), st.ram_pct)),
    ];
    let col_w = r.w / 3.0;
    for (i, (label, frac, color)) in cols.iter().enumerate() {
        let cx = r.x + col_w * (i as f32 + 0.5);
        draw_ring(cv, cx - 10.0, r.cy(), 6.5, 2.2, *frac, *color, 1.0);
        let pct = (frac * 100.0).round() as i32;
        cv.text(&format!("{label}{pct}"), cx - 1.0, r.cy() + 4.0, 10.5, Weight::Semibold, WHITE, 0.9);
    }
    cv.unclip_rect();
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
    let last = (TAB_COUNT - 1) as f32;

    // Sliding tab highlight + labels.
    let step = g.tabs[1].x - g.tabs[0].x;
    let hx = g.tabs[0].x + sc.tab_pos.clamp(0.0, last) * step;
    cv.fill_rrect(Rect::new(hx, g.tabs[0].y, g.tabs[0].w, g.tabs[0].h), 12.0, white(0.13 * a));
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let r = g.tabs[i];
        let act = 1.0 - (sc.tab_pos - i as f32).abs().min(1.0);
        let hov = if matches!(sc.hover, Hit::Tab(j) if j == i) { 1.0 } else { 0.0 };
        let alpha = (0.45 + 0.55 * act + 0.25 * hov * (1.0 - act)).min(1.0) * a;
        cv.text_centered(tab.short(), r.cx(), r.y + 16.0, 11.5, Weight::Semibold, WHITE, alpha);
    }

    draw_pin(cv, g.pin, sc.pinned, sc.hover == Hit::Pin, sc.accent, a);

    // Left slot of the header: "Clear" on the clipboard tab, the mode name on the Timer tab
    // (click to cycle Countdown / Stopwatch / Reminders), "Saved" after a settings change.
    let clip_vis = 1.0 - (sc.tab_pos - 1.0).abs().min(1.0);
    if clip_vis > 0.01 && !sc.clip.is_empty() {
        let al = if sc.hover == Hit::Clear { 0.9 } else { 0.5 };
        cv.text("Clear", g.clear.x + 4.0, g.clear.y + 16.0, 11.5, Weight::Semibold, WHITE, al * clip_vis * a);
    }
    let timer_vis = 1.0 - (sc.tab_pos - 3.0).abs().min(1.0);
    if timer_vis > 0.01 {
        let label = match sc.timer.mode {
            TimerMode::Countdown => "Timer",
            TimerMode::Stopwatch => "Stopwatch",
            TimerMode::Reminders => "Reminders",
        };
        let al = if sc.hover == Hit::Clear { 0.9 } else { 0.55 };
        cv.text(label, g.clear.x + 4.0, g.clear.y + 16.0, 11.5, Weight::Semibold, sc.accent, al * timer_vis * a);
    }
    let set_vis = 1.0 - (sc.tab_pos - 4.0).abs().min(1.0);
    if set_vis > 0.01 && sc.saved > 0.01 {
        cv.text("Saved", g.clear.x + 4.0, g.clear.y + 16.0, 11.5, Weight::Semibold, sc.accent, sc.saved * set_vis * a);
    }

    // Pages: neighbours slide and cross-fade while the highlight moves.
    let mut marquee = false;
    for i in 0..TAB_COUNT {
        let d = i as f32 - sc.tab_pos;
        if d.abs() >= 1.0 {
            continue;
        }
        let pa = a * (1.0 - d.abs());
        let ox = d * 40.0;
        match i {
            0 => marquee = page_media(cv, sc, ox, pa),
            1 => page_clip(cv, sc, ox, pa),
            2 => page_system(cv, sc, ox, pa),
            3 => page_timer(cv, sc, ox, pa),
            _ => page_settings(cv, sc, ox, pa),
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
/// Leading icon of a clipboard row, inside `cell` (30 x 20).
fn draw_kind_icon(cv: &mut Canvas, e: &crate::history::Entry, cell: Rect, a: f32) {
    let (cx, cy) = (cell.cx(), cell.cy());
    match e.kind {
        Kind::Image => {
            cv.fill_rrect(cell, 5.0, white(0.07 * a));
            if let Some(img) = &e.image {
                if let Some(px) = pixmap_from_rgba(img.thumb.clone(), img.thumb_w, img.thumb_h) {
                    draw_thumb(cv, &px, cell, 4.0, a);
                }
            }
        }
        Kind::Color(c) => {
            let sw = Rect::new(cx - 10.0, cy - 10.0, 20.0, 20.0);
            cv.fill_rrect(sw.inflate(0.8), 6.5, white(0.20 * a));
            cv.fill_rrect(sw, 6.0, rgba(c, a));
        }
        Kind::Link(_) => {
            // Globe: outline, equator and meridian.
            let col = rgba((120, 190, 255), 0.95 * a);
            if let Some(c) = gfx::circle(cx, cy, 7.0) {
                cv.stroke(&c, 1.4, col, false);
            }
            cv.fill_rrect(Rect::new(cx - 7.0, cy - 0.6, 14.0, 1.2), 0.6, col);
            cv.fill_rrect(Rect::new(cx - 0.6, cy - 7.0, 1.2, 14.0), 0.6, col);
            if let Some(c) = gfx::circle(cx, cy, 3.2) {
                cv.stroke(&c, 1.0, col, false);
            }
        }
        Kind::Files(_) => {
            let col = rgba((255, 204, 90), 0.95 * a);
            cv.fill_rrect(Rect::new(cx - 9.0, cy - 6.5, 7.0, 4.0), 1.5, col);
            cv.fill_rrect(Rect::new(cx - 9.0, cy - 4.5, 18.0, 11.5), 2.5, col);
        }
        Kind::Text => {
            for (k, w) in [12.0f32, 9.0, 11.0].iter().enumerate() {
                cv.fill_rrect(Rect::new(cx - 6.0, cy - 5.5 + k as f32 * 4.4, *w, 1.6), 0.8, white(0.45 * a));
            }
        }
    }
}

fn page_clip(cv: &mut Canvas, sc: &Scene, ox: f32, a: f32) {
    let g = sc.geo;
    let c = g.content;

    if sc.clip.is_empty() {
        let cx = c.cx() + ox;
        draw_clip_icon(cv, cx, c.y + 34.0, 0.30 * a, sc.accent);
        cv.text_centered("Nothing copied yet", cx, c.y + 72.0, 14.0, Weight::Semibold, WHITE, 0.85 * a);
        cv.text_centered("Text, links, colours, files and images show up here", cx, c.y + 90.0, 11.5, Weight::Regular, WHITE, 0.45 * a);
        return;
    }

    draw_search_box(cv, sc, ox, a);

    let filtered = sc.clip.filter(sc.search);
    let items = sc.clip.items();
    if filtered.is_empty() {
        let cx = c.cx() + ox;
        let cy = c.y + CLIP_LIST_OFFSET + 30.0;
        cv.text_centered("No matches", cx, cy, 13.0, Weight::Semibold, WHITE, 0.6 * a);
        return;
    }

    let list_y = c.y + CLIP_LIST_OFFSET;
    cv.clip_rect(Rect::new(c.x - 8.0, list_y, c.w + 16.0, c.h - CLIP_LIST_OFFSET));
    let first = sc.scroll.floor().max(0.0) as usize;
    let last = (first + CLIP_VISIBLE_ROWS + 1).min(filtered.len());
    for pos in first..last {
        let abs = filtered[pos];
        let e = &items[abs];
        let y = list_y + (pos as f32 - sc.scroll) * ROW_H;
        let row = Rect::new(c.x + ox, y + 2.0, c.w, ROW_H - 4.0);
        let hov = matches!(sc.hover, Hit::Row(j) if j == pos);
        if hov {
            cv.fill_rrect(row, 9.0, white(0.08 * a));
        }
        let flash = match sc.flash {
            Some((fi, v)) if fi == abs => v,
            _ => 0.0,
        };
        if flash > 0.0 {
            cv.fill_rrect(row, 9.0, rgba(sc.accent, 0.28 * flash * a));
        }
        if e.pinned {
            cv.fill_circle(row.x + 6.0, row.cy(), 2.3, rgba(sc.accent, a));
        }
        draw_kind_icon(cv, e, Rect::new(row.x + 12.0, row.y + 2.0, 30.0, row.h - 4.0), a);

        let tx = row.x + 12.0 + 30.0 + 8.0;
        let avail = row.w - (tx - row.x) - 64.0;
        let ty = row.y + 16.5;
        match e.kind {
            Kind::Link(hl) => {
                // Host in full strength, path and query dimmed.
                let hl = if e.preview.is_char_boundary(hl) { hl } else { e.preview.len() };
                let (host, rest) = e.preview.split_at(hl);
                let host = cv.fit(host, 12.5, Weight::Semibold, avail);
                let hw = cv.text_width(&host, 12.5, Weight::Semibold);
                cv.text(&host, tx, ty, 12.5, Weight::Semibold, WHITE, 0.92 * a);
                if avail - hw > 12.0 && !rest.is_empty() {
                    let rest = cv.fit(rest, 12.5, Weight::Regular, avail - hw);
                    cv.text(&rest, tx + hw, ty, 12.5, Weight::Regular, WHITE, 0.45 * a);
                }
            }
            Kind::Color((r, gr, b)) => {
                let hex = format!("#{r:02X}{gr:02X}{b:02X}");
                let hw = cv.text_width(&hex, 12.5, Weight::Semibold);
                cv.text(&hex, tx, ty, 12.5, Weight::Semibold, WHITE, 0.92 * a);
                let rgb = format!("rgb({r}, {gr}, {b})");
                cv.text(&rgb, tx + hw + 10.0, ty, 11.5, Weight::Regular, WHITE, 0.45 * a);
            }
            _ => {
                let txt = cv.fit(&e.preview, 12.5, Weight::Regular, avail);
                cv.text(&txt, tx, ty, 12.5, Weight::Regular, WHITE, 0.88 * a);
            }
        }

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

    scroll_thumb(cv, Rect::new(c.x, list_y, c.w, c.h - CLIP_LIST_OFFSET), filtered.len(), CLIP_VISIBLE_ROWS, sc.scroll, ox, a);
}

/// Search box above the clipboard list: placeholder text, typed text, and a blinking caret
/// while it has keyboard focus.
fn draw_search_box(cv: &mut Canvas, sc: &Scene, ox: f32, a: f32) {
    let r = Rect::new(sc.geo.search.x + ox, sc.geo.search.y, sc.geo.search.w, sc.geo.search.h);
    let focused = sc.search_active;
    cv.fill_rrect(r, 10.0, white((if focused { 0.14 } else { 0.08 }) * a));
    if focused {
        if let Some(p) = gfx::rrect(r.inflate(0.5), 10.5) {
            cv.stroke(&p, 1.0, rgba(sc.accent, 0.8 * a), false);
        }
    }
    draw_magnifier(cv, r.x + 14.0, r.cy(), a);
    let tx = r.x + 26.0;
    if sc.search.is_empty() {
        cv.text("Search", tx, r.cy() + 4.0, 12.0, Weight::Regular, WHITE, 0.35 * a);
    } else {
        let w = cv.fit(sc.search, 12.5, Weight::Regular, r.w - 60.0);
        cv.text(&w, tx, r.cy() + 4.0, 12.5, Weight::Regular, WHITE, 0.9 * a);
        if focused && (sc.now * 2.0).fract() < 0.5 {
            let cx = tx + cv.text_width(&w, 12.5, Weight::Regular) + 1.5;
            cv.fill_rrect(Rect::new(cx, r.cy() - 7.0, 1.3, 14.0), 0.6, white(0.8 * a));
        }
        cv.text_centered("Esc", r.right() - 20.0, r.cy() + 3.5, 9.5, Weight::Semibold, WHITE, 0.35 * a);
    }
}

/// Small magnifying-glass icon.
fn draw_magnifier(cv: &mut Canvas, cx: f32, cy: f32, a: f32) {
    let col = white(0.5 * a);
    if let Some(p) = gfx::circle(cx - 1.0, cy - 1.0, 4.0) {
        cv.stroke(&p, 1.4, col, false);
    }
    tri(cv, &[(cx + 2.6, cy + 2.6), (cx + 4.4, cy + 4.4), (cx + 3.5, cy + 5.3), (cx + 1.7, cy + 3.5)], col);
}

/// Thin scroll indicator to the right of a list page.
fn scroll_thumb(cv: &mut Canvas, c: Rect, total: usize, visible: usize, scroll: f32, ox: f32, a: f32) {
    if total > visible {
        let max_scroll = (total - visible) as f32;
        let th = (c.h * visible as f32 / total as f32).max(16.0);
        let ty = c.y + (c.h - th) * (scroll / max_scroll).clamp(0.0, 1.0);
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
    let (gpu_center, gpu_sub, gpu_col) = match st.gpu_pct {
        Some(p) => (format!("{p:.0}%"), String::new(), load_color((255, 159, 10), p)),
        None => ("\u{2014}".to_string(), "n/a".to_string(), (120, 120, 128)),
    };
    let cols: [(&str, String, String, (u8, u8, u8), f32); 4] = [
        ("CPU", format!("{:.0}%", st.cpu), format!("{} threads", st.threads), load_color((90, 200, 250), st.cpu), sc.gauges[0]),
        ("GPU", gpu_center, gpu_sub, gpu_col, st.gpu_pct.unwrap_or(0.0) / 100.0),
        (
            "Memory",
            format!("{:.0}%", st.ram_pct),
            format!("{:.1}/{:.0} GB", st.ram_used_gb, st.ram_total_gb),
            load_color((191, 90, 242), st.ram_pct),
            sc.gauges[1],
        ),
        ("Battery", batt_center, batt_sub, batt_col, sc.gauges[2]),
    ];

    for (i, (label, center, sub, color, frac)) in cols.iter().enumerate() {
        let cx = g.content.x + g.col_w * (i as f32 + 0.5) + ox;
        if let Some(track) = gfx::circle(cx, cy, r) {
            cv.stroke(&track, 5.0, white(0.10 * a), false);
        }
        let f = frac.clamp(0.0, 1.0);
        if f > 0.004 {
            if let Some(arc) = gfx::arc_path(cx, cy, r, -FRAC_PI_2, TAU * f.min(0.9999)) {
                cv.stroke(&arc, 5.0, rgba(*color, a), true);
            }
        }
        cv.text_centered(center, cx, cy + 4.5, 12.5, Weight::Semibold, WHITE, a);
        cv.text_centered(label, cx, cy + r + 18.0, 10.5, Weight::Semibold, WHITE, 0.9 * a);
        if !sub.is_empty() {
            cv.text_centered(sub, cx, cy + r + 31.0, 9.0, Weight::Regular, WHITE, 0.42 * a);
        }
    }

    // Network speed, centred under the rings.
    let cx = g.content.cx() + ox;
    let line = format!("\u{2193} {}   \u{2191} {}", fmt_rate(st.net_down_bps), fmt_rate(st.net_up_bps));
    cv.text_centered(&line, cx, g.net_y, 11.0, Weight::Regular, WHITE, 0.55 * a);
}

// -------------------------------------------------------------------- timer
fn pill_button(cv: &mut Canvas, r: Rect, label: &str, hover: bool, primary: bool, a: f32) {
    if primary {
        cv.fill_rrect(r, r.h * 0.5, white((if hover { 1.0 } else { 0.92 }) * a));
        cv.text_centered(label, r.cx(), r.y + r.h * 0.5 + 4.5, 12.5, Weight::Semibold, (0, 0, 0), a);
    } else {
        cv.fill_rrect(r, r.h * 0.5, white((if hover { 0.18 } else { 0.09 }) * a));
        cv.text_centered(label, r.cx(), r.y + r.h * 0.5 + 4.5, 12.0, Weight::Semibold, WHITE, 0.9 * a);
    }
}

fn page_timer(cv: &mut Canvas, sc: &Scene, ox: f32, a: f32) {
    use crate::timer::REMIND_PRESETS;
    let g = sc.geo;
    let t = &sc.timer;
    let ring = g.t_ring;
    let (cx, cy) = (ring.cx() + ox, ring.cy());
    let off = |r: Rect| Rect::new(r.x + ox, r.y, r.w, r.h);

    match t.mode {
        TimerMode::Reminders => {
            // The ring area becomes a short list of pending reminders instead of a clock.
            if t.reminders.is_empty() {
                draw_bell(cv, cx, cy - 6.0, 1.3, 0.35 * a);
                cv.text_centered("No reminders set", cx, cy + 24.0, 12.0, Weight::Regular, WHITE, 0.5 * a);
            } else {
                for (i, secs) in t.reminders.iter().take(3).enumerate() {
                    let y = ring.y + 14.0 + i as f32 * 20.0;
                    cv.text_centered(&format!("Reminder \u{b7} {}", fmt_clock(*secs)), cx, y, 12.5, Weight::Semibold, WHITE, a);
                }
                if t.reminders.len() > 3 {
                    cv.text_centered(&format!("+{} more", t.reminders.len() - 3), cx, ring.y + 14.0 + 60.0, 10.0, Weight::Regular, WHITE, 0.45 * a);
                }
            }
        }
        TimerMode::Stopwatch => {
            draw_ring(cv, cx, cy, 40.0, 7.0, 1.0, sc.accent, if t.running { a } else { 0.35 * a });
            let clock = fmt_clock(t.secs);
            let size = if t.secs >= 3600 { 16.0 } else { 21.0 };
            cv.text_centered(&clock, cx, cy + 4.0, size, Weight::Semibold, WHITE, a);
            cv.text_centered(if t.running { "Running" } else { "Paused" }, cx, cy + 22.0, 10.5, Weight::Regular, WHITE, 0.5 * a);
        }
        TimerMode::Countdown => {
            let color = if t.finished { (255, 159, 10) } else { sc.accent };
            if sc.hover == Hit::TimerToggle {
                cv.fill_circle(cx, cy, 33.0, white(0.05 * a));
            }
            draw_ring(cv, cx, cy, 40.0, 7.0, if t.finished { 1.0 } else { t.frac }, color, a);
            let clock = fmt_clock(t.secs);
            let size = if t.secs >= 3600 { 16.0 } else { 21.0 };
            cv.text_centered(&clock, cx, cy + 4.0, size, Weight::Semibold, WHITE, a);
            let state = if t.finished {
                "Time is up"
            } else if t.running {
                "Running"
            } else if t.frac < 0.999 {
                "Paused"
            } else {
                "Ready"
            };
            cv.text_centered(state, cx, cy + 22.0, 10.5, Weight::Regular, WHITE, 0.5 * a);
        }
    }

    // Preset chips: minutes to count down to (Countdown) or to add as a reminder (Reminders).
    if t.mode != TimerMode::Stopwatch {
        let n = if t.mode == TimerMode::Reminders { REMIND_PRESETS.len() } else { TIMER_PRESETS.len() };
        for i in 0..n {
            let r = off(g.t_presets[i]);
            let hov = sc.hover == Hit::Preset(i);
            let sel = t.mode == TimerMode::Countdown && t.total_min == TIMER_PRESETS[i];
            if sel {
                cv.fill_rrect(r, 13.0, rgba(sc.accent, 0.25 * a));
            } else {
                cv.fill_rrect(r, 13.0, white((if hov { 0.16 } else { 0.08 }) * a));
            }
            let al = if sel || hov { 1.0 } else { 0.75 };
            let mins = if t.mode == TimerMode::Reminders { REMIND_PRESETS[i] } else { TIMER_PRESETS[i] };
            let label = if t.mode == TimerMode::Reminders { format!("+{mins}m") } else { format!("{mins}m") };
            cv.text_centered(&label, r.cx(), r.y + 17.5, 12.0, Weight::Semibold, WHITE, al * a);
        }
    }

    // Transport row: Countdown gets the full set; Stopwatch only start/reset; Reminders only
    // a "clear all" (via the reset button) once something is pending.
    match t.mode {
        TimerMode::Countdown => {
            pill_button(cv, off(g.t_minus), "-1m", sc.hover == Hit::TimerMinus, false, a);
            let label = if t.finished {
                "Restart"
            } else if t.running {
                "Pause"
            } else if t.frac < 0.999 {
                "Resume"
            } else {
                "Start"
            };
            pill_button(cv, off(g.t_toggle), label, sc.hover == Hit::TimerToggle, true, a);
            pill_button(cv, off(g.t_plus), "+1m", sc.hover == Hit::TimerPlus, false, a);
            pill_button(cv, off(g.t_reset), "Reset", sc.hover == Hit::TimerReset, false, a);
            cv.text("Scroll to change the time", g.t_presets[0].x + ox, g.content.y + 104.0, 10.5, Weight::Regular, WHITE, 0.32 * a);
        }
        TimerMode::Stopwatch => {
            let label = if t.running { "Pause" } else { "Start" };
            pill_button(cv, off(g.t_toggle), label, sc.hover == Hit::TimerToggle, true, a);
            pill_button(cv, off(g.t_reset), "Reset", sc.hover == Hit::TimerReset, false, a);
        }
        TimerMode::Reminders => {
            if !t.reminders.is_empty() {
                pill_button(cv, off(g.t_reset), "Clear all", sc.hover == Hit::TimerReset, false, a);
            }
        }
    }
}

// ----------------------------------------------------------------- settings// ----------------------------------------------------------------- settings
fn page_settings(cv: &mut Canvas, sc: &Scene, ox: f32, a: f32) {
    let g = sc.geo;
    let c = g.content;
    let view = settings::View { cfg: sc.cfg, autostart: sc.autostart };
    let right = settings_ctl_right(c) + ox;

    cv.clip_rect(Rect::new(c.x - 8.0, c.y, c.w + 16.0, c.h));
    let first = sc.sscroll.floor().max(0.0) as usize;
    let last = (first + VISIBLE_ROWS + 1).min(settings::ROWS.len());
    for i in first..last {
        let row = &settings::ROWS[i];
        let y = c.y + (i as f32 - sc.sscroll) * ROW_H;
        let hov_row = matches!(sc.hover, Hit::Set(j, _) if j == i);
        if hov_row && row.ctl == Ctl::Toggle {
            cv.fill_rrect(Rect::new(c.x + ox, y + 2.0, c.w, ROW_H - 4.0), 9.0, white(0.06 * a));
        }
        cv.text(row.label, c.x + ox + 8.0, y + 18.5, 12.5, Weight::Regular, WHITE, 0.88 * a);

        let (rects, n) = settings::sub_rects(row.ctl, right, y);
        let hov_sub = |k: usize| matches!(sc.hover, Hit::Set(j, s) if j == i && s as usize == k);
        match row.ctl {
            Ctl::Toggle => {
                let on = settings::toggle_on(row.id, &view);
                let r = rects[0];
                let track = Rect::new(r.x + 1.0, r.y + 1.0, 34.0, 18.0);
                if on {
                    cv.fill_rrect(track, 9.0, rgba(sc.accent, a));
                } else {
                    cv.fill_rrect(track, 9.0, white((if hov_row { 0.24 } else { 0.16 }) * a));
                }
                let kx = if on { track.right() - 9.0 } else { track.x + 9.0 };
                cv.fill_circle(kx, track.cy(), 7.0, white(a));
            }
            Ctl::Seg(labels) => {
                let sel = settings::seg_selected(row.id, sc.cfg);
                let whole = Rect::new(rects[0].x, rects[0].y, rects[n - 1].right() - rects[0].x, rects[0].h);
                cv.fill_rrect(whole, 10.0, white(0.08 * a));
                for k in 0..n {
                    let r = rects[k];
                    let is_sel = sel == Some(k);
                    if is_sel {
                        cv.fill_rrect(r.inflate(-2.0), 8.0, white(0.20 * a));
                    }
                    let al = if is_sel { 1.0 } else if hov_sub(k) { 0.85 } else { 0.5 };
                    cv.text_centered(labels[k], r.cx(), r.y + 14.5, 11.5, Weight::Semibold, WHITE, al * a);
                }
            }
            Ctl::Swatch => {
                let sel = settings::swatch_selected(sc.cfg);
                for k in 0..n {
                    let r = rects[k];
                    let (cx, cy) = (r.cx(), r.cy());
                    if k == 0 {
                        if let Some(ring) = gfx::circle(cx, cy, 5.5) {
                            cv.stroke(&ring, 1.4, white(0.6 * a), false);
                        }
                        cv.text_centered("A", cx, cy + 3.2, 8.5, Weight::Semibold, WHITE, 0.7 * a);
                    } else {
                        let col = settings::ACCENTS[k - 1];
                        cv.fill_circle(cx, cy, if hov_sub(k) { 7.2 } else { 6.5 }, rgba(col, a));
                    }
                    if sel == Some(k) {
                        if let Some(ring) = gfx::circle(cx, cy, 9.0) {
                            cv.stroke(&ring, 1.5, white(0.9 * a), false);
                        }
                    }
                }
            }
            Ctl::Step => {
                for k in 0..2 {
                    let r = rects[k];
                    cv.fill_rrect(r, 10.0, white((if hov_sub(k) { 0.20 } else { 0.10 }) * a));
                    cv.fill_rrect(Rect::new(r.cx() - 4.0, r.cy() - 0.8, 8.0, 1.6), 0.8, white(a));
                    if k == 1 {
                        cv.fill_rrect(Rect::new(r.cx() - 0.8, r.cy() - 4.0, 1.6, 8.0), 0.8, white(a));
                    }
                }
                let mid = (rects[0].right() + rects[1].x) * 0.5;
                cv.text_centered(&settings::step_text(row.id, sc.cfg), mid, rects[0].y + 14.5, 12.0, Weight::Semibold, WHITE, a);
            }
            Ctl::Buttons(labels) => {
                for k in 0..n {
                    let r = rects[k];
                    cv.fill_rrect(r, 10.0, white((if hov_sub(k) { 0.20 } else { 0.10 }) * a));
                    cv.text_centered(labels[k], r.cx(), r.y + 14.5, 11.5, Weight::Semibold, WHITE, 0.9 * a);
                }
            }
        }
    }
    cv.unclip_rect();
    scroll_thumb(cv, c, settings::ROWS.len(), VISIBLE_ROWS, sc.sscroll, ox, a);
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
