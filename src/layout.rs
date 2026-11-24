//! Geometry and hit-testing. Everything here is in *logical* pixels (480 x 200 window);
//! the renderer multiplies by the DPI scale. No OS calls, so it is unit-tested.

use crate::settings;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w * 0.5
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h * 0.5
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
    pub fn inflate(&self, d: f32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }
    pub fn inflate_xy(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x - dx, self.y - dy, self.w + 2.0 * dx, self.h + 2.0 * dy)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Media,
    Clipboard,
    System,
    Timer,
    Settings,
}

pub const TAB_COUNT: usize = 5;

impl Tab {
    pub const ALL: [Tab; TAB_COUNT] = [Tab::Media, Tab::Clipboard, Tab::System, Tab::Timer, Tab::Settings];

    pub fn index(self) -> usize {
        self as usize
    }
    pub fn from_index(i: usize) -> Tab {
        Self::ALL[i.min(TAB_COUNT - 1)]
    }
    pub fn label(self) -> &'static str {
        match self {
            Tab::Media => "Media",
            Tab::Clipboard => "Clipboard",
            Tab::System => "System",
            Tab::Timer => "Timer",
            Tab::Settings => "Settings",
        }
    }
    /// Label that fits the narrow tab buttons.
    pub fn short(self) -> &'static str {
        match self {
            Tab::Clipboard => "Clips",
            other => other.label(),
        }
    }
    pub fn parse(s: &str) -> Option<Tab> {
        match s.trim().to_ascii_lowercase().as_str() {
            "media" => Some(Tab::Media),
            "clipboard" | "clip" => Some(Tab::Clipboard),
            "system" | "sys" => Some(Tab::System),
            "timer" => Some(Tab::Timer),
            "settings" | "prefs" => Some(Tab::Settings),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Fused to the top screen edge with concave "ears", like a hardware notch.
    Notch,
    /// A free-floating pill, 8 px below the top edge.
    Island,
}

pub const WIN_W: f32 = 480.0;
pub const WIN_H: f32 = 200.0;

/// Pill sizes (w, h) for the three modes.
pub const COLLAPSED: (f32, f32) = (184.0, 32.0);
pub const PEEK: (f32, f32) = (300.0, 44.0);
pub const EXPANDED: (f32, f32) = (452.0, 172.0);

pub const ROW_H: f32 = 28.0;
pub const VISIBLE_ROWS: usize = 4;
/// Clipboard rows visible at once: one fewer than `VISIBLE_ROWS` to leave room for the search box.
pub const CLIP_VISIBLE_ROWS: usize = 3;
/// Height of the clipboard search box plus the gap before the first row.
pub const CLIP_LIST_OFFSET: f32 = 28.0;

/// Size of the tiny always-click-through bar shown instead of the pill in "game bar" mode.
pub const GAME_BAR: (f32, f32) = (150.0, 22.0);

/// The game bar, centred at the top edge (it never uses `Style`: it is flush with the edge
/// either way, since nothing should draw attention by floating).
pub fn game_bar_rect() -> Rect {
    Rect::new((WIN_W - GAME_BAR.0) * 0.5, 0.0, GAME_BAR.0, GAME_BAR.1)
}

pub fn margin_top(style: Style) -> f32 {
    match style {
        Style::Notch => 0.0,
        Style::Island => 8.0,
    }
}

/// A pill of size (w, h), centred horizontally in the window.
pub fn pill_rect(style: Style, w: f32, h: f32) -> Rect {
    Rect::new((WIN_W - w) * 0.5, margin_top(style), w, h)
}

/// Positions of every control in the fully expanded pill.
#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub pill: Rect,
    pub tabs: [Rect; TAB_COUNT],
    pub pin: Rect,
    pub clear: Rect,
    pub content: Rect,
    /// Clipboard search box; the row list starts below it.
    pub search: Rect,
    // media page
    pub art: Rect,
    pub text_x: f32,
    pub text_w: f32,
    pub title_y: f32,
    pub artist_y: f32,
    pub bar: Rect,
    pub time_y: f32,
    pub prev: Rect,
    pub play: Rect,
    pub next: Rect,
    // system page
    pub ring_y: f32,
    pub ring_r: f32,
    pub col_w: f32,
    /// Baseline of the network-speed text row under the four system rings.
    pub net_y: f32,
    // timer page
    pub t_ring: Rect,
    pub t_presets: [Rect; 5],
    pub t_minus: Rect,
    pub t_toggle: Rect,
    pub t_plus: Rect,
    pub t_reset: Rect,
}

/// Preset durations (minutes) of the timer chips.
pub const TIMER_PRESETS: [u32; 5] = [1, 5, 10, 25, 45];

/// Right edge of the controls in a settings row.
pub fn settings_ctl_right(content: Rect) -> f32 {
    content.right() - 8.0
}

pub fn geo(style: Style) -> Geo {
    let p = pill_rect(style, EXPANDED.0, EXPANDED.1);
    let (tab_w, gap) = (62.0, 3.0);
    let x0 = p.x + 80.0;
    let ty = p.y + 10.0;
    let mut tabs = [Rect::default(); TAB_COUNT];
    for (i, t) in tabs.iter_mut().enumerate() {
        *t = Rect::new(x0 + i as f32 * (tab_w + gap), ty, tab_w, 24.0);
    }
    let content = Rect::new(p.x + 18.0, p.y + 46.0, p.w - 36.0, p.h - 46.0 - 14.0);
    let search = Rect::new(content.x, content.y, content.w, 24.0);
    let art = Rect::new(content.x, content.y + 10.0, 92.0, 92.0);
    let text_x = art.right() + 16.0;
    let text_w = content.right() - text_x;
    let cx = text_x + text_w * 0.5;
    let cy = content.y + 88.0;

    // Timer page: ring on the left, preset chips and transport buttons on the right.
    let rx = content.x + 128.0;
    let mut t_presets = [Rect::default(); 5];
    for (i, r) in t_presets.iter_mut().enumerate() {
        *r = Rect::new(rx + i as f32 * 58.0, content.y + 8.0, 52.0, 26.0);
    }
    let by = content.y + 52.0;
    Geo {
        pill: p,
        tabs,
        pin: Rect::new(p.right() - 18.0 - 24.0, ty, 24.0, 24.0),
        clear: Rect::new(p.x + 18.0, ty, 56.0, 24.0),
        content,
        search,
        art,
        text_x,
        text_w,
        title_y: content.y + 20.0,
        artist_y: content.y + 38.0,
        bar: Rect::new(text_x, content.y + 48.0, text_w, 5.0),
        time_y: content.y + 66.0,
        prev: Rect::new(cx - 56.0 - 14.0, cy - 14.0, 28.0, 28.0),
        play: Rect::new(cx - 17.0, cy - 17.0, 34.0, 34.0),
        next: Rect::new(cx + 56.0 - 14.0, cy - 14.0, 28.0, 28.0),
        ring_y: content.y + 30.0,
        ring_r: 21.0,
        col_w: content.w / 4.0,
        net_y: content.y + content.h - 9.0,
        t_ring: Rect::new(content.x + 8.0, content.y + 8.0, 96.0, 96.0),
        t_presets,
        t_minus: Rect::new(rx, by, 34.0, 32.0),
        t_toggle: Rect::new(rx + 42.0, by, 104.0, 32.0),
        t_plus: Rect::new(rx + 154.0, by, 34.0, 32.0),
        t_reset: Rect::new(rx + 196.0, by, 62.0, 32.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    None,
    /// Inside the pill but not on a control.
    Body,
    Tab(usize),
    Pin,
    Clear,
    Prev,
    Play,
    Next,
    /// Progress bar; payload is the fraction 0..=1.
    Seek(f32),
    /// Clipboard row: a position in the *currently displayed* (filtered) list, not necessarily
    /// an absolute index into the history - the caller maps it back using the same filter.
    Row(usize),
    /// Settings control: (row index, sub-control index, see `settings::sub_rects`).
    Set(usize, u8),
    Preset(usize),
    TimerMinus,
    TimerToggle,
    TimerPlus,
    TimerReset,
    /// Clipboard search box.
    Search,
}

/// `scroll` is the clipboard scroll position in rows (fractional while animating).
pub fn hit_test(g: &Geo, tab: Tab, x: f32, y: f32, scroll: f32, rows: usize) -> Hit {
    if !g.pill.contains(x, y) {
        return Hit::None;
    }
    if g.pin.inflate(2.0).contains(x, y) {
        return Hit::Pin;
    }
    if matches!(tab, Tab::Clipboard | Tab::Timer) && g.clear.contains(x, y) {
        return Hit::Clear;
    }
    for (i, t) in g.tabs.iter().enumerate() {
        if t.inflate_xy(2.0, 3.0).contains(x, y) {
            return Hit::Tab(i);
        }
    }
    match tab {
        Tab::Media => {
            if g.play.inflate(3.0).contains(x, y) {
                return Hit::Play;
            }
            if g.prev.inflate(4.0).contains(x, y) {
                return Hit::Prev;
            }
            if g.next.inflate(4.0).contains(x, y) {
                return Hit::Next;
            }
            if g.bar.inflate_xy(0.0, 8.0).contains(x, y) {
                return Hit::Seek(((x - g.bar.x) / g.bar.w).clamp(0.0, 1.0));
            }
        }
        Tab::Clipboard => {
            if g.search.inflate(2.0).contains(x, y) {
                return Hit::Search;
            }
            let list_y = g.content.y + CLIP_LIST_OFFSET;
            if y >= list_y && x >= g.content.x && x < g.content.right() {
                let idx = (scroll + (y - list_y) / ROW_H).floor();
                if idx >= 0.0 && (idx as usize) < rows {
                    return Hit::Row(idx as usize);
                }
            }
        }
        Tab::Settings => {
            if g.content.contains(x, y) {
                let idx = (scroll + (y - g.content.y) / ROW_H).floor();
                if idx >= 0.0 && (idx as usize) < rows.min(settings::ROWS.len()) {
                    let i = idx as usize;
                    let row_y = g.content.y + (i as f32 - scroll) * ROW_H;
                    if let Some(sub) = settings::hit(&settings::ROWS[i], settings_ctl_right(g.content), row_y, x, y) {
                        return Hit::Set(i, sub as u8);
                    }
                }
            }
        }
        Tab::Timer => {
            for (i, r) in g.t_presets.iter().enumerate() {
                if r.contains(x, y) {
                    return Hit::Preset(i);
                }
            }
            if g.t_toggle.contains(x, y) || g.t_ring.contains(x, y) {
                return Hit::TimerToggle;
            }
            if g.t_minus.contains(x, y) {
                return Hit::TimerMinus;
            }
            if g.t_plus.contains(x, y) {
                return Hit::TimerPlus;
            }
            if g.t_reset.contains(x, y) {
                return Hit::TimerReset;
            }
        }
        Tab::System => {}
    }
    Hit::Body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_pill_fits_window_with_overshoot_room() {
        for style in [Style::Notch, Style::Island] {
            let g = geo(style);
            assert!(g.pill.x >= 10.0 && g.pill.right() <= WIN_W - 10.0);
            // 8% spring overshoot of the growth must still fit vertically.
            let grow = EXPANDED.1 - COLLAPSED.1;
            assert!(g.pill.y + EXPANDED.1 + 0.08 * grow <= WIN_H);
        }
    }

    #[test]
    fn tabs_hit() {
        let g = geo(Style::Notch);
        for i in 0..TAB_COUNT {
            let t = g.tabs[i];
            assert_eq!(hit_test(&g, Tab::Media, t.cx(), t.cy(), 0.0, 0), Hit::Tab(i));
        }
    }

    #[test]
    fn media_controls_hit() {
        let g = geo(Style::Island);
        assert_eq!(hit_test(&g, Tab::Media, g.play.cx(), g.play.cy(), 0.0, 0), Hit::Play);
        assert_eq!(hit_test(&g, Tab::Media, g.prev.cx(), g.prev.cy(), 0.0, 0), Hit::Prev);
        assert_eq!(hit_test(&g, Tab::Media, g.next.cx(), g.next.cy(), 0.0, 0), Hit::Next);
        match hit_test(&g, Tab::Media, g.bar.x + g.bar.w * 0.25, g.bar.cy(), 0.0, 0) {
            Hit::Seek(f) => assert!((f - 0.25).abs() < 1e-3),
            other => panic!("{other:?}"),
        }
        // Controls are not active on other tabs.
        assert_eq!(hit_test(&g, Tab::System, g.play.cx(), g.play.cy(), 0.0, 0), Hit::Body);
    }

    #[test]
    fn clipboard_rows_follow_scroll() {
        let g = geo(Style::Notch);
        let y0 = g.content.y + CLIP_LIST_OFFSET + 3.0;
        assert_eq!(hit_test(&g, Tab::Clipboard, g.content.cx(), y0, 0.0, 10), Hit::Row(0));
        assert_eq!(hit_test(&g, Tab::Clipboard, g.content.cx(), y0 + ROW_H, 0.0, 10), Hit::Row(1));
        assert_eq!(hit_test(&g, Tab::Clipboard, g.content.cx(), y0, 2.0, 10), Hit::Row(2));
        // Past the last item.
        assert_eq!(hit_test(&g, Tab::Clipboard, g.content.cx(), y0 + ROW_H * 3.0, 0.0, 2), Hit::Body);
        // The search box sits above the list.
        assert_eq!(hit_test(&g, Tab::Clipboard, g.search.cx(), g.search.cy(), 0.0, 10), Hit::Search);
        // Clear button doubles as the Timer mode toggle, but exists nowhere else.
        assert_eq!(hit_test(&g, Tab::Clipboard, g.clear.cx(), g.clear.cy(), 0.0, 3), Hit::Clear);
        assert_eq!(hit_test(&g, Tab::Timer, g.clear.cx(), g.clear.cy(), 0.0, 3), Hit::Clear);
        assert_eq!(hit_test(&g, Tab::Media, g.clear.cx(), g.clear.cy(), 0.0, 3), Hit::Body);
    }

    #[test]
    fn game_bar_is_centred_and_tiny() {
        let r = game_bar_rect();
        assert!((r.cx() - WIN_W * 0.5).abs() < 0.01);
        assert_eq!(r.y, 0.0);
        assert!(r.w < COLLAPSED.0 && r.h < COLLAPSED.1);
    }

    #[test]
    fn outside_is_none() {
        let g = geo(Style::Notch);
        assert_eq!(hit_test(&g, Tab::Media, 2.0, 2.0, 0.0, 0), Hit::None);
        assert_eq!(hit_test(&g, Tab::Media, 240.0, 199.0, 0.0, 0), Hit::None);
    }

    #[test]
    fn tab_parse() {
        assert_eq!(Tab::parse(" Clip "), Some(Tab::Clipboard));
        assert_eq!(Tab::parse("nope"), None);
        assert_eq!(Tab::from_index(9), Tab::Settings);
        assert_eq!(Tab::parse("timer"), Some(Tab::Timer));
        assert_eq!(Tab::parse("Settings"), Some(Tab::Settings));
        for (i, t) in Tab::ALL.iter().enumerate() {
            assert_eq!(t.index(), i);
            assert_eq!(Tab::from_index(i), *t);
        }
    }

    #[test]
    fn tab_strip_clear_and_pin_do_not_overlap() {
        for style in [Style::Notch, Style::Island] {
            let g = geo(style);
            assert!(g.clear.right() < g.tabs[0].x);
            assert!(g.tabs[TAB_COUNT - 1].right() < g.pin.inflate(2.0).x);
            for w in g.tabs.windows(2) {
                assert!(w[0].right() <= w[1].x);
            }
            // everything sits inside the pill
            assert!(g.tabs[0].x >= g.pill.x && g.pin.right() <= g.pill.right());
        }
    }

    #[test]
    fn timer_controls_hit_and_fit() {
        let g = geo(Style::Island);
        for (i, r) in g.t_presets.iter().enumerate() {
            assert_eq!(hit_test(&g, Tab::Timer, r.cx(), r.cy(), 0.0, 0), Hit::Preset(i));
            assert!(r.right() <= g.content.right());
        }
        assert_eq!(hit_test(&g, Tab::Timer, g.t_toggle.cx(), g.t_toggle.cy(), 0.0, 0), Hit::TimerToggle);
        assert_eq!(hit_test(&g, Tab::Timer, g.t_ring.cx(), g.t_ring.cy(), 0.0, 0), Hit::TimerToggle);
        assert_eq!(hit_test(&g, Tab::Timer, g.t_minus.cx(), g.t_minus.cy(), 0.0, 0), Hit::TimerMinus);
        assert_eq!(hit_test(&g, Tab::Timer, g.t_plus.cx(), g.t_plus.cy(), 0.0, 0), Hit::TimerPlus);
        assert_eq!(hit_test(&g, Tab::Timer, g.t_reset.cx(), g.t_reset.cy(), 0.0, 0), Hit::TimerReset);
        assert!(g.t_reset.right() <= g.content.right() && g.t_reset.bottom() <= g.content.bottom());
        // Timer controls do not exist on other tabs.
        assert_eq!(hit_test(&g, Tab::Media, g.t_reset.cx(), g.t_reset.cy(), 0.0, 0), Hit::Body);
    }

    #[test]
    fn settings_rows_hit_their_controls() {
        let g = geo(Style::Notch);
        let n = settings::ROWS.len();
        let right = settings_ctl_right(g.content);
        for (i, row) in settings::ROWS.iter().enumerate().take(4) {
            let y = g.content.y + i as f32 * ROW_H;
            let (rects, count) = settings::sub_rects(row.ctl, right, y);
            for k in 0..count {
                let r = rects[k];
                assert_eq!(hit_test(&g, Tab::Settings, r.cx(), r.cy(), 0.0, n), Hit::Set(i, k as u8), "row {i} sub {k}");
            }
        }
        // Scrolled by two rows, the third visible row is row 5.
        let (rects, _) = settings::sub_rects(settings::ROWS[5].ctl, right, g.content.y + 3.0 * ROW_H);
        assert_eq!(hit_test(&g, Tab::Settings, rects[0].cx(), rects[0].cy(), 2.0, n), Hit::Set(5, 0));
    }
}
