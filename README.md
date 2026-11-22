# Notch Island

A dynamic-island style overlay for the top edge of your Windows desktop.
A matte-black pill sits at the top centre of the screen; point at it and it springs open into a
small control surface with five tabs.

| Tab | What it does |
|---|---|
| **Media** | Whatever is playing in *any* app (Spotify, browsers, Media Player, ...) via Windows' Global System Media Transport Controls: cover art, title, artist, progress bar (click to seek), previous / play-pause / next. |
| **Clipboard** | History of everything you copy: **text, links** (host highlighted), **colours** (`#RRGGBB`, `rgb(...)` get a swatch), **files** (Explorer copies) and **images** (thumbnail in the row; screenshots, "Copy image" in a browser, ...). Click a row to copy it back (images go back as images), right-click to pin, middle-click to delete, wheel to scroll, "Clear" to wipe unpinned items. |
| **System** | CPU, memory and battery as live ring gauges, plus thread count, used/total RAM and charge state. |
| **Timer** | Countdown with presets (1 / 5 / 10 / 25 / 45 min), start / pause / reset, +-1 min buttons, wheel to adjust. While it runs, the remaining time with a small progress ring shows in the collapsed pill; at zero the pill peeks and the system sound plays. |
| **Settings** | Every option in the island itself: expand mode, style, accent colour swatches, toggles (album art, equalizer, peeks, battery alerts, timer sound, clipboard images, password-manager filter, full-screen hiding, start with Windows), steppers (history size, hover delay), frame rate. Changes apply immediately and are saved to `config.ini`. |

Native Win32 layered window + `tiny-skia` software rendering. No webview, no GPU framework,
no Electron/Tauri runtime, no polling loop.

## Build

Windows 10 1809+ / Windows 11, Rust 1.75+ (stable), MSVC build tools.

```
cargo build --release
```

Result: `target\release\notch-island.exe` - one self-contained file (C runtime linked statically,
fonts are read from `C:\Windows\Fonts`, nothing else to install). `cargo test` runs the unit tests
for the geometry, hit-testing, spring physics, clipboard history (kinds, image budget), link / colour
detection, settings model, timer, battery-alert logic, config round-trip and helpers.

## Using it

* **Hover** the pill (or **click** it, see `expand_on`) to expand; move away to collapse.
* **Pin** with the thumbtack in the island, the tray icon (left-click) or **Ctrl+Alt+N**.
* **Tray icon** (right-click): pin, open settings, start with Windows, reload config, open config, quit.
* The pill briefly widens (peek) on a track change (cover + title), on every new clipboard item
  (thumbnail for images, swatch for colours), when the timer ends, and for battery events: charger
  plugged in / removed, battery low (20 %) and critical (10 %), fully charged.
* The island hides itself while a full-screen app, game or presentation is running.
* Wheel over the tab strip switches tabs.

## Configuration

Normally you never touch the file: use the **Settings** tab. `%APPDATA%\NotchIsland\config.ini` is
created on first run and rewritten (with comments) whenever you change a setting; hand edits work too
(tray -> *Open config file*, then *Reload config*). Keys: `expand_on`, `style` (`notch` = fused to the screen edge with concave ears,
`island` = floating pill), `start_tab`, `hover_delay_ms`, `collapse_delay_ms`, `history_size`,
`clip_images`, `image_budget_mb` (compressed-image memory budget, 8-512), `album_art`, `equalizer`,
`auto_peek`, `power_alerts`, `timer_sound`, `privacy_filter`, `hotkey`, `hide_fullscreen`, `fps_cap`,
`accent` (`auto` = from the cover art, or `#RRGGBB`). `start_tab` also accepts `timer` and `settings`.
Existing config files keep working; missing keys use their defaults.

## How it stays light

* **Zero idle cost.** A collapsed island with nothing playing does nothing: hover is detected with
  *raw mouse input* delivered to the window even while it is click-through (it wakes only when the
  mouse moves), media and clipboard changes arrive as OS events, and the only timer is a 4-second
  housekeeping tick (fullscreen check, keep-on-top).
* **Frames only while animating.** Spring physics (fixed 240 Hz sub-steps, frame-rate independent)
  drives pill size, tab slide, scroll and gauges. While any spring is moving the loop paces itself on
  `DwmFlush` (vsync) and paints at `fps_cap` (60); the moment everything settles it blocks in
  `GetMessage` again. The equalizer / progress bar / marquee use their own slow timers only while visible.
* **Click-through.** The window is a layered, per-pixel-alpha, topmost, no-activate tool window
  (no taskbar button). Fully transparent pixels never receive mouse input; additionally
  `WS_EX_TRANSPARENT` is set while collapsed and cleared on hover so even the pill itself does not
  block clicks until you actually reach for it.
* **Small footprint.** Software rendering into a 480x200 (DPI-scaled) DIB, glyph cache, 128 px album
  art, history capped at 4000 chars/entry, working set trimmed after collapsing. System stats are sampled
  once a second only while the System tab is open. Clipboard history lives in RAM only and is never written to disk;
  content flagged by password managers (`ExcludeClipboardContentFromMonitorProcessing`) is skipped.
* **Images stay cheap.** A copied image is deflate-compressed and thumbnailed (96x60) on the clipboard
  worker thread; only the compressed bytes and the thumbnail are kept. The oldest unpinned images are
  dropped once `image_budget_mb` is exceeded (the newest one always stays), identical images are
  deduplicated by hash, and images over 128 MB raw are ignored. Copying a row back decompresses it on
  the worker thread.
* **Timer costs one wake-up per second, only while it runs.** Paused or idle, it schedules nothing.

## Layout

```
src/main.rs     message loop, vsync pacing
src/app.rs      state machine: hover/click/pin, springs, workers, render scheduling
src/ui.rs       all drawing (pill, compact/peek content, five pages)
src/layout.rs   geometry + hit-testing (pure, unit-tested)
src/settings.rs Settings tab as data: rows, controls, what a click does to the config (pure, unit-tested)
src/timer.rs    countdown state machine (pure, unit-tested)
src/power.rs    charger / low-battery event detection (pure, unit-tested)
src/detect.rs   link and colour recognition for clipboard text (pure, unit-tested)
src/gfx.rs      tiny-skia canvas, shapes, glyph cache, image blitting
src/win.rs      layered window, DIB present, raw input, tray, autostart
src/ffi.rs      hand-written Win32 declarations (the `windows` crate is used only for WinRT)
src/media.rs    GSMTC worker thread + cover-art decoding
src/clip.rs     clipboard worker: files, text, images (arboard + Win32), compression, thumbnails
src/sys.rs      CPU / RAM / battery sampling
src/history.rs  clipboard history: kinds, pinning, image budget (pure, unit-tested)
src/spring.rs   damped-spring physics (pure, unit-tested)
src/config.rs   config.ini parser + writer (pure, unit-tested)
```

## Limitations

Primary monitor only. Clicking a *file* row copies the paths as text (it does not put the files themselves
back). Images are captured when no text is on the clipboard (spreadsheet cells give text, not a picture).
Media controls act on the session Windows
considers current (the same one the volume flyout shows). Not code-signed, so SmartScreen may warn on first run.
