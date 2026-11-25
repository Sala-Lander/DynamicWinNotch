//! Tiny `key = value` config file: `%APPDATA%\NotchIsland\config.ini`.
//! Missing file or unknown keys are fine; every value is clamped to a sane range.

use crate::layout::{Style, Tab};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpandOn {
    Hover,
    Click,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullscreenMode {
    /// Hide the island entirely (previous, still-default behaviour).
    Hide,
    /// Shrink to a small always-click-through bar showing live CPU/GPU/RAM.
    GameBar,
}

/// Game-bar readouts, stored as a bit set in `Config::game_items`.
pub mod gb {
    pub const FPS: u16 = 1 << 0;
    pub const LOW1: u16 = 1 << 1;
    pub const LOW01: u16 = 1 << 2;
    pub const FRAMETIME: u16 = 1 << 3;
    pub const CPU: u16 = 1 << 4;
    pub const GPU: u16 = 1 << 5;
    pub const RAM: u16 = 1 << 6;
    pub const PING: u16 = 1 << 7;
    pub const NET: u16 = 1 << 8;
    /// Readouts that need the present-event (ETW) frame tracker.
    pub const FRAMES: u16 = FPS | LOW1 | LOW01 | FRAMETIME;
    pub const DEFAULT: u16 = FPS | LOW1 | CPU | GPU | RAM;
    /// Config-file names, in display order.
    pub const NAMES: [(&str, u16); 9] = [
        ("fps", FPS),
        ("low1", LOW1),
        ("low01", LOW01),
        ("frametime", FRAMETIME),
        ("cpu", CPU),
        ("gpu", GPU),
        ("ram", RAM),
        ("ping", PING),
        ("net", NET),
    ];

    pub fn parse(v: &str) -> u16 {
        v.split([',', ' ', ';'])
            .filter_map(|t| NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(t.trim())).map(|(_, b)| *b))
            .fold(0, |a, b| a | b)
    }

    pub fn render(bits: u16) -> String {
        let v: Vec<&str> = NAMES.iter().filter(|(_, b)| bits & b != 0).map(|(n, _)| *n).collect();
        if v.is_empty() {
            "none".into()
        } else {
            v.join(",")
        }
    }
}

/// Do-not-disturb: mutes the informational peeks (track, clipboard, notification, Bluetooth,
/// mic / camera). Volume, lock-key, timer and battery peeks are direct feedback and stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dnd {
    Off,
    On,
    /// Only while a full-screen app or game is running.
    Games,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub expand_on: ExpandOn,
    pub style: Style,
    pub start_tab: Tab,
    pub hover_delay_ms: u32,
    pub collapse_delay_ms: u32,
    pub history_size: usize,
    pub album_art: bool,
    pub equalizer: bool,
    pub auto_peek: bool,
    pub privacy_filter: bool,
    pub hotkey: bool,
    pub hide_fullscreen: bool,
    pub fps_cap: u32,
    /// `None` = derive from album art.
    pub accent: Option<(u8, u8, u8)>,
    /// Keep images from the clipboard (stored compressed, bounded by `image_budget_mb`).
    pub clip_images: bool,
    /// Memory budget for clipboard images, in MiB (compressed size).
    pub image_budget_mb: u32,
    /// Peek when the charger is plugged / unplugged and when the battery runs low.
    pub power_alerts: bool,
    /// Play the system notification sound when the timer ends.
    pub timer_sound: bool,
    /// Peek on new Windows toast notifications. Requires notification-listener access, which is
    /// commonly denied to an unpackaged .exe - see the README.
    pub notifications: bool,
    /// What happens to the island while a full-screen app/game/presentation owns the screen.
    pub fullscreen_mode: FullscreenMode,
    /// Live upload / download speed on the System tab (Performance setting).
    pub net_speed: bool,
    /// Which readouts the game bar shows (`gb::*` bits).
    pub game_items: u16,
    /// Host pinged once a second while the game bar shows ping.
    pub ping_host: String,
    /// Peek with a level bar when the master volume changes.
    pub volume_peek: bool,
    /// Peek with a level bar when the (laptop) screen brightness changes.
    pub brightness_peek: bool,
    /// Peek when Caps Lock / Num Lock is switched.
    pub lock_peek: bool,
    /// Peek when a Bluetooth device connects or disconnects.
    pub bt_peek: bool,
    /// Coloured dot while an app uses the microphone (orange) or camera (green).
    pub privacy_dot: bool,
    pub dnd: Dnd,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            expand_on: ExpandOn::Hover,
            style: Style::Notch,
            start_tab: Tab::Media,
            hover_delay_ms: 80,
            collapse_delay_ms: 450,
            history_size: 30,
            album_art: true,
            equalizer: true,
            auto_peek: true,
            privacy_filter: true,
            hotkey: true,
            hide_fullscreen: true,
            fps_cap: 60,
            accent: None,
            clip_images: true,
            image_budget_mb: 64,
            power_alerts: true,
            timer_sound: true,
            notifications: true,
            fullscreen_mode: FullscreenMode::GameBar,
            net_speed: true,
            game_items: gb::DEFAULT,
            ping_host: "1.1.1.1".into(),
            volume_peek: true,
            brightness_peek: true,
            lock_peek: true,
            bt_peek: true,
            privacy_dot: true,
            dnd: Dnd::Off,
        }
    }
}

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn parse_hex(v: &str) -> Option<(u8, u8, u8)> {
    let h = v.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

pub fn parse(text: &str) -> Config {
    let mut c = Config::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "expand_on" => {
                c.expand_on = if v.eq_ignore_ascii_case("click") { ExpandOn::Click } else { ExpandOn::Hover }
            }
            "style" => {
                c.style = if v.eq_ignore_ascii_case("island") { Style::Island } else { Style::Notch }
            }
            "start_tab" => {
                if let Some(t) = Tab::parse(v) {
                    c.start_tab = t;
                }
            }
            "hover_delay_ms" => c.hover_delay_ms = v.parse::<u32>().unwrap_or(c.hover_delay_ms).min(2000),
            "collapse_delay_ms" => {
                c.collapse_delay_ms = v.parse::<u32>().unwrap_or(c.collapse_delay_ms).clamp(100, 5000)
            }
            "history_size" => c.history_size = v.parse::<usize>().unwrap_or(c.history_size).clamp(5, 200),
            "album_art" => c.album_art = truthy(v),
            "equalizer" => c.equalizer = truthy(v),
            "auto_peek" => c.auto_peek = truthy(v),
            "privacy_filter" => c.privacy_filter = truthy(v),
            "hotkey" => c.hotkey = truthy(v),
            "hide_fullscreen" => c.hide_fullscreen = truthy(v),
            "fps_cap" => c.fps_cap = v.parse::<u32>().unwrap_or(c.fps_cap).clamp(15, 240),
            "accent" => c.accent = if v.eq_ignore_ascii_case("auto") { None } else { parse_hex(v) },
            "clip_images" => c.clip_images = truthy(v),
            "image_budget_mb" => c.image_budget_mb = v.parse::<u32>().unwrap_or(c.image_budget_mb).clamp(8, 512),
            "power_alerts" => c.power_alerts = truthy(v),
            "timer_sound" => c.timer_sound = truthy(v),
            "notifications" => c.notifications = truthy(v),
            "fullscreen_mode" => {
                c.fullscreen_mode = if v.eq_ignore_ascii_case("gamebar") || v.eq_ignore_ascii_case("game_bar") {
                    FullscreenMode::GameBar
                } else {
                    FullscreenMode::Hide
                }
            }
            "net_speed" => c.net_speed = truthy(v),
            "volume_peek" => c.volume_peek = truthy(v),
            "brightness_peek" => c.brightness_peek = truthy(v),
            "lock_peek" => c.lock_peek = truthy(v),
            "bt_peek" => c.bt_peek = truthy(v),
            "privacy_dot" => c.privacy_dot = truthy(v),
            "dnd" => {
                c.dnd = match v.to_ascii_lowercase().as_str() {
                    "on" | "1" | "true" | "yes" => Dnd::On,
                    "games" | "game" | "fullscreen" => Dnd::Games,
                    _ => Dnd::Off,
                }
            }
            "gamebar_items" => c.game_items = gb::parse(v),
            "ping_host" if !v.is_empty() && v.len() <= 253 && !v.contains(char::is_whitespace) => {
                c.ping_host = v.to_string()
            }
            _ => {}
        }
    }
    c
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("NotchIsland").join("config.ini"))
}

pub fn load() -> Config {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse(&t))
        .unwrap_or_default()
}

fn b(v: bool) -> u8 {
    v as u8
}

/// The config file as text, with comments. Used for the first-run file and whenever the
/// Settings tab saves, so the file on disk always documents every key.
pub fn render(c: &Config) -> String {
    let accent = match c.accent {
        Some((r, g, bl)) => format!("#{r:02X}{g:02X}{bl:02X}"),
        None => "auto".to_string(),
    };
    format!(
        "\
# Notch Island settings. Lines starting with # are comments.
# Most of these can also be changed in the Settings tab; use the tray menu -> \"Reload config\"
# after editing this file by hand.

# hover = expands when the pointer touches the notch, click = expands on click.
expand_on = {expand}

# notch  = fused to the top edge with concave ears (like a hardware notch)
# island = floating pill, a few pixels below the top edge
style = {style}

# Tab shown first: media | clipboard | system | timer | settings
start_tab = {tab}

# Milliseconds the pointer must rest on the notch before it expands (hover mode).
hover_delay_ms = {hover}

# Milliseconds after the pointer leaves before the island collapses.
collapse_delay_ms = {collapse}

# Clipboard entries kept in memory (pinned entries do not count). History is never written to disk.
history_size = {history}

# Keep images copied to the clipboard (1/0). Stored compressed, in memory only.
clip_images = {images}

# Memory budget for clipboard images in MiB (8-512, compressed size). Oldest unpinned images go first.
image_budget_mb = {budget}

# Fetch and show album art (1/0).
album_art = {art}

# Animated equalizer bars in the collapsed pill while media is playing (1/0).
# Costs a few redraws per second only while music plays.
equalizer = {eq}

# Briefly widen the pill on track change and on new clipboard content (1/0).
auto_peek = {peek}

# Peek when the charger is plugged in / removed and when the battery is low (1/0).
power_alerts = {power}

# Play the system sound when the timer ends (1/0).
timer_sound = {sound}

# Skip clipboard content that password managers flag as sensitive (1/0).
privacy_filter = {privacy}

# Ctrl+Alt+N toggles the island pinned open (1/0).
hotkey = {hotkey}

# Hide the island while a full-screen app, game or presentation is running (1/0).
hide_fullscreen = {hide}

# What happens instead of hiding, if hide_fullscreen is on and this is \"gamebar\":
# hide    = the island disappears completely (default)
# gamebar = it shrinks to a tiny click-through bar with live FPS / CPU / GPU / RAM (gamebar_items), never reacts to
#           the mouse and never expands - so it never gets in the way of a game.
fullscreen_mode = {fsmode}

# Game-bar readouts, comma separated, in any combination:
# fps, low1 (1% low), low01 (0.1% low), frametime, cpu, gpu, ram, ping, net  - or \"none\"
gamebar_items = {gbitems}

# Host for the game bar's ping readout (IPv4 address or name, e.g. your game server).
ping_host = {pinghost}

# Live upload / download speed on the System tab (1/0). Off = network counters are never opened.
net_speed = {netspeed}

# Peek on new Windows toast notifications (1/0). Needs notification-listener permission, which
# an unpackaged .exe like this one is commonly denied - see the README. Off by default.
notifications = {notif}

# Peek with a level bar when the volume changes (1/0).
volume_peek = {volpeek}

# Peek with a level bar when the screen brightness changes (1/0). Laptop panels; external
# monitors usually do not report brightness to Windows.
brightness_peek = {brightpeek}

# Peek when Caps Lock / Num Lock is switched (1/0).
lock_peek = {lockpeek}

# Peek when a Bluetooth device (earbuds, controller, ...) connects or disconnects (1/0).
bt_peek = {btpeek}

# Dot on the collapsed island while an app uses the microphone (orange) or camera (green) (1/0).
privacy_dot = {privdot}

# Do not disturb: off | on | games (only while a full-screen app runs). Mutes track, clipboard,
# notification, Bluetooth and mic / camera peeks; volume, lock keys, timer and battery stay.
dnd = {dnd}

# Animation frame cap (15-240). The loop is vsync-paced, so this only limits very high refresh rates.
fps_cap = {fps}

# auto = taken from the album art, or a fixed colour such as #7AA8FF
accent = {accent}
",
        expand = if c.expand_on == ExpandOn::Click { "click" } else { "hover" },
        style = if c.style == Style::Island { "island" } else { "notch" },
        tab = c.start_tab.label().to_ascii_lowercase(),
        hover = c.hover_delay_ms,
        collapse = c.collapse_delay_ms,
        history = c.history_size,
        images = b(c.clip_images),
        budget = c.image_budget_mb,
        art = b(c.album_art),
        eq = b(c.equalizer),
        peek = b(c.auto_peek),
        power = b(c.power_alerts),
        sound = b(c.timer_sound),
        privacy = b(c.privacy_filter),
        hotkey = b(c.hotkey),
        hide = b(c.hide_fullscreen),
        fsmode = if c.fullscreen_mode == FullscreenMode::GameBar { "gamebar" } else { "hide" },
        notif = b(c.notifications),
        fps = c.fps_cap,
        accent = accent,
        gbitems = gb::render(c.game_items),
        pinghost = c.ping_host,
        volpeek = b(c.volume_peek),
        brightpeek = b(c.brightness_peek),
        lockpeek = b(c.lock_peek),
        btpeek = b(c.bt_peek),
        privdot = b(c.privacy_dot),
        dnd = match c.dnd {
            Dnd::Off => "off",
            Dnd::On => "on",
            Dnd::Games => "games",
        },
        netspeed = b(c.net_speed),
    )
}

/// Write the default file if there is no config yet; returns the file path.
pub fn ensure_template() -> Option<PathBuf> {
    let p = path()?;
    if !p.exists() {
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::write(&p, render(&Config::default())).ok()?;
    }
    Some(p)
}

/// Persist the settings (called by the Settings tab after a change).
pub fn save(c: &Config) -> bool {
    let Some(p) = path() else { return false };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write beside the file and rename, so a crash cannot leave a half-written config.
    let tmp = p.with_extension("ini.tmp");
    std::fs::write(&tmp, render(c)).is_ok() && std::fs::rename(&tmp, &p).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        assert_eq!(parse(""), Config::default());
    }

    #[test]
    fn default_file_parses_to_defaults() {
        assert_eq!(parse(&render(&Config::default())), Config::default());
    }

    #[test]
    fn render_round_trips_every_field() {
        let c = Config {
            expand_on: ExpandOn::Click,
            style: Style::Island,
            start_tab: Tab::Settings,
            hover_delay_ms: 140,
            collapse_delay_ms: 900,
            history_size: 70,
            album_art: false,
            equalizer: false,
            auto_peek: false,
            privacy_filter: false,
            hotkey: false,
            hide_fullscreen: false,
            fps_cap: 120,
            accent: Some((1, 2, 250)),
            clip_images: false,
            image_budget_mb: 200,
            power_alerts: false,
            timer_sound: false,
            notifications: true,
            fullscreen_mode: FullscreenMode::GameBar,
            net_speed: false,
            game_items: gb::FPS | gb::LOW01 | gb::PING | gb::NET,
            ping_host: "eu.game.example".into(),
            volume_peek: false,
            brightness_peek: false,
            lock_peek: false,
            bt_peek: false,
            privacy_dot: false,
            dnd: Dnd::Games,
        };
        assert_eq!(parse(&render(&c)), c);
    }

    #[test]
    fn game_items_parse() {
        assert_eq!(gb::parse("fps, LOW1 ,ping,bogus"), gb::FPS | gb::LOW1 | gb::PING);
        assert_eq!(gb::parse("none"), 0);
        assert_eq!(parse(&render(&Config { game_items: 0, ..Config::default() })).game_items, 0);
        assert_eq!(parse("ping_host = a b").ping_host, "1.1.1.1");
    }

    #[test]
    fn new_keys_parse_and_clamp() {
        let c = parse("clip_images = 0\nimage_budget_mb = 1\npower_alerts = no\ntimer_sound = off\nstart_tab = timer");
        assert!(!c.clip_images && !c.power_alerts && !c.timer_sound);
        assert_eq!(c.image_budget_mb, 8);
        assert_eq!(c.start_tab, Tab::Timer);
        assert_eq!(parse("image_budget_mb = 99999").image_budget_mb, 512);
        let c = parse("notifications = yes\nfullscreen_mode = gamebar");
        assert!(c.notifications);
        assert_eq!(c.fullscreen_mode, FullscreenMode::GameBar);
        assert_eq!(parse("fullscreen_mode = hide").fullscreen_mode, FullscreenMode::Hide);
        assert_eq!(parse("fullscreen_mode = bogus").fullscreen_mode, FullscreenMode::Hide);
    }

    #[test]
    fn parses_and_clamps() {
        let c = parse(
            "expand_on = CLICK\nstyle=island\nstart_tab = system\nhover_delay_ms = 99999\n\
             collapse_delay_ms = 1\nhistory_size = 5000\nalbum_art = 0\nfps_cap = 5\naccent = #FF8800\n\
             bogus = 1\n; comment\n[section]\n",
        );
        assert_eq!(c.expand_on, ExpandOn::Click);
        assert_eq!(c.style, Style::Island);
        assert_eq!(c.start_tab, Tab::System);
        assert_eq!(c.hover_delay_ms, 2000);
        assert_eq!(c.collapse_delay_ms, 100);
        assert_eq!(c.history_size, 200);
        assert!(!c.album_art);
        assert_eq!(c.fps_cap, 15);
        assert_eq!(c.accent, Some((0xFF, 0x88, 0x00)));
    }

    #[test]
    fn bad_values_keep_defaults() {
        let c = parse("history_size = lots\naccent = #12\nstart_tab = nope");
        assert_eq!(c, Config::default());
    }
}
