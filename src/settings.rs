//! The Settings tab as data: the six categories, which rows each one holds, what control each row
//! has, where the control sits and what a click on it does to the `Config`. No OS calls and no
//! drawing, so the whole thing is unit-tested; `ui.rs` paints it and `layout.rs` hit-tests it
//! from the same numbers.
//!
//! The tab has two levels: a home page with one card per category, and a category page that
//! shows that category's rows (a list, or for Peeks a grid of switches).

use crate::config::{
    gb, sev, AlarmRepeat, AlarmTone, Config, Dnd, ExpandOn, FullscreenMode, SoundPack,
};
use crate::layout::{settings_ctl_right, Rect, Style, ROW_H, VISIBLE_ROWS};

/// Accent swatches offered in the UI (sub-control 0 is "auto", 1.. map to these).
pub const ACCENTS: [(u8, u8, u8); 6] =
    [(122, 168, 255), (48, 209, 88), (255, 159, 10), (255, 95, 160), (191, 90, 242), (255, 69, 58)];

pub const FPS_STEPS: [u32; 3] = [30, 60, 120];

/// Game-bar readouts behind the two chip rows, in chip order.
pub const GB_FRAMES: [u16; 4] = [gb::FPS, gb::LOW1, gb::LOW01, gb::FRAMETIME];
pub const GB_SYSTEM: [u16; 5] = [gb::CPU, gb::GPU, gb::RAM, gb::PING, gb::NET];

/// Event-sound chips, in chip order (same order as `sound::EVENT_SOUNDS`).
pub const EVENT_BITS: [u8; 4] = [sev::NOTIFY, sev::POWER, sev::COPY, sev::BLUETOOTH];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    ExpandOn,
    Style,
    Accent,
    AlbumArt,
    Equalizer,
    AutoPeek,
    PowerAlerts,
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
    VolumePeek,
    BrightnessPeek,
    LockPeek,
    BtPeek,
    PrivacyDot,
    Dnd,
    SoundsOn,
    SoundVolume,
    SoundPack,
    UiSounds,
    EventSounds,
    AlarmTone,
    AlarmRepeat,
    SoundFiles,
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

pub const ROWS: [Row; 34] = [
    Row { id: Id::ExpandOn, label: "Expand on", ctl: Ctl::Seg(&["Hover", "Click"]) },
    Row { id: Id::Style, label: "Style", ctl: Ctl::Seg(&["Notch", "Island"]) },
    Row { id: Id::Accent, label: "Accent colour", ctl: Ctl::Swatch },
    Row { id: Id::AlbumArt, label: "Album art", ctl: Ctl::Toggle },
    Row { id: Id::Equalizer, label: "Equalizer bars", ctl: Ctl::Toggle },
    Row { id: Id::AutoPeek, label: "Peek on track / copy", ctl: Ctl::Toggle },
    Row { id: Id::PowerAlerts, label: "Battery & charger alerts", ctl: Ctl::Toggle },
    Row { id: Id::ClipImages, label: "Clipboard images", ctl: Ctl::Toggle },
    Row { id: Id::Notifications, label: "Notification peeks", ctl: Ctl::Toggle },
    Row { id: Id::VolumePeek, label: "Volume peek", ctl: Ctl::Toggle },
    Row { id: Id::BrightnessPeek, label: "Brightness peek", ctl: Ctl::Toggle },
    Row { id: Id::LockPeek, label: "Caps / Num Lock peek", ctl: Ctl::Toggle },
    Row { id: Id::BtPeek, label: "Bluetooth peek", ctl: Ctl::Toggle },
    Row { id: Id::PrivacyDot, label: "Mic & camera indicator", ctl: Ctl::Toggle },
    Row { id: Id::Dnd, label: "Do not disturb", ctl: Ctl::Seg(&["Off", "On", "Games"]) },
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
    Row { id: Id::SoundsOn, label: "Sounds", ctl: Ctl::Toggle },
    Row { id: Id::SoundVolume, label: "Volume", ctl: Ctl::Step },
    Row { id: Id::SoundPack, label: "Sound pack", ctl: Ctl::Seg(&["Soft", "Glass", "Retro"]) },
    Row { id: Id::UiSounds, label: "UI sounds", ctl: Ctl::Toggle },
    Row { id: Id::EventSounds, label: "Event sounds", ctl: Ctl::Chips(&["Notify", "Power", "Copy", "BT"]) },
    Row { id: Id::AlarmTone, label: "Timer alarm", ctl: Ctl::Seg(&["Off", "Chime", "Beep", "Bell"]) },
    Row { id: Id::AlarmRepeat, label: "Alarm repeat", ctl: Ctl::Seg(&["Once", "3\u{d7}", "Loop"]) },
    Row { id: Id::SoundFiles, label: "Own sounds (.wav)", ctl: Ctl::Buttons(&["Folder", "Reload"]) },
];

/// The row definition for an id.
pub fn row(id: Id) -> &'static Row {
    ROWS.iter().find(|r| r.id == id).unwrap_or(&ROWS[0])
}

// ------------------------------------------------------------------------------- categories

/// The six categories on the Settings home page, in display order (3 x 2 cards).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cat {
    Look,
    General,
    Peeks,
    Sounds,
    Focus,
    Data,
}

impl Cat {
    pub const ALL: [Cat; 6] = [Cat::Look, Cat::General, Cat::Peeks, Cat::Sounds, Cat::Focus, Cat::Data];

    pub fn from_index(i: usize) -> Cat {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn title(self) -> &'static str {
        match self {
            Cat::Look => "Look & feel",
            Cat::General => "General",
            Cat::Peeks => "Peeks",
            Cat::Sounds => "Sounds",
            Cat::Focus => "Focus & games",
            Cat::Data => "Data",
        }
    }

    /// Peeks are all switches: they are drawn as a compact grid instead of a list.
    pub fn is_grid(self) -> bool {
        self == Cat::Peeks
    }
}

/// Short label for a switch in the Peeks grid.
pub fn grid_label(id: Id) -> &'static str {
    match id {
        Id::AutoPeek => "Track / copy",
        Id::PowerAlerts => "Battery",
        Id::Notifications => "Notifications",
        Id::VolumePeek => "Volume",
        Id::BrightnessPeek => "Brightness",
        Id::LockPeek => "Caps / Num Lock",
        Id::BtPeek => "Bluetooth",
        Id::PrivacyDot => "Mic & camera",
        _ => row(id).label,
    }
}

pub const MAX_PAGE_ROWS: usize = 8;

/// The rows currently shown on a category page (some depend on other settings).
#[derive(Clone, Copy, Debug)]
pub struct RowList {
    ids: [Id; MAX_PAGE_ROWS],
    n: usize,
}

impl RowList {
    fn from(list: &[Id]) -> RowList {
        let mut ids = [Id::ExpandOn; MAX_PAGE_ROWS];
        let n = list.len().min(MAX_PAGE_ROWS);
        ids[..n].copy_from_slice(&list[..n]);
        RowList { ids, n }
    }
    pub fn as_slice(&self) -> &[Id] {
        &self.ids[..self.n]
    }
    pub fn len(&self) -> usize {
        self.n
    }
}

/// Rows of a category. Sounds shows only the master switch while sounds are off, and Focus shows
/// the game-bar chips only while "Full screen behaviour" is "Game bar".
pub fn rows_of(cat: Cat, c: &Config) -> RowList {
    match cat {
        Cat::Look => RowList::from(&[Id::Style, Id::Accent, Id::AlbumArt, Id::Equalizer]),
        Cat::General => RowList::from(&[Id::ExpandOn, Id::HoverDelay, Id::Fps, Id::Autostart, Id::ConfigFile]),
        Cat::Peeks => RowList::from(&[
            Id::AutoPeek,
            Id::PowerAlerts,
            Id::Notifications,
            Id::VolumePeek,
            Id::BrightnessPeek,
            Id::LockPeek,
            Id::BtPeek,
            Id::PrivacyDot,
        ]),
        Cat::Sounds => {
            if c.sounds {
                RowList::from(&[
                    Id::SoundsOn,
                    Id::SoundVolume,
                    Id::SoundPack,
                    Id::UiSounds,
                    Id::EventSounds,
                    Id::AlarmTone,
                    Id::AlarmRepeat,
                    Id::SoundFiles,
                ])
            } else {
                RowList::from(&[Id::SoundsOn])
            }
        }
        Cat::Focus => {
            if c.fullscreen_mode == FullscreenMode::GameBar {
                RowList::from(&[Id::Dnd, Id::HideFullscreen, Id::FullscreenMode, Id::GameFrames, Id::GameSystem])
            } else {
                RowList::from(&[Id::Dnd, Id::HideFullscreen, Id::FullscreenMode])
            }
        }
        Cat::Data => RowList::from(&[Id::ClipImages, Id::Privacy, Id::HistorySize, Id::NetSpeed]),
    }
}

/// One-line live summary on a category card.
pub fn summary(cat: Cat, c: &Config) -> String {
    match cat {
        Cat::Look => format!(
            "{} \u{b7} {}",
            if c.style == Style::Island { "Island" } else { "Notch" },
            if c.accent.is_some() { "custom" } else { "auto" }
        ),
        Cat::General => format!(
            "{} \u{b7} {} fps",
            if c.expand_on == ExpandOn::Click { "Click" } else { "Hover" },
            c.fps_cap
        ),
        Cat::Peeks => {
            let on = rows_of(Cat::Peeks, c).as_slice().iter().filter(|&&id| toggle_cfg(id, c)).count();
            format!("{on} of {} on", rows_of(Cat::Peeks, c).len())
        }
        Cat::Sounds => {
            if c.sounds {
                let pack = match c.sound_pack {
                    SoundPack::Soft => "Soft",
                    SoundPack::Glass => "Glass",
                    SoundPack::Retro => "Retro",
                };
                format!("{pack} \u{b7} {}%", c.sound_volume)
            } else {
                "Off".to_string()
            }
        }
        Cat::Focus => format!(
            "DND {} \u{b7} {}",
            match c.dnd {
                Dnd::Off => "off",
                Dnd::On => "on",
                Dnd::Games => "games",
            },
            if c.fullscreen_mode == FullscreenMode::GameBar { "bar" } else { "hide" }
        ),
        Cat::Data => format!("{} clips \u{b7} {}", c.history_size, if c.clip_images { "images" } else { "text" }),
    }
}

/// What the Settings tab currently shows: the home page (`cat` = None) or a category.
#[derive(Clone, Copy, Debug)]
pub struct Page {
    pub cat: Option<Cat>,
    pub rows: RowList,
}

impl Page {
    pub fn new(cat: Option<Cat>, c: &Config) -> Page {
        Page { cat, rows: cat.map_or(RowList::from(&[]), |k| rows_of(k, c)) }
    }

    pub fn is_grid(&self) -> bool {
        self.cat.is_some_and(Cat::is_grid)
    }

    /// How far a list page can scroll, in rows (home page and grid never scroll).
    pub fn max_scroll(&self) -> f32 {
        match self.cat {
            Some(k) if !k.is_grid() => self.rows.len().saturating_sub(VISIBLE_ROWS) as f32,
            _ => 0.0,
        }
    }
}

// ------------------------------------------------------------------------------------ geometry

pub const CARD_W: f32 = 132.0;
pub const CARD_H: f32 = 52.0;
const CARD_GAP_X: f32 = 10.0;
const CARD_GAP_Y: f32 = 8.0;

/// Category card `i` (3 per row) on the home page.
pub fn card_rect(content: Rect, i: usize) -> Rect {
    let (col, line) = ((i % 3) as f32, (i / 3) as f32);
    Rect::new(content.x + col * (CARD_W + CARD_GAP_X), content.y + line * (CARD_H + CARD_GAP_Y), CARD_W, CARD_H)
}

pub const CHIP_W_GRID: f32 = 134.0;
pub const CHIP_H_GRID: f32 = 34.0;
const GRID_GAP_X: f32 = 7.0;
const GRID_GAP_Y: f32 = 5.0;

/// Switch `i` (3 per row) of the Peeks grid.
pub fn grid_rect(content: Rect, i: usize) -> Rect {
    let (col, line) = ((i % 3) as f32, (i / 3) as f32);
    Rect::new(
        content.x + col * (CHIP_W_GRID + GRID_GAP_X),
        content.y + line * (CHIP_H_GRID + GRID_GAP_Y),
        CHIP_W_GRID,
        CHIP_H_GRID,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageHit {
    /// A category card on the home page.
    Cat(usize),
    /// (position in `Page::rows`, sub-control).
    Row(usize, usize),
}

/// What is under (x, y) on the Settings page. `scroll` is the list scroll in rows.
pub fn page_hit(page: &Page, content: Rect, scroll: f32, x: f32, y: f32) -> Option<PageHit> {
    if !content.contains(x, y) {
        return None;
    }
    match page.cat {
        None => (0..Cat::ALL.len()).find(|&i| card_rect(content, i).contains(x, y)).map(PageHit::Cat),
        Some(k) if k.is_grid() => {
            (0..page.rows.len()).find(|&i| grid_rect(content, i).contains(x, y)).map(|i| PageHit::Row(i, 0))
        }
        Some(_) => {
            let idx = (scroll + (y - content.y) / ROW_H).floor();
            if idx < 0.0 {
                return None;
            }
            let i = idx as usize;
            let id = *page.rows.as_slice().get(i)?;
            let row_y = content.y + (i as f32 - scroll) * ROW_H;
            hit(row(id), settings_ctl_right(content), row_y, x, y).map(|s| PageHit::Row(i, s))
        }
    }
}

const CTL_H: f32 = 20.0;
const CHIP_W: f32 = 42.0;

fn seg_w(labels: &[&str]) -> f32 {
    if labels.iter().all(|l| l.chars().count() <= 3) {
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

// --------------------------------------------------------------------------------------- state

/// Read-only state the rows need besides the config.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    pub cfg: &'a Config,
    pub autostart: bool,
}

fn toggle_cfg(id: Id, c: &Config) -> bool {
    match id {
        Id::AlbumArt => c.album_art,
        Id::Equalizer => c.equalizer,
        Id::AutoPeek => c.auto_peek,
        Id::PowerAlerts => c.power_alerts,
        Id::ClipImages => c.clip_images,
        Id::Privacy => c.privacy_filter,
        Id::HideFullscreen => c.hide_fullscreen,
        Id::Notifications => c.notifications,
        Id::NetSpeed => c.net_speed,
        Id::VolumePeek => c.volume_peek,
        Id::BrightnessPeek => c.brightness_peek,
        Id::LockPeek => c.lock_peek,
        Id::BtPeek => c.bt_peek,
        Id::PrivacyDot => c.privacy_dot,
        Id::SoundsOn => c.sounds,
        Id::UiSounds => c.ui_sounds,
        _ => false,
    }
}

pub fn toggle_on(id: Id, v: &View) -> bool {
    if id == Id::Autostart {
        v.autostart
    } else {
        toggle_cfg(id, v.cfg)
    }
}

/// Selected option of a segmented control.
pub fn seg_selected(id: Id, c: &Config) -> Option<usize> {
    match id {
        Id::ExpandOn => Some((c.expand_on == ExpandOn::Click) as usize),
        Id::Style => Some((c.style == Style::Island) as usize),
        Id::Fps => FPS_STEPS.iter().position(|&f| f == c.fps_cap),
        Id::FullscreenMode => Some((c.fullscreen_mode == FullscreenMode::GameBar) as usize),
        Id::Dnd => Some(match c.dnd {
            Dnd::Off => 0,
            Dnd::On => 1,
            Dnd::Games => 2,
        }),
        Id::SoundPack => Some(match c.sound_pack {
            SoundPack::Soft => 0,
            SoundPack::Glass => 1,
            SoundPack::Retro => 2,
        }),
        Id::AlarmTone => Some(match c.alarm_tone {
            AlarmTone::Off => 0,
            AlarmTone::Chime => 1,
            AlarmTone::Beep => 2,
            AlarmTone::Bell => 3,
        }),
        Id::AlarmRepeat => Some(match c.alarm_repeat {
            AlarmRepeat::Once => 0,
            AlarmRepeat::Three => 1,
            AlarmRepeat::Loop => 2,
        }),
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
    if id == Id::EventSounds {
        return EVENT_BITS.get(k).is_some_and(|b| c.sound_events & b != 0);
    }
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
        Id::SoundVolume => format!("{}%", c.sound_volume),
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
    /// Open the folder for your own `.wav` files (created on demand).
    OpenSounds,
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
        Id::ClipImages => c.clip_images = !c.clip_images,
        Id::Notifications => c.notifications = !c.notifications,
        Id::NetSpeed => c.net_speed = !c.net_speed,
        Id::VolumePeek => c.volume_peek = !c.volume_peek,
        Id::BrightnessPeek => c.brightness_peek = !c.brightness_peek,
        Id::LockPeek => c.lock_peek = !c.lock_peek,
        Id::BtPeek => c.bt_peek = !c.bt_peek,
        Id::PrivacyDot => c.privacy_dot = !c.privacy_dot,
        Id::Dnd => {
            c.dnd = match sub {
                1 => Dnd::On,
                2 => Dnd::Games,
                _ => Dnd::Off,
            }
        }
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
        Id::SoundsOn => c.sounds = !c.sounds,
        Id::SoundVolume => {
            let d = if sub == 0 { -1 } else { 1 };
            c.sound_volume = step(c.sound_volume as i64, d, 10, 0, 100) as u32;
        }
        Id::SoundPack => {
            c.sound_pack = match sub {
                1 => SoundPack::Glass,
                2 => SoundPack::Retro,
                _ => SoundPack::Soft,
            }
        }
        Id::UiSounds => c.ui_sounds = !c.ui_sounds,
        Id::EventSounds => {
            if let Some(b) = EVENT_BITS.get(sub) {
                c.sound_events ^= b;
            }
        }
        Id::AlarmTone => {
            c.alarm_tone = match sub {
                1 => AlarmTone::Chime,
                2 => AlarmTone::Beep,
                3 => AlarmTone::Bell,
                _ => AlarmTone::Off,
            }
        }
        Id::AlarmRepeat => {
            c.alarm_repeat = match sub {
                0 => AlarmRepeat::Once,
                2 => AlarmRepeat::Loop,
                _ => AlarmRepeat::Three,
            }
        }
        Id::Autostart => return Action::Autostart(!autostart),
        Id::ConfigFile => return if sub == 0 { Action::OpenConfig } else { Action::Reload },
        Id::SoundFiles => return if sub == 0 { Action::OpenSounds } else { Action::Reload },
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
    use crate::layout::{geo, Style as LStyle};

    fn all_on() -> Config {
        Config { sounds: true, fullscreen_mode: FullscreenMode::GameBar, ..Config::default() }
    }

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
    fn row_ids_are_unique_and_found() {
        for (i, a) in ROWS.iter().enumerate() {
            assert_eq!(row(a.id).id, a.id);
            for b in ROWS.iter().skip(i + 1) {
                assert_ne!(a.id, b.id);
            }
        }
    }

    #[test]
    fn every_row_lives_in_exactly_one_category() {
        let c = all_on();
        let mut seen: Vec<Id> = Vec::new();
        for cat in Cat::ALL {
            let list = rows_of(cat, &c);
            assert!(list.len() >= 1 && list.len() <= MAX_PAGE_ROWS, "{cat:?}");
            for &id in list.as_slice() {
                assert!(!seen.contains(&id), "{id:?} appears twice");
                seen.push(id);
            }
        }
        for r in ROWS.iter() {
            assert!(seen.contains(&r.id), "{:?} is in no category", r.id);
        }
        assert_eq!(seen.len(), ROWS.len());
    }

    #[test]
    fn pages_stay_small_and_hidden_rows_follow_their_switch() {
        let mut c = Config::default();
        assert_eq!(rows_of(Cat::Focus, &c).len(), 3); // game-bar chips hidden
        c.fullscreen_mode = FullscreenMode::GameBar;
        assert_eq!(rows_of(Cat::Focus, &c).len(), 5);
        assert_eq!(rows_of(Cat::Sounds, &c).len(), 8);
        c.sounds = false;
        assert_eq!(rows_of(Cat::Sounds, &c).as_slice(), &[Id::SoundsOn]);
        // Only Focus (game bar on), General and Sounds scroll; Look and Data fit exactly.
        assert_eq!(Page::new(Some(Cat::Look), &c).max_scroll(), 0.0);
        assert_eq!(Page::new(Some(Cat::Data), &c).max_scroll(), 0.0);
        assert_eq!(Page::new(Some(Cat::General), &c).max_scroll(), 1.0);
        assert_eq!(Page::new(Some(Cat::Peeks), &c).max_scroll(), 0.0);
        assert_eq!(Page::new(None, &c).max_scroll(), 0.0);
        c.sounds = true;
        assert_eq!(Page::new(Some(Cat::Sounds), &c).max_scroll(), 4.0);
    }

    #[test]
    fn peeks_are_all_switches() {
        let c = Config::default();
        for &id in rows_of(Cat::Peeks, &c).as_slice() {
            assert_eq!(row(id).ctl, Ctl::Toggle, "{id:?}");
            assert!(!grid_label(id).is_empty());
        }
        assert!(rows_of(Cat::Peeks, &c).len() <= 9);
    }

    #[test]
    fn cards_and_grid_fit_the_content_area_without_overlap() {
        let content = geo(LStyle::Notch).content;
        let rects: Vec<Rect> = (0..6).map(|i| card_rect(content, i)).collect();
        let grid: Vec<Rect> = (0..9).map(|i| grid_rect(content, i)).collect();
        for set in [&rects, &grid] {
            for (i, r) in set.iter().enumerate() {
                assert!(r.x >= content.x - 0.01 && r.right() <= content.right() + 0.01, "{i}");
                assert!(r.y >= content.y - 0.01 && r.bottom() <= content.bottom() + 0.01, "{i}");
                for o in set.iter().skip(i + 1) {
                    let apart = r.right() <= o.x || o.right() <= r.x || r.bottom() <= o.y || o.bottom() <= r.y;
                    assert!(apart, "cells overlap");
                }
            }
        }
    }

    #[test]
    fn home_page_hits_cards() {
        let g = geo(LStyle::Notch);
        let page = Page::new(None, &Config::default());
        for i in 0..6 {
            let r = card_rect(g.content, i);
            assert_eq!(page_hit(&page, g.content, 0.0, r.cx(), r.cy()), Some(PageHit::Cat(i)));
        }
        // the gap between two cards is nothing
        let a = card_rect(g.content, 0);
        assert_eq!(page_hit(&page, g.content, 0.0, a.right() + 4.0, a.cy()), None);
        assert_eq!(page_hit(&page, g.content, 0.0, 1.0, 1.0), None);
    }

    #[test]
    fn grid_page_hits_switches() {
        let g = geo(LStyle::Island);
        let c = Config::default();
        let page = Page::new(Some(Cat::Peeks), &c);
        for i in 0..page.rows.len() {
            let r = grid_rect(g.content, i);
            assert_eq!(page_hit(&page, g.content, 0.0, r.cx(), r.cy()), Some(PageHit::Row(i, 0)));
        }
        // the 9th cell is empty
        let r = grid_rect(g.content, 8);
        assert_eq!(page_hit(&page, g.content, 0.0, r.cx(), r.cy()), None);
    }

    #[test]
    fn list_page_hits_controls_and_follows_scroll() {
        let g = geo(LStyle::Notch);
        let c = all_on();
        let page = Page::new(Some(Cat::Sounds), &c);
        let right = settings_ctl_right(g.content);
        for (i, &id) in page.rows.as_slice().iter().enumerate().take(4) {
            let y = g.content.y + i as f32 * ROW_H;
            let (rects, count) = sub_rects(row(id).ctl, right, y);
            for k in 0..count {
                let r = rects[k];
                assert_eq!(page_hit(&page, g.content, 0.0, r.cx(), r.cy()), Some(PageHit::Row(i, k)), "row {i} sub {k}");
            }
        }
        // Scrolled by two rows, the third visible row is row 4 (event chips).
        let (rects, _) = sub_rects(row(page.rows.as_slice()[4]).ctl, right, g.content.y + 2.0 * ROW_H);
        assert_eq!(page_hit(&page, g.content, 2.0, rects[1].cx(), rects[1].cy()), Some(PageHit::Row(4, 1)));
        // Below the last row is nothing.
        let small = Page::new(Some(Cat::Sounds), &Config { sounds: false, ..c });
        assert_eq!(page_hit(&small, g.content, 0.0, g.content.cx(), g.content.y + 2.0 * ROW_H), None);
    }

    #[test]
    fn summaries_fit_a_card() {
        let mut c = all_on();
        c.accent = Some((1, 2, 3));
        c.dnd = Dnd::Games;
        c.history_size = 100;
        for cat in Cat::ALL {
            let s = summary(cat, &c);
            assert!(!s.is_empty() && s.chars().count() <= 20, "{cat:?}: {s}");
        }
        assert_eq!(summary(Cat::Sounds, &Config { sounds: false, ..c.clone() }), "Off");
        assert_eq!(summary(Cat::Peeks, &Config::default()), format!("{} of 8 on", 7));
    }

    #[test]
    fn new_rows_toggle_and_dnd_cycles() {
        let mut c = Config::default();
        assert_eq!(apply(&mut c, false, Id::VolumePeek, 0), Action::Changed);
        assert!(!c.volume_peek);
        assert_eq!(apply(&mut c, false, Id::Dnd, 2), Action::Changed);
        assert_eq!(c.dnd, Dnd::Games);
        assert_eq!(seg_selected(Id::Dnd, &c), Some(2));
        assert_eq!(apply(&mut c, false, Id::Dnd, 2), Action::Nothing);
    }

    #[test]
    fn toggles_hit_anywhere_in_the_row() {
        let r = row(Id::AlbumArt);
        assert_eq!(hit(r, 400.0, 50.0, 10.0, 60.0), Some(0));
        assert_eq!(hit(r, 400.0, 50.0, 10.0, 49.0), None);
        assert_eq!(hit(&ROWS[0], 400.0, 50.0, 10.0, 60.0), None);
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
        assert_eq!(apply(&mut c, false, Id::SoundFiles, 0), Action::OpenSounds);
        assert_eq!(apply(&mut c, false, Id::SoundFiles, 1), Action::Reload);
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
    fn sound_rows_edit_the_sound_config() {
        let mut c = Config::default();
        assert!(toggle_on(Id::SoundsOn, &View { cfg: &c, autostart: false }));
        assert_eq!(apply(&mut c, false, Id::SoundVolume, 1), Action::Changed);
        assert_eq!(c.sound_volume, 70);
        assert_eq!(step_text(Id::SoundVolume, &c), "70%");
        c.sound_volume = 100;
        assert_eq!(apply(&mut c, false, Id::SoundVolume, 1), Action::Nothing);
        c.sound_volume = 0;
        assert_eq!(apply(&mut c, false, Id::SoundVolume, 0), Action::Nothing);

        assert_eq!(apply(&mut c, false, Id::SoundPack, 2), Action::Changed);
        assert_eq!(c.sound_pack, SoundPack::Retro);
        assert_eq!(seg_selected(Id::SoundPack, &c), Some(2));
        assert_eq!(apply(&mut c, false, Id::SoundPack, 2), Action::Nothing);

        assert_eq!(seg_selected(Id::AlarmTone, &c), Some(1)); // chime by default
        assert_eq!(apply(&mut c, false, Id::AlarmTone, 3), Action::Changed);
        assert_eq!(c.alarm_tone, AlarmTone::Bell);
        assert_eq!(apply(&mut c, false, Id::AlarmTone, 0), Action::Changed);
        assert_eq!(c.alarm_tone, AlarmTone::Off);

        assert_eq!(seg_selected(Id::AlarmRepeat, &c), Some(1)); // 3x by default
        apply(&mut c, false, Id::AlarmRepeat, 2);
        assert_eq!(c.alarm_repeat, AlarmRepeat::Loop);
        apply(&mut c, false, Id::AlarmRepeat, 0);
        assert_eq!(c.alarm_repeat, AlarmRepeat::Once);

        assert!(chip_on(Id::EventSounds, &c, 2));
        assert_eq!(apply(&mut c, false, Id::EventSounds, 2), Action::Changed);
        assert!(!chip_on(Id::EventSounds, &c, 2));
        assert_eq!(c.sound_events, sev::NOTIFY | sev::POWER | sev::BLUETOOTH);
        assert_eq!(apply(&mut c, false, Id::EventSounds, 9), Action::Nothing);

        assert_eq!(apply(&mut c, false, Id::UiSounds, 0), Action::Changed);
        assert!(!c.ui_sounds);
        assert_eq!(apply(&mut c, false, Id::SoundsOn, 0), Action::Changed);
        assert!(!c.sounds);
    }
}
