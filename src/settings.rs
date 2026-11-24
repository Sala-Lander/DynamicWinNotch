//! The Settings tab as data: which rows exist, what control each one has, where the control
//! sits and what a click on it does to the `Config`. No OS calls and no drawing, so the whole
//! thing is unit-tested; `ui.rs` paints it and `layout.rs` hit-tests it from the same numbers.

use crate::config::{gb, Config, ExpandOn, FullscreenMode};
use crate::layout::{Rect, Style, ROW_H};

/// Accent swatches offered in the UI (sub-control 0 is "auto", 1.. map to these).
pub const ACCENTS: [(u8, u8, u8); 6] =
    [(122, 168, 255), (48, 209, 88), (255, 159, 10), (255, 95, 160), (191, 90, 242), (255, 69, 58)];

pub const FPS_STEPS: [u32; 3] = [30, 60, 120];

/// Game-bar readouts behind the two chip rows, in chip order.
pub const GB_FRAMES: [u16; 4] = [gb::FPS, gb::LOW1, gb::LOW01, gb::FRAMETIME];
pub const GB_SYSTEM: [u16; 5] = [gb::CPU, gb::GPU, gb::RAM, gb::PING, gb::NET];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    ExpandOn,
    Style,
    Accent,
    AlbumArt,
    Equalizer,
    AutoPeek,
    PowerAlerts,
    TimerSound,
    Privacy,
    HideFullscreen,
    ClipImages,
    HistorySize,
    HoverDelay,
    Fps,
    Autostart,
    ConfigFile,
    Notifications,
    FullscreenMode,
    GameFrames,
    GameSystem,
    NetSpeed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctl {
    Toggle,
    /// Segmented control (exactly one option selected).
    Seg(&'static [&'static str]),
    /// "Auto" plus the six accent colours.
    Swatch,
    /// `-` value `+`.
    Step,
    /// Multi-select chips (any number on).
    Chips(&'static [&'static str]),
    /// Plain buttons (no selected state).
    Buttons(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub id: Id,
    pub label: &'static str,
    pub ctl: Ctl,
}

pub const ROWS: [Row; 21] = [
    Row { id: Id::ExpandOn, label: "Expand on", ctl: Ctl::Seg(&["Hover", "Click"]) },
    Row { id: Id::Style, label: "Style", ctl: Ctl::Seg(&["Notch", "Island"]) },
    Row { id: Id::Accent, label: "Accent colour", ctl: Ctl::Swatch },
    Row { id: Id::AlbumArt, label: "Album art", ctl: Ctl::Toggle },
    Row { id: Id::Equalizer, label: "Equalizer bars", ctl: Ctl::Toggle },
    Row { id: Id::AutoPeek, label: "Peek on track / copy", ctl: Ctl::Toggle },
    Row { id: Id::PowerAlerts, label: "Battery & charger alerts", ctl: Ctl::Toggle },
    Row { id: Id::TimerSound, label: "Timer sound", ctl: Ctl::Toggle },
    Row { id: Id::ClipImages, label: "Clipboard images", ctl: Ctl::Toggle },
    Row { id: Id::Notifications, label: "Notification peeks", ctl: Ctl::Toggle },
    Row { id: Id::Privacy, label: "Skip password managers", ctl: Ctl::Toggle },
    Row { id: Id::HistorySize, label: "Clipboard history", ctl: Ctl::Step },
    Row { id: Id::HideFullscreen, label: "Hide in full screen", ctl: Ctl::Toggle },
    Row { id: Id::FullscreenMode, label: "Full screen behaviour", ctl: Ctl::Seg(&["Hide", "Game bar"]) },
    Row { id: Id::GameFrames, label: "Game bar: frames", ctl: Ctl::Chips(&["FPS", "1%", "0.1%", "ms"]) },
    Row { id: Id::GameSystem, label: "Game bar: system", ctl: Ctl::Chips(&["CPU", "GPU", "RAM", "Ping", "Net"]) },
    Row { id: Id::HoverDelay, label: "Hover delay", ctl: Ctl::Step },
    Row { id: Id::Fps, label: "Frame rate", ctl: Ctl::Seg(&["30", "60", "120"]) },
    Row { id: Id::NetSpeed, label: "Live network speed", ctl: Ctl::Toggle },
    Row { id: Id::Autostart, label: "Start with Windows", ctl: Ctl::Toggle },
    Row { id: Id::ConfigFile, label: "Config file", ctl: Ctl::Buttons(&["Open", "Reload"]) },
];

const CTL_H: f32 = 20.0;
const CHIP_W: f32 = 42.0;

fn seg_w(labels: &[&str]) -> f32 {
    if labels.iter().all(|l| l.len() <= 3) {
        38.0
    } else {
        56.0
    }
}

/// Rectangles of a row's sub-controls, for controls whose right edge is `right` and whose row
/// starts at `y`. Returns the rectangles and how many are used.
pub fn sub_rects(ctl: Ctl, right: f32, y: f32) -> ([Rect; 8], usize) {
    let mut out = [Rect::default(); 8];
    let ty = y + (ROW_H - CTL_H) * 0.5;
    let n = match ctl {
        Ctl::Toggle => {
            out[0] = Rect::new(right - 36.0, ty, 36.0, CTL_H);
            1
        }
        Ctl::Seg(labels) => {
            let w = seg_w(labels);
            let x0 = right - w * labels.len() as f32;
            for i in 0..labels.len().min(8) {
                out[i] = Rect::new(x0 + i as f32 * w, ty, w, CTL_H);
            }
            labels.len().min(8)
        }
        Ctl::Chips(labels) => {
            let n = labels.len().min(8);
            let x0 = right - CHIP_W * n as f32;
            for i in 0..n {
                out[i] = Rect::new(x0 + i as f32 * CHIP_W, ty, CHIP_W, CTL_H);
            }
            n
        }
        Ctl::Swatch => {
            // Auto + six colours, 20 px pitch.
            let x0 = right - 7.0 * 20.0;
            for i in 0..7 {
                out[i] = Rect::new(x0 + i as f32 * 20.0, ty, 20.0, CTL_H);
            }
            7
        }
        Ctl::Step => {
            out[0] = Rect::new(right - 24.0 - 48.0 - 24.0, ty, 24.0, CTL_H);
            out[1] = Rect::new(right - 24.0, ty, 24.0, CTL_H);
            2
        }
        Ctl::Buttons(labels) => {
            let n = labels.len().min(8);
            let (w, gap) = (60.0, 6.0);
            let x0 = right - (w * n as f32 + gap * (n as f32 - 1.0));
            for i in 0..n {
                out[i] = Rect::new(x0 + i as f32 * (w + gap), ty, w, CTL_H);
            }
            n
        }
    };
    (out, n)
}

/// Which sub-control of `row` is under (x, y)? Toggles react on the whole row.
pub fn hit(row: &Row, right: f32, row_y: f32, x: f32, y: f32) -> Option<usize> {
    let (rects, n) = sub_rects(row.ctl, right, row_y);
    for (i, r) in rects.iter().take(n).enumerate() {
        if r.inflate_xy(1.0, 3.0).contains(x, y) {
            return Some(i);
        }
    }
    if row.ctl == Ctl::Toggle && y >= row_y && y < row_y + ROW_H {
        return Some(0);
    }
    None
}

/// Read-only state the rows need besides the config.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    pub cfg: &'a Config,
    pub autostart: bool,
}

pub fn toggle_on(id: Id, v: &View) -> bool {
    let c = v.cfg;
    match id {
        Id::AlbumArt => c.album_art,
        Id::Equalizer => c.equalizer,
        Id::AutoPeek => c.auto_peek,
        Id::PowerAlerts => c.power_alerts,
        Id::TimerSound => c.timer_sound,
        Id::ClipImages => c.clip_images,
        Id::Privacy => c.privacy_filter,
        Id::HideFullscreen => c.hide_fullscreen,
        Id::Notifications => c.notifications,
        Id::NetSpeed => c.net_speed,
        Id::Autostart => v.autostart,
        _ => false,
    }
}

/// Selected option of a segmented control.
pub fn seg_selected(id: Id, c: &Config) -> Option<usize> {
    match id {
        Id::ExpandOn => Some((c.expand_on == ExpandOn::Click) as usize),
        Id::Style => Some((c.style == Style::Island) as usize),
        Id::Fps => FPS_STEPS.iter().position(|&f| f == c.fps_cap),
        Id::FullscreenMode => Some((c.fullscreen_mode == FullscreenMode::GameBar) as usize),
        _ => None,
    }
}

fn chip_bits(id: Id) -> &'static [u16] {
    match id {
        Id::GameFrames => &GB_FRAMES,
        Id::GameSystem => &GB_SYSTEM,
        _ => &[],
    }
}

/// Is chip `k` of row `id` switched on?
pub fn chip_on(id: Id, c: &Config, k: usize) -> bool {
    chip_bits(id).get(k).is_some_and(|b| c.game_items & b != 0)
}

/// Selected swatch: 0 = auto, 1.. = `ACCENTS`, `None` = a custom colour from the file.
pub fn swatch_selected(c: &Config) -> Option<usize> {
    match c.accent {
        None => Some(0),
        Some(a) => ACCENTS.iter().position(|&x| x == a).map(|i| i + 1),
    }
}

pub fn step_text(id: Id, c: &Config) -> String {
    match id {
        Id::HistorySize => c.history_size.to_string(),
        Id::HoverDelay => format!("{} ms", c.hover_delay_ms),
        _ => String::new(),
    }
}

/// What the application has to do besides updating the config it passed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// The config changed: apply and save it.
    Changed,
    Autostart(bool),
    OpenConfig,
    Reload,
}

fn step(v: i64, dir: i64, by: i64, lo: i64, hi: i64) -> i64 {
    // Snap to the grid first, so a hand-edited 33 steps to 40, not 43.
    let snapped = if dir > 0 { (v / by + 1) * by } else { ((v + by - 1) / by - 1) * by };
    snapped.clamp(lo, hi)
}

/// Apply a click on `sub` of row `id` to `c`.
pub fn apply(c: &mut Config, autostart: bool, id: Id, sub: usize) -> Action {
    let before = c.clone();
    match id {
        Id::ExpandOn => c.expand_on = if sub == 1 { ExpandOn::Click } else { ExpandOn::Hover },
        Id::Style => c.style = if sub == 1 { Style::Island } else { Style::Notch },
        Id::Accent => c.accent = if sub == 0 { None } else { ACCENTS.get(sub - 1).copied().or(c.accent) },
        Id::AlbumArt => c.album_art = !c.album_art,
        Id::Equalizer => c.equalizer = !c.equalizer,
        Id::AutoPeek => c.auto_peek = !c.auto_peek,
        Id::PowerAlerts => c.power_alerts = !c.power_alerts,
        Id::TimerSound => c.timer_sound = !c.timer_sound,
        Id::ClipImages => c.clip_images = !c.clip_images,
        Id::Notifications => c.notifications = !c.notifications,
        Id::NetSpeed => c.net_speed = !c.net_speed,
        Id::GameFrames | Id::GameSystem => {
            if let Some(b) = chip_bits(id).get(sub) {
                c.game_items ^= b;
            }
        }
        Id::Privacy => c.privacy_filter = !c.privacy_filter,
        Id::HideFullscreen => c.hide_fullscreen = !c.hide_fullscreen,
        Id::FullscreenMode => c.fullscreen_mode = if sub == 1 { FullscreenMode::GameBar } else { FullscreenMode::Hide },
        Id::HistorySize => {
            let d = if sub == 0 { -1 } else { 1 };
            c.history_size = step(c.history_size as i64, d, 10, 10, 100) as usize;
        }
        Id::HoverDelay => {
            let d = if sub == 0 { -1 } else { 1 };
            c.hover_delay_ms = step(c.hover_delay_ms as i64, d, 20, 0, 400) as u32;
        }
        Id::Fps => {
            if let Some(&f) = FPS_STEPS.get(sub) {
                c.fps_cap = f;
            }
        }
        Id::Autostart => return Action::Autostart(!autostart),
        Id::ConfigFile => return if sub == 0 { Action::OpenConfig } else { Action::Reload },
    }
    if *c != before {
        Action::Changed
    } else {
        Action::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_rects_inside_the_row() {
        let right = 400.0;
        for row in ROWS.iter() {
            let (rects, n) = sub_rects(row.ctl, right, 100.0);
            assert!(n >= 1);
            for r in rects.iter().take(n) {
                assert!(r.right() <= right + 0.01, "{:?}", row.id);
                assert!(r.y >= 100.0 && r.bottom() <= 100.0 + ROW_H);
            }
            for w in rects[..n].windows(2) {
                assert!(w[0].right() <= w[1].x + 0.01, "{:?} overlaps", row.id);
            }
        }
    }

    #[test]
    fn labels_leave_room_for_controls() {
        // Row content is 416 wide; the widest control block must leave 150 px for the label.
        let right = 416.0 - 8.0;
        for row in ROWS.iter() {
            let (rects, n) = sub_rects(row.ctl, right, 0.0);
            assert!(rects[0].x > 150.0, "{:?} control starts at {}", row.id, rects[0].x);
            assert!(n <= 8);
        }
    }

    #[test]
    fn toggles_hit_anywhere_in_the_row() {
        let row = ROWS.iter().find(|r| r.id == Id::AlbumArt).unwrap();
        assert_eq!(hit(row, 400.0, 50.0, 10.0, 60.0), Some(0));
        assert_eq!(hit(row, 400.0, 50.0, 10.0, 49.0), None);
        let seg = &ROWS[0];
        assert_eq!(hit(seg, 400.0, 50.0, 10.0, 60.0), None);
    }

    #[test]
    fn segmented_and_swatch_selection() {
        let mut c = Config::default();
        assert_eq!(seg_selected(Id::ExpandOn, &c), Some(0));
        assert_eq!(apply(&mut c, false, Id::ExpandOn, 1), Action::Changed);
        assert_eq!(seg_selected(Id::ExpandOn, &c), Some(1));
        assert_eq!(apply(&mut c, false, Id::ExpandOn, 1), Action::Nothing); // already selected
        assert_eq!(apply(&mut c, false, Id::Style, 1), Action::Changed);
        assert_eq!(c.style, Style::Island);
        assert_eq!(apply(&mut c, false, Id::Fps, 2), Action::Changed);
        assert_eq!(c.fps_cap, 120);
        assert_eq!(seg_selected(Id::Fps, &c), Some(2));

        assert_eq!(swatch_selected(&c), Some(0));
        apply(&mut c, false, Id::Accent, 3);
        assert_eq!(c.accent, Some(ACCENTS[2]));
        assert_eq!(swatch_selected(&c), Some(3));
        apply(&mut c, false, Id::Accent, 0);
        assert_eq!(c.accent, None);
        c.accent = Some((1, 2, 3));
        assert_eq!(swatch_selected(&c), None);
    }

    #[test]
    fn toggles_flip_and_report() {
        let mut c = Config::default();
        assert!(toggle_on(Id::ClipImages, &View { cfg: &c, autostart: false }));
        assert_eq!(apply(&mut c, false, Id::ClipImages, 0), Action::Changed);
        assert!(!toggle_on(Id::ClipImages, &View { cfg: &c, autostart: false }));
        assert_eq!(apply(&mut c, false, Id::Autostart, 0), Action::Autostart(true));
        assert_eq!(apply(&mut c, true, Id::Autostart, 0), Action::Autostart(false));
        assert!(toggle_on(Id::Autostart, &View { cfg: &c, autostart: true }));
        assert_eq!(apply(&mut c, false, Id::ConfigFile, 0), Action::OpenConfig);
        assert_eq!(apply(&mut c, false, Id::ConfigFile, 1), Action::Reload);
    }

    #[test]
    fn steppers_snap_and_clamp() {
        let mut c = Config::default();
        c.history_size = 33;
        apply(&mut c, false, Id::HistorySize, 1);
        assert_eq!(c.history_size, 40);
        c.history_size = 33;
        apply(&mut c, false, Id::HistorySize, 0);
        assert_eq!(c.history_size, 30);
        c.history_size = 100;
        assert_eq!(apply(&mut c, false, Id::HistorySize, 1), Action::Nothing);
        c.history_size = 10;
        assert_eq!(apply(&mut c, false, Id::HistorySize, 0), Action::Nothing);
        c.hover_delay_ms = 0;
        assert_eq!(apply(&mut c, false, Id::HoverDelay, 0), Action::Nothing);
        apply(&mut c, false, Id::HoverDelay, 1);
        assert_eq!(c.hover_delay_ms, 20);
        assert_eq!(step_text(Id::HoverDelay, &c), "20 ms");
        c.history_size = 30;
        assert_eq!(step_text(Id::HistorySize, &c), "30");
    }

    #[test]
    fn notifications_and_fullscreen_mode() {
        let mut c = Config::default();
        assert!(!toggle_on(Id::Notifications, &View { cfg: &c, autostart: false }));
        assert_eq!(apply(&mut c, false, Id::Notifications, 0), Action::Changed);
        assert!(toggle_on(Id::Notifications, &View { cfg: &c, autostart: false }));

        assert_eq!(seg_selected(Id::FullscreenMode, &c), Some(0));
        assert_eq!(apply(&mut c, false, Id::FullscreenMode, 1), Action::Changed);
        assert_eq!(c.fullscreen_mode, FullscreenMode::GameBar);
        assert_eq!(seg_selected(Id::FullscreenMode, &c), Some(1));
        assert_eq!(apply(&mut c, false, Id::FullscreenMode, 1), Action::Nothing);
    }

    #[test]
    fn chips_flip_game_items() {
        let mut c = Config::default();
        assert!(chip_on(Id::GameFrames, &c, 0)); // FPS on by default
        assert!(!chip_on(Id::GameSystem, &c, 3)); // ping off by default
        assert_eq!(apply(&mut c, false, Id::GameSystem, 3), Action::Changed);
        assert!(c.game_items & gb::PING != 0);
        assert_eq!(apply(&mut c, false, Id::GameFrames, 0), Action::Changed);
        assert!(c.game_items & gb::FPS == 0);
        assert_eq!(apply(&mut c, false, Id::GameFrames, 9), Action::Nothing);
        assert!(toggle_on(Id::NetSpeed, &View { cfg: &c, autostart: false }));
        apply(&mut c, false, Id::NetSpeed, 0);
        assert!(!c.net_speed);
    }

    #[test]
    fn row_ids_are_unique() {
        for (i, a) in ROWS.iter().enumerate() {
            for b in ROWS.iter().skip(i + 1) {
                assert_ne!(a.id, b.id);
            }
        }
    }
}
