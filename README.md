# Notch Island

A dynamic-island style overlay for the top edge of your Windows desktop.
A matte-black pill sits at the top centre of the screen; point at it and it springs open into a
small control surface with five tabs.

| Tab | What it does |
|---|---|
| **Media** | Whatever is playing in *any* app (Spotify, browsers, Media Player, ...) via Windows' Global System Media Transport Controls: cover art, title, artist, progress bar (click to seek), previous / play-pause / next. |
| **Clipboard** | History of everything you copy: **text, links** (host highlighted), **colours** (`#RRGGBB`, `rgb(...)` get a swatch), **files** (Explorer copies) and **images** (thumbnail in the row; screenshots, "Copy image" in a browser, ...). A **search box** at the top filters the list live (case-insensitive, matches the preview text). Click a row to copy it back (images go back as images), right-click to pin, middle-click to delete, wheel to scroll, "Clear" to wipe unpinned items. |
| **System** | CPU, GPU, memory and battery as live ring gauges, plus thread count, used/total RAM, charge state and live **network throughput** (down / up, auto-scaled B·s⁻¹ / KB·s⁻¹ / MB·s⁻¹; switchable with *Live network speed*). |
| **Timer** | Three modes, cycled with the header's mode label (where "Clear" sits on the Clipboard tab): **Countdown** (presets, start / pause / reset, +-1 min, wheel to adjust; peeks and sounds at zero), **Stopwatch** (start / pause / reset, counts up), **Reminders** (quick-add +10 / +30 / +60 min, up to 5 pending at once, "Clear all"; each one peeks and sounds when it comes due). |
| **Settings** | Every option in the island itself: expand mode, style, accent colour swatches, toggles (album art, equalizer, peeks, battery alerts, timer sound, clipboard images, notification peeks, password-manager filter, full-screen hiding, start with Windows), a full-screen behaviour switch (hide / game bar), game-bar readout chips (FPS, 1 %, 0.1 %, frame time / CPU, GPU, RAM, ping, network), live network speed, steppers (history size, hover delay), frame rate. Changes apply immediately and are saved to `config.ini`. |

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
  (thumbnail for images, swatch for colours), when the timer/stopwatch-adjacent reminders fire, for
  battery events (charger plugged in / removed, battery low (20 %) and critical (10 %), fully
  charged), and - best-effort, see **Limitations** - on a new Windows toast notification.
* While a full-screen app, game or presentation is running, the island either hides itself or, if
  `fullscreen_mode` is set to **game bar**, shrinks into a tiny click-through bar at the top of the
  screen, which ignores the mouse entirely and never expands. Its readouts are picked with the
  *Game bar* chips in Settings: **FPS**, **1 % low**, **0.1 % low**, **frame time** (ms), CPU / GPU /
  RAM rings, **ping** (to `ping_host`) and network down / up. The bar grows to fit what is enabled.
* Wheel over the tab strip switches tabs.

## Configuration

Normally you never touch the file: use the **Settings** tab. `%APPDATA%\NotchIsland\config.ini` is
created on first run and rewritten (with comments) whenever you change a setting; hand edits work too
(tray -> *Open config file*, then *Reload config*). Keys: `expand_on`, `style` (`notch` = fused to the screen edge with concave ears,
`island` = floating pill), `start_tab`, `hover_delay_ms`, `collapse_delay_ms`, `history_size`,
`clip_images`, `image_budget_mb` (compressed-image memory budget, 8-512), `album_art`, `equalizer`,
`auto_peek`, `power_alerts`, `timer_sound`, `privacy_filter`, `hotkey`, `hide_fullscreen`, `fps_cap`,
`notifications` (toast-notification peeks, best-effort - see **Limitations**), `fullscreen_mode`
(`hide` or `gamebar`, what `hide_fullscreen` switches to when triggered), `gamebar_items`
(comma list of `fps,low1,low01,frametime,cpu,gpu,ram,ping,net`, or `none`), `ping_host` (default
`1.1.1.1`), `net_speed` (live network throughput, 1/0), `accent` (`auto` = from the
cover art, or `#RRGGBB`). `start_tab` also accepts `timer` and `settings`.
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
  art, history capped at 4000 chars/entry, working set trimmed after collapsing. System, GPU and
  network stats are sampled once a second, and only while the System tab is open (or, in `gamebar`
  full-screen mode, while the game bar itself is showing). Clipboard history lives in RAM only and is
  never written to disk; content flagged by password managers
  (`ExcludeClipboardContentFromMonitorProcessing`) is skipped.
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
src/notify.rs   toast-notification worker thread (WinRT UserNotificationListener, best-effort)
src/clip.rs     clipboard worker: files, text, images (arboard + Win32), compression, thumbnails
src/sys.rs      CPU / GPU / RAM / network / battery sampling (GPU and network via PDH)
src/history.rs  clipboard history: kinds, pinning, image budget (pure, unit-tested)
src/spring.rs   damped-spring physics (pure, unit-tested)
src/config.rs   config.ini parser + writer (pure, unit-tested)
```

## Limitations

Primary monitor only. Clicking a *file* row copies the paths as text (it does not put the files themselves
back). Images are captured when no text is on the clipboard (spreadsheet cells give text, not a picture).
Media controls act on the session Windows
considers current (the same one the volume flyout shows). Not code-signed, so SmartScreen may warn on first run.

**Notification peeks need Windows' permission.** They go through WinRT's `UserNotificationListener`.
Turn on *Settings > Privacy & security > Notifications > Let apps / desktop apps access your
notifications*; while that is off the island shows a one-time "Notification access blocked" peek and
re-checks every 15 s, so no restart is needed after allowing it. Unpackaged apps usually cannot
subscribe to the live notification event, so while peeks are on the island polls the toast list every
2 s (new toasts are recognised by id; nothing that was already there is replayed). With the setting
off the worker only sleeps.

**GPU usage and network speed go through PDH** (`\GPU Engine(*)\Utilization Percentage`,
`\Network Interface(*)\Bytes Received/sec` and `...Bytes Sent/sec`). GPU load is the busiest engine
(per-process instances summed per engine, like Task Manager); network speed is the busiest adapter, so
virtual adapters (Hyper-V / WSL / VPN) do not double-count. If a counter fails to open on a given machine (unsupported GPU driver, no active
network adapter, etc.) that one gauge just shows "n/a" / stays at the last good value - it does not
bring anything else down.

**The clipboard search box steals keyboard focus while you type in it**, via an explicit, narrow
exception to this window's usual "never takes focus" behaviour (`SetForegroundWindow` + `SetFocus`,
released back on `Escape`, `Enter`, a click elsewhere, or focus loss such as Alt+Tab). This is the one
place the island behaves like a normal focusable window, and it has had less real-world exercise than
the rest of the UI - if it ever seems to "eat" a keystroke meant for another window, click elsewhere
or press `Escape` to release it.

**Game bar mode** (`fullscreen_mode = gamebar`) makes the window click-through and ignore the mouse
entirely while a full-screen app is active, by design - that is what lets clicks and gameplay pass
straight through to the game underneath it.

**Game-bar FPS / lows / frame time** come from a private real-time ETW session on the DXGI and D3D9
`Present` events (the source PresentMon uses), measured for the foreground process - no injection,
works with any Direct3D game, and the session only exists while the game bar shows a frame readout.
Starting an ETW session needs administrator rights or membership of the *Performance Log Users* group;
without that the bar shows `FPS admin` (add yourself with `net localgroup "Performance Log Users"
%USERNAME% /add`, then sign out and in). Vulkan / OpenGL titles that do not present through DXGI show
`FPS –`. Lows are the 99th / 99.9th percentile frame time of the last 10 s, as FPS. Ping is an ICMP
echo once a second; hosts that drop ICMP show `–`.
