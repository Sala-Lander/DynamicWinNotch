//! Application state machine: hover / click / pin logic, spring targets, workers, rendering.
//!
//! The main loop calls `update()` after every batch of OS messages. `update()` never blocks and
//! returns what the loop should do next:
//!   * `Next::Frame`  - something is animating, wait for the next vsync and call again;
//!   * `Next::At(d)`  - nothing moves, sleep until the next deadline (or any message);
//!   * `Next::Idle`   - nothing scheduled at all.
//! A collapsed, silent island therefore costs ~0 CPU: no polling, no timers except one tiny
//! 4-second housekeeping tick.

use crate::clip::{self, ClipCmd};
use crate::config::{self, Config, ExpandOn};
use crate::ffi::{PostQuitMessage, WM_LBUTTONUP, WM_RBUTTONUP};
use crate::gfx::{pixmap_from_rgba, Canvas};
use crate::history::History;
use crate::layout::{self, Geo, Hit, Rect, Tab, COLLAPSED, EXPANDED, PEEK, VISIBLE_ROWS};
use crate::media::{self, Cmd, MediaView, Msg, Shared};
use crate::spring::Spring;
use crate::sys::Sampler;
use crate::ui::{self, PeekInfo, PeekKind, Scene};
use crate::util::{clamp01, single_line};
use crate::win::{self, Btn, Ev, Tray, Win};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub enum Next {
    Frame,
    At(Duration),
    Idle,
}

const DEFAULT_ACCENT: (u8, u8, u8) = (122, 168, 255);
const PEEK_MS: u64 = 3200;
const FLASH_SECS: f32 = 0.8;
const SAMPLE_EVERY: Duration = Duration::from_secs(1);
const HOUSEKEEPING_EVERY: Duration = Duration::from_secs(4);

pub struct App {
    cfg: Config,
    geo: Geo,
    win: Win,
    cv: Canvas,
    tray: Tray,

    // Springs: pill width / height, collapsed->peek blend, tab slide, clipboard scroll,
    // press "squash", and the three system gauges.
    sw: Spring,
    sh: Spring,
    peek_s: Spring,
    tab_s: Spring,
    scroll_s: Spring,
    press_s: Spring,
    gauge: [Spring; 3],

    expanded: bool,
    pinned: bool,
    tab: Tab,
    peek: Option<(PeekInfo, Instant)>,
    hover_since: Option<Instant>,
    leave_since: Option<Instant>,
    press_until: Option<Instant>,
    hover: Hit,
    /// Cursor in logical pixels relative to the window's top-left corner.
    cursor: (f32, f32),
    scroll_target: f32,
    flash: Option<(usize, Instant)>,

    media: MediaView,
    media_tx: Sender<Msg>,
    media_shared: Arc<Mutex<Shared>>,
    clip: History,
    clip_tx: Sender<ClipCmd>,
    clip_rx: Receiver<String>,
    sampler: Sampler,
    last_sample: Instant,

    start: Instant,
    last_step: Instant,
    last_render: Instant,
    last_periodic: Instant,
    last_housekeeping: Instant,
    dirty: bool,
    was_moving: bool,
    trimmed: bool,
    /// The last frame had a scrolling title, so it needs ~30 fps while on screen.
    marquee: bool,
    fs_hidden: bool,
}

fn ms(v: u32) -> Duration {
    Duration::from_millis(v as u64)
}

/// `Hit::Seek` carries the pointer fraction, which changes constantly; it is the same target.
fn same_hit(a: Hit, b: Hit) -> bool {
    match (a, b) {
        (Hit::Seek(_), Hit::Seek(_)) => true,
        _ => a == b,
    }
}

impl App {
    pub fn new(cfg: Config) -> Option<App> {
        let win = Win::create()?;
        let cv = Canvas::new(win.size_px.0 as u32, win.size_px.1 as u32, win.scale);
        let tray = Tray::add(win.hwnd);
        win.listen_clipboard();
        win.register_hotkey(cfg.hotkey);
        let (media_tx, media_shared) = media::spawn(win.hwnd, cfg.album_art);
        let (clip_tx, clip_rx) = clip::spawn(win.hwnd, cfg.privacy_filter);
        let _ = clip_tx.send(ClipCmd::Read); // seed the history with whatever is on the clipboard now

        let now = Instant::now();
        let tab = cfg.start_tab;
        let app = App {
            geo: layout::geo(cfg.style),
            clip: History::new(cfg.history_size),
            cfg,
            win,
            cv,
            tray,
            sw: Spring::new(COLLAPSED.0, 380.0, 0.80),
            sh: Spring::new(COLLAPSED.1, 340.0, 0.66),
            peek_s: Spring::new(0.0, 300.0, 1.0),
            tab_s: Spring::new(tab.index() as f32, 320.0, 0.85),
            scroll_s: Spring::new(0.0, 260.0, 1.0),
            press_s: Spring::new(0.0, 520.0, 0.55),
            gauge: [Spring::new(0.0, 110.0, 1.0); 3],
            expanded: false,
            pinned: false,
            tab,
            peek: None,
            hover_since: None,
            leave_since: None,
            press_until: None,
            hover: Hit::None,
            cursor: (-1000.0, -1000.0),
            scroll_target: 0.0,
            flash: None,
            media: MediaView::new(),
            media_tx,
            media_shared,
            clip_tx,
            clip_rx,
            sampler: Sampler::new(),
            last_sample: now,
            start: now,
            last_step: now,
            last_render: now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
            last_periodic: now,
            last_housekeeping: now,
            dirty: true,
            was_moving: false,
            trimmed: false,
            marquee: false,
            fs_hidden: false,
        };
        Some(app)
    }

    pub fn shutdown(&mut self) {
        self.tray.remove();
        let _ = self.media_tx.send(Msg::Quit);
        self.win.destroy();
    }

    pub fn arm_timer(&self, d: Duration) {
        self.win.arm_timer(d.as_millis().min(u32::MAX as u128) as u32);
    }

    pub fn disarm_timer(&self) {
        self.win.disarm_timer();
    }

    // ------------------------------------------------------------ main step
    pub fn update(&mut self) -> Next {
        let now = Instant::now();
        self.cursor = self.win.cursor_logical();
        self.process_events(now);
        self.update_timers(now);
        self.update_hover(now);
        self.apply_targets();
        let moving = self.step_springs(now);
        if moving {
            self.dirty = true;
        }
        self.update_periodic(now);
        self.win.set_click_through(!self.expanded && self.cfg.expand_on == ExpandOn::Hover);

        // Render at most `fps_cap` times per second.
        let min_iv = Duration::from_secs_f32(1.0 / self.cfg.fps_cap.max(1) as f32);
        let mut wait_render = None;
        if self.dirty {
            if !self.win.is_visible() {
                self.dirty = false; // hidden behind a full-screen app: nothing to paint
            } else {
                let since = now.saturating_duration_since(self.last_render);
                if since + Duration::from_millis(1) >= min_iv {
                    self.render(now);
                } else {
                    wait_render = Some(min_iv - since);
                }
            }
        }

        let still_moving = self.any_moving();
        self.was_moving = still_moving;
        if still_moving {
            return Next::Frame;
        }
        if let Some(d) = wait_render {
            return Next::At(d);
        }
        if !self.expanded && self.peek.is_none() && !self.dirty && !self.trimmed {
            win::trim_memory();
            self.trimmed = true;
        }
        match self.next_deadline(now) {
            Some(d) => Next::At(d),
            None => Next::Idle,
        }
    }

    fn any_moving(&self) -> bool {
        !(self.sw.at_rest()
            && self.sh.at_rest()
            && self.peek_s.at_rest()
            && self.tab_s.at_rest()
            && self.scroll_s.at_rest()
            && self.press_s.at_rest()
            && self.gauge.iter().all(|g| g.at_rest()))
    }

    fn step_springs(&mut self, now: Instant) -> bool {
        let mut dt = now.saturating_duration_since(self.last_step).as_secs_f32();
        if !self.was_moving {
            dt = dt.min(1.0 / 60.0); // first frame after idle: do not jump ahead
        }
        self.last_step = now;
        let mut moving = false;
        for s in [&mut self.sw, &mut self.sh, &mut self.peek_s, &mut self.tab_s, &mut self.scroll_s, &mut self.press_s] {
            if !s.at_rest() {
                s.step(dt);
                moving = true;
            }
        }
        for s in self.gauge.iter_mut() {
            if !s.at_rest() {
                s.step(dt);
                moving = true;
            }
        }
        moving
    }

    fn apply_targets(&mut self) {
        let want_peek = !self.expanded && self.peek.is_some();
        let (w, h) = if self.expanded {
            EXPANDED
        } else if want_peek {
            PEEK
        } else {
            COLLAPSED
        };
        self.sw.set_target(w);
        self.sh.set_target(h);
        self.peek_s.set_target(if want_peek { 1.0 } else { 0.0 });
        self.tab_s.set_target(self.tab.index() as f32);
        let max_scroll = self.clip.len().saturating_sub(VISIBLE_ROWS) as f32;
        self.scroll_target = self.scroll_target.clamp(0.0, max_scroll);
        self.scroll_s.set_target(self.scroll_target);
    }

    // ------------------------------------------------------- hover / modes
    /// Hot zone (logical window coordinates) that opens the island: the pill plus a small margin,
    /// reaching up to the screen edge so a fast flick to y = 0 still counts.
    fn hot_rect(&self) -> Rect {
        let (w, h) = if self.peek.is_some() { PEEK } else { COLLAPSED };
        let base = layout::pill_rect(self.cfg.style, w, h);
        Rect::new(base.x - 8.0, 0.0, base.w + 16.0, base.bottom() + 4.0)
    }

    fn update_hover(&mut self, now: Instant) {
        if !self.win.is_visible() {
            self.hover_since = None;
            return;
        }
        let (cx, cy) = self.cursor;
        if !self.expanded {
            if self.hot_rect().contains(cx, cy) {
                let since = *self.hover_since.get_or_insert(now);
                if self.cfg.expand_on == ExpandOn::Hover && now.saturating_duration_since(since) >= ms(self.cfg.hover_delay_ms) {
                    self.expand();
                }
            } else {
                self.hover_since = None;
            }
        } else {
            let zone = self.geo.pill.inflate(10.0);
            if self.pinned || zone.contains(cx, cy) {
                self.leave_since = None;
            } else {
                let since = *self.leave_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= ms(self.cfg.collapse_delay_ms) {
                    self.collapse();
                }
            }
            let h = if self.expanded && self.sh.pos > EXPANDED.1 * 0.8 {
                layout::hit_test(&self.geo, self.tab, cx, cy, self.scroll_s.pos, self.clip.len())
            } else {
                Hit::None
            };
            if !same_hit(h, self.hover) {
                self.hover = h;
                self.dirty = true;
            }
        }
    }

    fn expand(&mut self) {
        if self.expanded {
            return;
        }
        self.expanded = true;
        self.peek = None;
        self.hover_since = None;
        self.leave_since = None;
        self.trimmed = false;
        self.dirty = true;
        if self.tab == Tab::System {
            self.sample_now();
        }
    }

    fn collapse(&mut self) {
        self.expanded = false;
        self.hover = Hit::None;
        self.leave_since = None;
        self.trimmed = false;
        self.dirty = true;
    }

    fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
        if self.pinned {
            self.expand();
        } else {
            self.leave_since = None; // the hover logic starts the collapse countdown if needed
        }
        self.dirty = true;
    }

    fn set_tab(&mut self, t: Tab) {
        if t == self.tab {
            return;
        }
        self.tab = t;
        if t == Tab::System {
            self.sample_now();
        }
        self.dirty = true;
    }

    fn start_peek(&mut self, kind: PeekKind, line1: String, line2: String, now: Instant) {
        if self.expanded {
            return;
        }
        self.peek = Some((PeekInfo { kind, line1, line2 }, now + Duration::from_millis(PEEK_MS)));
        self.trimmed = false;
        self.dirty = true;
    }

    // ------------------------------------------------------------- sampling
    fn sample_now(&mut self) {
        self.sampler.sample();
        self.last_sample = Instant::now();
        let s = &self.sampler.stats;
        self.gauge[0].set_target(s.cpu / 100.0);
        self.gauge[1].set_target(s.ram_pct / 100.0);
        self.gauge[2].set_target(s.battery.map_or(0.0, |b| b as f32 / 100.0));
        self.dirty = true;
    }

    // --------------------------------------------------------------- timers
    fn update_timers(&mut self, now: Instant) {
        if let Some((_, until)) = &self.peek {
            if now >= *until {
                self.peek = None;
                self.dirty = true;
            }
        }
        if let Some(t) = self.press_until {
            if now >= t {
                self.press_until = None;
                self.press_s.set_target(0.0);
            }
        }
        if let Some((_, t0)) = self.flash {
            if now.saturating_duration_since(t0).as_secs_f32() > FLASH_SECS {
                self.flash = None;
                self.dirty = true;
            }
        }
        if self.expanded && self.tab == Tab::System && now.saturating_duration_since(self.last_sample) >= SAMPLE_EVERY {
            self.sample_now();
        }
        if now.saturating_duration_since(self.last_housekeeping) >= HOUSEKEEPING_EVERY {
            self.last_housekeeping = now;
            self.housekeeping();
        }
    }

    /// Hide for full-screen apps; keep the window above other topmost windows.
    fn housekeeping(&mut self) {
        let hide = self.cfg.hide_fullscreen && win::fullscreen_app_active();
        if hide != self.fs_hidden {
            self.fs_hidden = hide;
            self.win.set_visible(!hide);
            if !hide {
                self.dirty = true;
            }
        }
        if !hide {
            self.win.raise();
        }
    }

    /// How often the (otherwise static) frame must be repainted right now.
    fn periodic_interval(&self) -> Option<Duration> {
        if !self.win.is_visible() {
            return None;
        }
        let m = &self.media.info;
        let playing = m.has_session && m.playing;
        if self.expanded {
            if self.flash.is_some() || (self.marquee && self.tab == Tab::Media) {
                Some(Duration::from_millis(33))
            } else if self.tab == Tab::Media && playing && m.dur_ms > 0 {
                Some(Duration::from_millis(250)) // progress bar + elapsed time
            } else {
                None
            }
        } else if playing && self.cfg.equalizer && !(m.title.is_empty() && m.artist.is_empty()) {
            Some(Duration::from_millis(50)) // equalizer bars
        } else {
            None
        }
    }

    fn update_periodic(&mut self, now: Instant) {
        if let Some(p) = self.periodic_interval() {
            if now.saturating_duration_since(self.last_periodic) >= p {
                self.last_periodic = now;
                self.dirty = true;
            }
        }
    }

    fn next_deadline(&self, now: Instant) -> Option<Duration> {
        let mut best: Option<Instant> = None;
        let mut consider = |t: Instant| {
            best = Some(match best {
                Some(b) if b <= t => b,
                _ => t,
            });
        };
        if !self.expanded && self.cfg.expand_on == ExpandOn::Hover {
            if let Some(t) = self.hover_since {
                consider(t + ms(self.cfg.hover_delay_ms));
            }
        }
        if self.expanded && !self.pinned {
            if let Some(t) = self.leave_since {
                consider(t + ms(self.cfg.collapse_delay_ms));
            }
        }
        if let Some((_, until)) = &self.peek {
            consider(*until);
        }
        if let Some(t) = self.press_until {
            consider(t);
        }
        if let Some(p) = self.periodic_interval() {
            consider(self.last_periodic + p);
        }
        if self.expanded && self.tab == Tab::System {
            consider(self.last_sample + SAMPLE_EVERY);
        }
        consider(self.last_housekeeping + HOUSEKEEPING_EVERY);
        best.map(|t| t.saturating_duration_since(now))
    }

    // --------------------------------------------------------------- events
    fn process_events(&mut self, now: Instant) {
        let scale = self.win.scale;
        while let Some(ev) = win::pop() {
            match ev {
                Ev::Mouse => {} // the cursor is re-read at the top of every update
                Ev::Down(btn, x, y) => self.on_down(btn, x as f32 / scale, y as f32 / scale, now),
                Ev::Wheel(d) => self.on_wheel(d),
                Ev::MediaChanged => self.on_media(now),
                Ev::ClipUpdate => {
                    let _ = self.clip_tx.send(ClipCmd::Read);
                }
                Ev::ClipReady => self.on_clip_ready(now),
                Ev::Tray(m) => {
                    if m == WM_LBUTTONUP {
                        self.toggle_pin();
                    } else if m == WM_RBUTTONUP {
                        self.tray_menu();
                    }
                }
                Ev::Hotkey => self.toggle_pin(),
                Ev::DisplayChange => {
                    if self.win.reposition() {
                        self.cv.resize(self.win.size_px.0 as u32, self.win.size_px.1 as u32, self.win.scale);
                    }
                    self.dirty = true;
                }
                Ev::TaskbarCreated => self.tray.re_add(),
            }
        }
    }

    fn on_down(&mut self, btn: Btn, x: f32, y: f32, now: Instant) {
        if !self.expanded {
            // Click mode: the collapsed pill itself receives the click.
            if btn == Btn::Left && self.cfg.expand_on == ExpandOn::Click && self.hot_rect().contains(x, y) {
                self.expand();
            }
            return;
        }
        let hit = layout::hit_test(&self.geo, self.tab, x, y, self.scroll_s.pos, self.clip.len());
        if hit == Hit::None {
            return;
        }
        // Tiny "squash" feedback for any press inside the pill; released by a timer, so a button
        // released outside the window cannot leave it stuck.
        self.press_s.set_target(1.0);
        self.press_until = Some(now + Duration::from_millis(110));

        match (btn, hit) {
            (Btn::Left, Hit::Tab(i)) => self.set_tab(Tab::from_index(i)),
            (Btn::Left, Hit::Pin) => self.toggle_pin(),
            (Btn::Left, Hit::Clear) => {
                self.clip.clear_unpinned();
                self.scroll_target = 0.0;
            }
            (Btn::Left, Hit::Prev) => self.media_cmd(Cmd::Prev),
            (Btn::Left, Hit::Next) => self.media_cmd(Cmd::Next),
            (Btn::Left, Hit::Play) => {
                if self.media.info.has_session {
                    // Optimistic flip so the button reacts instantly; the real state follows.
                    self.media.info.pos_ms = self.media.pos_ms(now);
                    self.media.at = now;
                    self.media.info.playing = !self.media.info.playing;
                }
                self.media_cmd(Cmd::TogglePlay);
            }
            (Btn::Left, Hit::Seek(f)) => {
                let (can_seek, dur) = (self.media.info.can_seek, self.media.info.dur_ms);
                if can_seek && dur > 0 {
                    let target = (f * dur as f32) as i64;
                    self.media_cmd(Cmd::Seek(target));
                    self.media.info.pos_ms = target;
                    self.media.at = now;
                }
            }
            (Btn::Left, Hit::Row(i)) => self.copy_row(i, now),
            (Btn::Right, Hit::Row(i)) => self.clip.toggle_pin(i),
            (Btn::Middle, Hit::Row(i)) => self.clip.remove(i),
            _ => {}
        }
        self.dirty = true;
    }

    fn on_wheel(&mut self, delta: i32) {
        if !self.expanded || delta == 0 {
            return;
        }
        let (x, y) = self.cursor;
        if !self.geo.pill.contains(x, y) {
            return;
        }
        let notches = delta as f32 / 120.0;
        if y < self.geo.content.y {
            // Over the tab strip: the wheel flips tabs.
            let next = (self.tab.index() as i32 - notches.signum() as i32).clamp(0, 2) as usize;
            self.set_tab(Tab::from_index(next));
        } else if self.tab == Tab::Clipboard {
            let max = self.clip.len().saturating_sub(VISIBLE_ROWS) as f32;
            self.scroll_target = (self.scroll_target - notches).clamp(0.0, max);
        }
    }

    fn media_cmd(&self, c: Cmd) {
        let _ = self.media_tx.send(Msg::Cmd(c));
    }

    fn on_media(&mut self, now: Instant) {
        let (info, art_update) = {
            let mut g = self.media_shared.lock().unwrap_or_else(|e| e.into_inner());
            (g.info.clone(), g.art_update.take())
        };
        let prev_track = self.media.info.track_id;
        self.media.info = info;
        self.media.at = now;
        if let Some(update) = art_update {
            match update {
                Some(a) => {
                    self.media.art = pixmap_from_rgba(a.rgba, a.side, a.side);
                    self.media.accent = Some(a.accent);
                }
                None => {
                    self.media.art = None;
                    self.media.accent = None;
                }
            }
        }
        let m = &self.media.info;
        if self.cfg.auto_peek && m.has_session && m.playing && m.track_id != prev_track && !m.title.is_empty() {
            let (l1, l2) = (m.title.clone(), if m.artist.is_empty() { m.app.clone() } else { m.artist.clone() });
            self.start_peek(PeekKind::Track, l1, l2, now);
        }
        self.dirty = true;
    }

    fn on_clip_ready(&mut self, now: Instant) {
        // The first read after launch only seeds the history with what is already on the
        // clipboard; announcing that as "Copied" would be wrong.
        let seeding = now.saturating_duration_since(self.start) < Duration::from_secs(2);
        while let Ok(text) = self.clip_rx.try_recv() {
            if self.clip.push(&text) {
                if self.cfg.auto_peek && !seeding {
                    self.start_peek(PeekKind::Clip, "Copied".to_string(), single_line(&text, 80), now);
                }
                self.dirty = true;
            }
        }
    }

    /// Put history row `i` back on the clipboard and move it to the front.
    fn copy_row(&mut self, i: usize, now: Instant) {
        let Some(text) = self.clip.get(i).map(|e| e.text.clone()) else { return };
        let _ = self.clip_tx.send(ClipCmd::Write(text.clone()));
        self.clip.push(&text);
        let idx = self.clip.items().iter().position(|e| e.text == text).unwrap_or(0);
        self.flash = Some((idx, now));
    }

    fn tray_menu(&mut self) {
        let autostart = win::autostart_enabled();
        let items = [
            (1usize, "Pin open  (Ctrl+Alt+N)", self.pinned),
            (2, "Start with Windows", autostart),
            (0, "", false),
            (3, "Reload config", false),
            (4, "Open config file", false),
            (0, "", false),
            (5, "Quit", false),
        ];
        match win::popup_menu(self.win.hwnd, &items) {
            1 => self.toggle_pin(),
            2 => win::set_autostart(!autostart),
            3 => self.reload_config(),
            4 => {
                if let Some(p) = config::ensure_template() {
                    win::open_in_editor(&p);
                }
            }
            5 => unsafe { PostQuitMessage(0) },
            _ => {}
        }
    }

    fn reload_config(&mut self) {
        let new = config::load();
        self.clip.set_cap(new.history_size);
        self.geo = layout::geo(new.style);
        self.win.register_hotkey(new.hotkey);
        self.cfg = new;
        self.dirty = true;
    }

    // --------------------------------------------------------------- render
    fn render(&mut self, now: Instant) {
        let press = self.press_s.pos;
        let w = (self.sw.pos * (1.0 - 0.012 * press)).max(40.0);
        let h = (self.sh.pos * (1.0 - 0.020 * press)).max(20.0);
        let pill = layout::pill_rect(self.cfg.style, w, h);
        let t_open = clamp01((self.sh.pos - COLLAPSED.1) / (EXPANDED.1 - COLLAPSED.1));
        let flash = self
            .flash
            .map(|(i, t0)| (i, clamp01(1.0 - now.saturating_duration_since(t0).as_secs_f32() / FLASH_SECS)));
        let accent = self.cfg.accent.or(self.media.accent).unwrap_or(DEFAULT_ACCENT);

        let scene = Scene {
            cfg: &self.cfg,
            geo: &self.geo,
            pill,
            t_open,
            peek_mix: clamp01(self.peek_s.pos),
            tab_pos: self.tab_s.pos,
            hover: self.hover,
            pinned: self.pinned,
            now: now.saturating_duration_since(self.start).as_secs_f32(),
            media: &self.media,
            media_pos_ms: self.media.pos_ms(now),
            clip: &self.clip,
            scroll: self.scroll_s.pos,
            flash,
            gauges: [self.gauge[0].pos, self.gauge[1].pos, self.gauge[2].pos],
            stats: &self.sampler.stats,
            accent,
            peek: self.peek.as_ref().map(|(p, _)| p),
        };
        self.marquee = ui::draw(&mut self.cv, &scene);
        self.win.present(self.cv.pm.data());
        self.last_render = now;
        self.dirty = false;
    }
}
