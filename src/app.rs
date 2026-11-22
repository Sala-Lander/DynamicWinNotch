//! Application state machine: hover / click / pin logic, spring targets, workers, rendering.
//!
//! The main loop calls `update()` after every batch of OS messages. `update()` never blocks and
//! returns what the loop should do next:
//!   * `Next::Frame`  - something is animating, wait for the next vsync and call again;
//!   * `Next::At(d)`  - nothing moves, sleep until the next deadline (or any message);
//!   * `Next::Idle`   - nothing scheduled at all.
//! A collapsed, silent island therefore costs ~0 CPU: no polling, no timers except one tiny
//! 4-second housekeeping tick.

use crate::clip::{self, ClipCmd, ClipFlags, ClipItem};
use crate::config::{self, Config, ExpandOn};
use crate::ffi::{MessageBeep, PostQuitMessage, MB_ICONASTERISK, WM_LBUTTONUP, WM_RBUTTONUP};
use crate::gfx::{pixmap_from_rgba, Canvas};
use crate::history::{Entry, History, Kind};
use crate::layout::{self, Geo, Hit, Rect, Tab, COLLAPSED, EXPANDED, PEEK, TAB_COUNT, TIMER_PRESETS, VISIBLE_ROWS};
use crate::media::{self, Cmd, MediaView, Msg, Shared};
use crate::power::{PowerEvent, Watcher};
use crate::settings::{self, Action};
use crate::spring::Spring;
use crate::sys::{self, Sampler};
use crate::timer::Timer;
use crate::ui::{self, PeekInfo, PeekKind, PowerIcon, Scene, TimerView};
use crate::util::{clamp01, fmt_bytes, single_line};
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
const TIMER_PEEK_MS: u64 = 6000;
/// "Saved" note in the Settings header: fully visible for this long, then it fades out.
const SAVED_HOLD: f32 = 1.0;
const SAVED_FADE: f32 = 0.6;
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
    sscroll_s: Spring,
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
    sscroll_target: f32,
    flash: Option<(usize, Instant)>,

    media: MediaView,
    media_tx: Sender<Msg>,
    media_shared: Arc<Mutex<Shared>>,
    clip: History,
    clip_tx: Sender<ClipCmd>,
    clip_rx: Receiver<ClipItem>,
    clip_flags: Arc<ClipFlags>,
    sampler: Sampler,
    timer: Timer,
    last_timer_secs: u64,
    power: Watcher,
    autostart: bool,
    saved_at: Option<Instant>,
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
        let clip_flags = ClipFlags::new(cfg.clip_images, cfg.privacy_filter);
        let (clip_tx, clip_rx) = clip::spawn(win.hwnd, clip_flags.clone());
        let _ = clip_tx.send(ClipCmd::Read); // seed the history with whatever is on the clipboard now

        let now = Instant::now();
        let tab = cfg.start_tab;
        let mut clip = History::new(cfg.history_size);
        clip.set_budget(cfg.image_budget_mb as usize * 1024 * 1024);
        let app = App {
            geo: layout::geo(cfg.style),
            clip,
            cfg,
            win,
            cv,
            tray,
            sw: Spring::new(COLLAPSED.0, 380.0, 0.80),
            sh: Spring::new(COLLAPSED.1, 340.0, 0.66),
            peek_s: Spring::new(0.0, 300.0, 1.0),
            tab_s: Spring::new(tab.index() as f32, 320.0, 0.85),
            scroll_s: Spring::new(0.0, 260.0, 1.0),
            sscroll_s: Spring::new(0.0, 260.0, 1.0),
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
            sscroll_target: 0.0,
            flash: None,
            media: MediaView::new(),
            media_tx,
            media_shared,
            clip_tx,
            clip_rx,
            clip_flags,
            sampler: Sampler::new(),
            timer: Timer::default(),
            last_timer_secs: u64::MAX,
            power: Watcher::new(sys::power_now()),
            autostart: win::autostart_enabled(),
            saved_at: None,
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
            && self.sscroll_s.at_rest()
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
        for s in [
            &mut self.sw,
            &mut self.sh,
            &mut self.peek_s,
            &mut self.tab_s,
            &mut self.scroll_s,
            &mut self.sscroll_s,
            &mut self.press_s,
        ] {
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
        let max_s = settings::ROWS.len().saturating_sub(VISIBLE_ROWS) as f32;
        self.sscroll_target = self.sscroll_target.clamp(0.0, max_s);
        self.sscroll_s.set_target(self.sscroll_target);
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
                self.hit_at(cx, cy)
            } else {
                Hit::None
            };
            if !same_hit(h, self.hover) {
                self.hover = h;
                self.dirty = true;
            }
        }
    }

    /// What is under (x, y) on the current tab (logical window coordinates).
    fn hit_at(&self, x: f32, y: f32) -> Hit {
        let (scroll, rows) = match self.tab {
            Tab::Settings => (self.sscroll_s.pos, settings::ROWS.len()),
            _ => (self.scroll_s.pos, self.clip.len()),
        };
        layout::hit_test(&self.geo, self.tab, x, y, scroll, rows)
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
        if t == Tab::Settings {
            self.autostart = win::autostart_enabled(); // may have been changed from the tray menu
        }
        self.dirty = true;
    }

    fn start_peek(&mut self, info: PeekInfo, for_ms: u64, now: Instant) {
        if self.expanded {
            return;
        }
        self.peek = Some((info, now + Duration::from_millis(for_ms)));
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
        self.update_timer(now);
        if let Some(t0) = self.saved_at {
            if now.saturating_duration_since(t0).as_secs_f32() > SAVED_HOLD + SAVED_FADE {
                self.saved_at = None;
                self.dirty = true;
            }
        }
        if now.saturating_duration_since(self.last_housekeeping) >= HOUSEKEEPING_EVERY {
            self.last_housekeeping = now;
            self.housekeeping();
        }
    }

    /// Milliseconds on the app clock (what `Timer` is driven with).
    fn clock_ms(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.start).as_millis() as u64
    }

    fn update_timer(&mut self, now: Instant) {
        let t = self.clock_ms(now);
        if self.timer.tick(t) {
            self.on_timer_done(now);
        }
        // The shown time only changes once per second: repaint exactly then.
        let secs = self.timer.display_secs(t);
        if secs != self.last_timer_secs {
            self.last_timer_secs = secs;
            if self.timer.running() {
                self.dirty = true;
            }
        }
    }

    fn on_timer_done(&mut self, now: Instant) {
        if self.cfg.timer_sound {
            unsafe { MessageBeep(MB_ICONASTERISK) };
        }
        let minutes = self.timer.total_ms() / 60_000;
        let info = PeekInfo::new(PeekKind::Timer, "Timer finished".to_string(), format!("{minutes} min"));
        self.start_peek(info, TIMER_PEEK_MS, now);
        self.dirty = true;
    }

    fn timer_view(&self, now: Instant) -> TimerView {
        let t = self.clock_ms(now);
        TimerView {
            running: self.timer.running(),
            finished: self.timer.finished(),
            secs: self.timer.display_secs(t),
            frac: self.timer.fraction(t),
            total_min: (self.timer.total_ms() / 60_000) as u32,
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
            if self.flash.is_some() || self.saved_at.is_some() || (self.marquee && self.tab == Tab::Media) {
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
        if let Some(w) = self.timer.next_wake_ms(self.clock_ms(now)) {
            consider(now + Duration::from_millis(w));
        }
        if let Some(t0) = self.saved_at {
            consider(t0 + Duration::from_secs_f32(SAVED_HOLD + SAVED_FADE + 0.02));
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
                Ev::Power => self.on_power(now),
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
        let hit = self.hit_at(x, y);
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
            (Btn::Left, Hit::Set(row, sub)) => self.on_setting(row, sub as usize, now),
            (Btn::Left, Hit::Preset(i)) => {
                self.timer.start_preset(TIMER_PRESETS[i.min(TIMER_PRESETS.len() - 1)], self.clock_ms(now));
                self.last_timer_secs = u64::MAX;
            }
            (Btn::Left, Hit::TimerToggle) => {
                self.timer.toggle(self.clock_ms(now));
                self.last_timer_secs = u64::MAX;
            }
            (Btn::Left, Hit::TimerMinus) => self.timer.adjust(-1, self.clock_ms(now)),
            (Btn::Left, Hit::TimerPlus) => self.timer.adjust(1, self.clock_ms(now)),
            (Btn::Left, Hit::TimerReset) => self.timer.reset(),
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
            let next = (self.tab.index() as i32 - notches.signum() as i32).clamp(0, TAB_COUNT as i32 - 1) as usize;
            self.set_tab(Tab::from_index(next));
        } else if self.tab == Tab::Clipboard {
            let max = self.clip.len().saturating_sub(VISIBLE_ROWS) as f32;
            self.scroll_target = (self.scroll_target - notches).clamp(0.0, max);
        } else if self.tab == Tab::Settings {
            let max = settings::ROWS.len().saturating_sub(VISIBLE_ROWS) as f32;
            self.sscroll_target = (self.sscroll_target - notches).clamp(0.0, max);
        } else if self.tab == Tab::Timer {
            let now = Instant::now();
            self.timer.adjust(notches.signum() as i32, self.clock_ms(now));
            self.last_timer_secs = u64::MAX;
        }
        self.dirty = true;
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
            self.start_peek(PeekInfo::new(PeekKind::Track, l1, l2), PEEK_MS, now);
        }
        self.dirty = true;
    }

    fn on_clip_ready(&mut self, now: Instant) {
        // The first read after launch only seeds the history with what is already on the
        // clipboard; announcing that as "Copied" would be wrong.
        let seeding = now.saturating_duration_since(self.start) < Duration::from_secs(2);
        while let Ok(item) = self.clip_rx.try_recv() {
            let changed = match item {
                ClipItem::Text(t) => self.clip.push(&t),
                ClipItem::Files(paths) => self.clip.push_files(&paths),
                ClipItem::Image(img) => self.clip.push_image(img),
            };
            if changed {
                if self.cfg.auto_peek && !seeding {
                    if let Some(e) = self.clip.newest() {
                        let info = peek_for_entry(e);
                        self.start_peek(info, PEEK_MS, now);
                    }
                }
                self.dirty = true;
            }
        }
    }

    /// Put history row `i` back on the clipboard and move it to the front.
    fn copy_row(&mut self, i: usize, now: Instant) {
        let Some(e) = self.clip.get(i) else { return };
        let cmd = match &e.image {
            Some(img) => ClipCmd::WriteImage(img.clone()),
            None => ClipCmd::Write(e.text.clone()),
        };
        let _ = self.clip_tx.send(cmd);
        let idx = self.clip.bump(i);
        self.flash = Some((idx, now));
    }

    // ------------------------------------------------------------- settings
    fn on_setting(&mut self, row: usize, sub: usize, now: Instant) {
        let Some(r) = settings::ROWS.get(row) else { return };
        let old = self.cfg.clone();
        match settings::apply(&mut self.cfg, self.autostart, r.id, sub) {
            Action::Nothing => {}
            Action::Changed => {
                self.apply_config(&old);
                config::save(&self.cfg);
                self.saved_at = Some(now);
            }
            Action::Autostart(on) => {
                win::set_autostart(on);
                self.autostart = win::autostart_enabled();
            }
            Action::OpenConfig => {
                config::save(&self.cfg); // the file then shows exactly what the UI shows
                if let Some(p) = config::ensure_template() {
                    win::open_in_editor(&p);
                }
            }
            Action::Reload => self.reload_config(),
        }
        self.dirty = true;
    }

    /// Make the running app match `self.cfg`; `old` is what was in effect before.
    fn apply_config(&mut self, old: &Config) {
        let c = &self.cfg;
        self.clip.set_cap(c.history_size);
        self.clip.set_budget(c.image_budget_mb as usize * 1024 * 1024);
        self.clip_flags.images.store(c.clip_images, std::sync::atomic::Ordering::Relaxed);
        self.clip_flags.privacy.store(c.privacy_filter, std::sync::atomic::Ordering::Relaxed);
        if c.style != old.style {
            self.geo = layout::geo(c.style);
        }
        if c.hotkey != old.hotkey {
            self.win.register_hotkey(c.hotkey);
        }
        if c.album_art != old.album_art {
            let _ = self.media_tx.send(Msg::SetArt(c.album_art));
            if !c.album_art {
                self.media.art = None;
                self.media.accent = None;
            } else {
                let _ = self.media_tx.send(Msg::Refresh);
            }
        }
        // Keep the Settings page inside its range if the history shrank, etc.
        self.scroll_target = self.scroll_target.min(self.clip.len().saturating_sub(VISIBLE_ROWS) as f32);
        self.trimmed = false;
        self.dirty = true;
    }

    // ---------------------------------------------------------------- power
    fn on_power(&mut self, now: Instant) {
        let ev = self.power.update(sys::power_now());
        if !self.cfg.power_alerts {
            return;
        }
        let pct_text = |p: Option<u8>| p.map_or(String::new(), |v| format!("{v}%"));
        let level = |p: Option<u8>| p.map(|v| v as f32 / 100.0);
        let (icon, l1, l2, lvl) = match ev {
            Some(PowerEvent::Plugged(p)) => (PowerIcon::Plugged, "Charger connected".to_string(), pct_text(p), level(p)),
            Some(PowerEvent::Unplugged(p)) => (PowerIcon::Unplugged, "On battery".to_string(), pct_text(p), level(p)),
            Some(PowerEvent::Low(p)) => (PowerIcon::Low, "Battery low".to_string(), format!("{p}% left"), level(Some(p))),
            Some(PowerEvent::Critical(p)) => {
                (PowerIcon::Low, "Battery critical".to_string(), format!("{p}% \u{b7} plug in now"), level(Some(p)))
            }
            Some(PowerEvent::Full) => (PowerIcon::Full, "Fully charged".to_string(), "100%".to_string(), Some(1.0)),
            None => return,
        };
        let mut info = PeekInfo::new(PeekKind::Power(icon), l1, l2);
        info.level = lvl;
        self.start_peek(info, PEEK_MS, now);
        if self.expanded && self.tab == Tab::System {
            self.sample_now(); // refresh the battery ring right away
        }
    }

    fn tray_menu(&mut self) {
        let autostart = win::autostart_enabled();
        let items = [
            (1usize, "Pin open  (Ctrl+Alt+N)", self.pinned),
            (6, "Open settings", false),
            (2, "Start with Windows", autostart),
            (0, "", false),
            (3, "Reload config", false),
            (4, "Open config file", false),
            (0, "", false),
            (5, "Quit", false),
        ];
        match win::popup_menu(self.win.hwnd, &items) {
            1 => self.toggle_pin(),
            2 => {
                win::set_autostart(!autostart);
                self.autostart = win::autostart_enabled();
            }
            3 => self.reload_config(),
            4 => {
                if let Some(p) = config::ensure_template() {
                    win::open_in_editor(&p);
                }
            }
            5 => unsafe { PostQuitMessage(0) },
            6 => {
                self.set_tab(Tab::Settings);
                if !self.pinned {
                    self.toggle_pin();
                }
            }
            _ => {}
        }
    }

    fn reload_config(&mut self) {
        let old = std::mem::replace(&mut self.cfg, config::load());
        self.apply_config(&old);
        self.win.register_hotkey(self.cfg.hotkey);
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
            sscroll: self.sscroll_s.pos,
            autostart: self.autostart,
            saved: self.saved_at.map_or(0.0, |t0| {
                let e = now.saturating_duration_since(t0).as_secs_f32();
                clamp01(1.0 - (e - SAVED_HOLD) / SAVED_FADE)
            }),
            timer: self.timer_view(now),
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

/// The "Copied ..." peek for a freshly added clipboard entry.
fn peek_for_entry(e: &Entry) -> PeekInfo {
    let mut info = match e.kind {
        Kind::Text => PeekInfo::new(PeekKind::Clip, "Copied".to_string(), single_line(&e.text, 80)),
        Kind::Link(_) => PeekInfo::new(PeekKind::Clip, "Copied link".to_string(), single_line(&e.preview, 80)),
        Kind::Color((r, g, b)) => {
            let mut i = PeekInfo::new(PeekKind::Clip, "Copied colour".to_string(), format!("#{r:02X}{g:02X}{b:02X}"));
            i.color = Some((r, g, b));
            i
        }
        Kind::Files(n) => {
            let title = if n == 1 { "Copied file".to_string() } else { format!("Copied {n} files") };
            PeekInfo::new(PeekKind::Clip, title, single_line(&e.preview, 80))
        }
        Kind::Image => {
            let (line2, thumb) = match &e.image {
                Some(img) => (
                    format!("{}\u{d7}{} \u{b7} {}", img.w, img.h, fmt_bytes(img.raw_len)),
                    pixmap_from_rgba(img.thumb.clone(), img.thumb_w, img.thumb_h),
                ),
                None => (String::new(), None),
            };
            let mut i = PeekInfo::new(PeekKind::Clip, "Copied image".to_string(), line2);
            i.thumb = thumb;
            i
        }
    };
    if info.line2.is_empty() {
        info.line2 = " ".to_string();
    }
    info
}
