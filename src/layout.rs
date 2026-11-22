//! Geometry and hit-testing. Everything here is in *logical* pixels (480 x 200 window);
//! the renderer multiplies by the DPI scale. No OS calls, so it is unit-tested.

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
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Media, Tab::Clipboard, Tab::System];

    pub fn index(self) -> usize {
        self as usize
    }
    pub fn from_index(i: usize) -> Tab {
        Self::ALL[i.min(2)]
    }
    pub fn label(self) -> &'static str {
        match self {
            Tab::Media => "Media",
            Tab::Clipboard => "Clipboard",
            Tab::System => "System",
        }
    }
    pub fn parse(s: &str) -> Option<Tab> {
        match s.trim().to_ascii_lowercase().as_str() {
            "media" => Some(Tab::Media),
            "clipboard" | "clip" => Some(Tab::Clipboard),
            "system" | "sys" => Some(Tab::System),
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
    pub tabs: [Rect; 3],
    pub pin: Rect,
    pub clear: Rect,
    pub content: Rect,
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
}

pub fn geo(style: Style) -> Geo {
    let p = pill_rect(style, EXPANDED.0, EXPANDED.1);
    let (tab_w, gap) = (92.0, 4.0);
    let strip_w = 3.0 * tab_w + 2.0 * gap;
    let x0 = p.x + (p.w - strip_w) * 0.5;
    let ty = p.y + 10.0;
    let tabs = [
        Rect::new(x0, ty, tab_w, 24.0),
        Rect::new(x0 + (tab_w + gap), ty, tab_w, 24.0),
        Rect::new(x0 + 2.0 * (tab_w + gap), ty, tab_w, 24.0),
    ];
    let content = Rect::new(p.x + 18.0, p.y + 46.0, p.w - 36.0, p.h - 46.0 - 14.0);
    let art = Rect::new(content.x, content.y + 10.0, 92.0, 92.0);
    let text_x = art.right() + 16.0;
    let text_w = content.right() - text_x;
    let cx = text_x + text_w * 0.5;
    let cy = content.y + 88.0;
    Geo {
        pill: p,
        tabs,
        pin: Rect::new(p.right() - 18.0 - 24.0, ty, 24.0, 24.0),
        clear: Rect::new(p.x + 18.0, ty, 56.0, 24.0),
        content,
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
        ring_y: content.y + 36.0,
        ring_r: 26.0,
        col_w: content.w / 3.0,
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
    /// Clipboard row (absolute index into the history).
    Row(usize),
}

/// `scroll` is the clipboard scroll position in rows (fractional while animating).
pub fn hit_test(g: &Geo, tab: Tab, x: f32, y: f32, scroll: f32, rows: usize) -> Hit {
    if !g.pill.contains(x, y) {
        return Hit::None;
    }
    if g.pin.inflate(2.0).contains(x, y) {
        return Hit::Pin;
    }
    if tab == Tab::Clipboard && g.clear.contains(x, y) {
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
            if g.content.contains(x, y) {
                let idx = (scroll + (y - g.content.y) / ROW_H).floor();
                if idx >= 0.0 && (idx as usize) < rows {
                    return Hit::Row(idx as usize);
                }
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
        for i in 0..3 {
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
        let y0 = g.content.y + 3.0;
        assert_eq!(hit_test(&g, Tab::Clipboard, g.content.cx(), y0, 0.0, 10), Hit::Row(0));
        assert_eq!(hit_test(&g, Tab::Clipboard, g.content.cx(), y0 + ROW_H, 0.0, 10), Hit::Row(1));
        assert_eq!(hit_test(&g, Tab::Clipboard, g.content.cx(), y0, 2.0, 10), Hit::Row(2));
        // Past the last item.
        assert_eq!(hit_test(&g, Tab::Clipboard, g.content.cx(), y0 + ROW_H * 3.0, 0.0, 2), Hit::Body);
        // Clear button only exists on the clipboard tab.
        assert_eq!(hit_test(&g, Tab::Clipboard, g.clear.cx(), g.clear.cy(), 0.0, 3), Hit::Clear);
        assert_eq!(hit_test(&g, Tab::Media, g.clear.cx(), g.clear.cy(), 0.0, 3), Hit::Body);
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
        assert_eq!(Tab::from_index(9), Tab::System);
    }
}
