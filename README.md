# Notch Island

A dynamic-island style overlay for the top edge of your Windows desktop.
A matte-black pill sits at the top centre of the screen; point at it and it springs open into a
small control surface with three tabs.

| Tab | What it does |
|---|---|
| **Media** | Whatever is playing in *any* app (Spotify, browsers, Media Player, ...) via Windows' Global System Media Transport Controls: cover art, title, artist, progress bar (click to seek), previous / play-pause / next. |
| **Clipboard** | History of copied text (via `arboard`). Click a row to copy it back, right-click to pin, middle-click to delete, wheel to scroll, "Clear" to wipe unpinned items. |
| **System** | CPU, memory and battery as live ring gauges, plus thread count, used/total RAM and charge state. |

Native Win32 layered window + `tiny-skia` software rendering. No webview, no GPU framework,
no Electron/Tauri runtime, no polling loop.

## Build

Windows 10 1809+ / Windows 11, Rust 1.75+ (stable), MSVC build tools.

```
cargo build --release
```

Result: `target\release\notch-island.exe` - one self-contained file (C runtime linked statically,
fonts are read from `C:\Windows\Fonts`, nothing else to install). `cargo test` runs the unit tests
for the geometry, hit-testing, spring physics, clipboard history, config parser and helpers.

## Using it

* **Hover** the pill (or **click** it, see `expand_on`) to expand; move away to collapse.
* **Pin** with the thumbtack in the island, the tray icon (left-click) or **Ctrl+Alt+N**.
* **Tray icon** (right-click): pin, start with Windows, reload config, open config, quit.
* A track change briefly widens the pill (peek) with cover + title; so does new clipboard text.
* The island hides itself while a full-screen app, game or presentation is running.
* Wheel over the tab strip switches tabs.

## Configuration

`%APPDATA%\NotchIsland\config.ini` is created on first run (tray -> *Open config file*, then
*Reload config*). Keys: `expand_on`, `style` (`notch` = fused to the screen edge with concave ears,
`island` = floating pill), `start_tab`, `hover_delay_ms`, `collapse_delay_ms`, `history_size`,
`album_art`, `equalizer`, `auto_peek`, `privacy_filter`, `hotkey`, `hide_fullscreen`, `fps_cap`,
`accent` (`auto` = from the cover art, or `#RRGGBB`). `album_art` and `privacy_filter` apply after a restart.

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

## Layout

```
src/main.rs     message loop, vsync pacing
src/app.rs      state machine: hover/click/pin, springs, workers, render scheduling
src/ui.rs       all drawing (pill, compact/peek content, three pages)
src/layout.rs   geometry + hit-testing (pure, unit-tested)
src/gfx.rs      tiny-skia canvas, shapes, glyph cache, image blitting
src/win.rs      layered window, DIB present, raw input, tray, autostart
src/ffi.rs      hand-written Win32 declarations (the `windows` crate is used only for WinRT)
src/media.rs    GSMTC worker thread + cover-art decoding
src/clip.rs     clipboard worker (arboard)
src/sys.rs      CPU / RAM / battery sampling
src/history.rs  clipboard history (pure, unit-tested)
src/spring.rs   damped-spring physics (pure, unit-tested)
src/config.rs   config.ini parser (pure, unit-tested)
```

## Limitations

Primary monitor only. Clipboard history is text only. Media controls act on the session Windows
considers current (the same one the volume flyout shows). Not code-signed, so SmartScreen may warn on first run.
