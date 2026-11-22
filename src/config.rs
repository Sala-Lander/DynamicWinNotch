//! Tiny `key = value` config file: `%APPDATA%\NotchIsland\config.ini`.
//! Missing file or unknown keys are fine; every value is clamped to a sane range.

use crate::layout::{Style, Tab};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpandOn {
    Hover,
    Click,
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
        let Some((k, v)) = line.split_once('=') else { continue };
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

pub const TEMPLATE: &str = "\
# Notch Island settings. Lines starting with # are comments.
# Use the tray menu -> \"Reload config\" after editing.

# hover = expands when the pointer touches the notch, click = expands on click.
expand_on = hover

# notch  = fused to the top edge with concave ears (like a hardware notch)
# island = floating pill, a few pixels below the top edge
style = notch

# Tab shown first: media | clipboard | system
start_tab = media

# Milliseconds the pointer must rest on the notch before it expands (hover mode).
hover_delay_ms = 80

# Milliseconds after the pointer leaves before the island collapses.
collapse_delay_ms = 450

# Clipboard entries kept in memory (pinned entries do not count). History is never written to disk.
history_size = 30

# Fetch and show album art (1/0).
album_art = 1

# Animated equalizer bars in the collapsed pill while media is playing (1/0).
# Costs a few redraws per second only while music plays.
equalizer = 1

# Briefly widen the pill on track change and on new clipboard text (1/0).
auto_peek = 1

# Skip clipboard content that password managers flag as sensitive (1/0).
privacy_filter = 1

# Ctrl+Alt+N toggles the island pinned open (1/0).
hotkey = 1

# Hide the island while a full-screen app, game or presentation is running (1/0).
hide_fullscreen = 1

# Animation frame cap (15-240). The loop is vsync-paced, so this only limits very high refresh rates.
fps_cap = 60

# auto = taken from the album art, or a fixed colour such as #7AA8FF
accent = auto
";

/// Write the template if there is no config yet; returns the file path.
pub fn ensure_template() -> Option<PathBuf> {
    let p = path()?;
    if !p.exists() {
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::write(&p, TEMPLATE).ok()?;
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        assert_eq!(parse(""), Config::default());
    }

    #[test]
    fn template_parses_to_defaults() {
        assert_eq!(parse(TEMPLATE), Config::default());
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
