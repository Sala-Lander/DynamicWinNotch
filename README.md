# Notch Island

A little black pill at the top of your screen, Windows-edition "dynamic island." Hover it, it opens
into a small panel with five tabs. Move away, it closes. That's the whole idea.

No Electron, no webview, no background bloat — just a native Win32 window drawn with `tiny-skia`.

## What's in it

| Tab | What you get |
|---|---|
| 🎵 **Media** | Cover art, title, artist, progress bar you can click to seek, prev/play-pause/next. Works with anything that reports to Windows (Spotify, browsers, Media Player, ...). |
| 📋 **Clipboard** | History of everything you copy — text, links, colours (with a swatch), files, images (with a thumbnail). Search box to filter live. Click to copy back, right-click to pin, middle-click to delete. |
| 📊 **System** | CPU / GPU / RAM / battery as live rings, plus live network speed (↓/↑). |
| ⏱️ **Timer** | Countdown, stopwatch, or up to 5 reminders — one label cycles between the three modes. |
| ⚙️ **Settings** | Everything below, in one place. Changes apply instantly and save to `config.ini`. |

## Peeks — things that make the pill pop open briefly

| Trigger | Shows |
|---|---|
| Track changes | Cover art + title |
| New clipboard item | Thumbnail / swatch / preview |
| Timer or reminder goes off | Which one, plus a sound |
| Battery plugged/unplugged, low (20%), critical (10%), full | A battery icon |
| Windows notification arrives | Sender + text — click it to open the app |
| Volume or screen brightness changes | A level bar |
| Caps Lock / Num Lock pressed | Which one, on/off |
| Bluetooth device connects/disconnects | Device name |
| Mic or camera turns on/off | A dot stays on the collapsed pill while it's in use (🟠 mic, 🟢 camera) |

**Do Not Disturb** has three modes: `off`, `on` (mute track/clipboard/notification/Bluetooth/mic-cam
peeks), or `games` (same, but only while something is full-screen). Volume, brightness, lock-key,
timer and battery peeks always get through — they're answering something you just did.

## Full-screen behaviour

While a game, video or presentation is full-screen, the island does one of two things (pick in
Settings → *Full-screen behaviour*):

| Mode | Behaviour |
|---|---|
| **Hide** | The island disappears completely until you exit full-screen. |
| **Game bar** | It shrinks into a tiny, click-through strip that ignores the mouse and never expands, showing whichever stats you've picked. |

Game-bar stats, picked individually in Settings:

| Stat | Notes |
|---|---|
| FPS | Real frame capture via ETW (same technique PresentMon uses) on the foreground game. Needs admin rights or the *Performance Log Users* group, otherwise shows `FPS admin`. Vulkan/OpenGL games that skip DXGI show `FPS –`. |
| 1% low / 0.1% low | 99th / 99.9th percentile frame time over the last 10s, shown as FPS. |
| Frame time | In ms. |
| CPU / GPU / RAM | Same live rings as the System tab. |
| Ping | ICMP echo once a second to `ping_host` (default `1.1.1.1`). Hosts that block ICMP show `–`. |
| Network | Live down/up speed. |

## Settings, at a glance

| Setting | Default | What it does |
|---|---|---|
| Expand on | Hover | Hover or click to open the pill |
| Style | Notch | `notch` (fused to the screen edge) or `island` (floating pill) |
| Accent colour | Auto | From the album art, or pick your own |
| Album art | On | Show cover art in Media |
| Equalizer | On | Little animated bars while something plays |
| Peek on events | On | Master toggle for the peek popups above |
| Battery alerts | On | Peek on charger plug/unplug and low battery |
| Timer sound | On | Sound when the timer/reminder ends |
| Clipboard images | On | Keep copied images in history (compressed, budget-capped) |
| Windows notifications | **Off** | Peek on toast notifications — needs a Windows privacy permission, see below |
| Volume / brightness peek | On | Peek with a level bar on change |
| Caps/Num Lock peek | On | Peek when either is switched |
| Bluetooth peek | On | Peek on device connect/disconnect |
| Mic/camera dot | On | Coloured dot on the pill while mic/camera is active |
| Password-manager filter | On | Skip clipboard items flagged "don't monitor me" |
| Pin hotkey | On | `Ctrl+Alt+N` pins/unpins the island |
| Hide on full-screen | On | Whether full-screen behaviour (above) kicks in at all |
| Full-screen mode | Hide | `hide` or `gamebar`, see above |
| Game bar stats | FPS, 1%, frame time, CPU, GPU | Pick any combination |
| Ping host | `1.1.1.1` | What the game bar pings |
| Live network speed | On | Show ↓/↑ on the System tab |
| History size | 30 | How many clipboard items to keep |
| Hover delay | 80 ms | How long to hover before it opens |
| Frame rate cap | 60 | Caps animation smoothness, not idle cost (idle cost is ~0) |

Everything here lives in `%APPDATA%\NotchIsland\config.ini`, auto-created on first run. You almost
never need to touch it by hand — the Settings tab writes it for you — but tray → *Open config file* /
*Reload config* works if you want to.

## Getting it running

```
cargo build --release
```

Needs Rust 1.75+ and the MSVC build tools, on Windows 10 (1809+) or 11. Output is one file:
`target\release\notch-island.exe`. No installer, nothing else to drop next to it.

```
cargo test
```

runs the unit tests (geometry, springs, clipboard history, link/colour detection, settings, timer,
battery logic, config round-trip, FPS math, device-consent logic).

## Good to know before you rely on it

| Thing | Why |
|---|---|
| **Windows notifications are off by default** | They go through `UserNotificationListener`, which most unpackaged `.exe`s can't get live events from. Turn on *Settings → Privacy & security → Notifications → Let desktop apps access your notifications*, then flip the toggle here — it polls every 2s and rechecks the permission every 15s, no restart needed. |
| **FPS needs a permission bump** | ETW frame capture needs admin or the *Performance Log Users* group (`net localgroup "Performance Log Users" %USERNAME% /add`, then sign out/in). |
| **Primary monitor only** | The island always lives on your main display. |
| **GPU/network can show "n/a"** | If the PDH counter can't open on your machine (odd driver, no active adapter), that one gauge shows "n/a" instead of taking anything else down with it. |
| **Not code-signed** | SmartScreen may warn on first run — that's normal for a small indie exe, not a red flag. |
| **Clipboard search briefly takes keyboard focus** | Only while you're typing in it; `Escape`, `Enter`, or clicking elsewhere gives it back. |

## Project layout

```
src/main.rs      message loop, vsync pacing
src/app.rs       state machine: hover/click/pin, springs, workers, render scheduling
src/ui.rs        all drawing — pill, peeks, five tabs
src/layout.rs    geometry + hit-testing
src/settings.rs  Settings tab as data
src/timer.rs     countdown / stopwatch / reminders
src/power.rs     charger & low-battery detection
src/detect.rs    link & colour recognition for clipboard text
src/gfx.rs       tiny-skia canvas, glyphs, image blitting
src/win.rs       layered window, raw input, tray, autostart
src/ffi.rs       hand-written Win32 declarations
src/media.rs     now-playing (GSMTC) worker
src/notify.rs    toast-notification worker
src/devices.rs   volume, mic/camera, Bluetooth watchers
src/fps.rs       ETW-based FPS / 1% low / ping capture for the game bar
src/ping.rs      background ICMP ping
src/clip.rs      clipboard worker (text, files, images)
src/sys.rs       CPU / GPU / RAM / network / battery sampling (PDH)
src/history.rs   clipboard history model
src/spring.rs    spring physics for all the animation
src/config.rs    config.ini read/write
```
