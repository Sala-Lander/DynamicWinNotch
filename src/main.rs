//! Notch Island: a dynamic-island style overlay for the top edge of the primary monitor.
//!
//! Native Win32 layered window + tiny-skia software rendering: no webview, no GPU framework,
//! no background polling. See README.md for the design notes.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod clip;
mod config;
mod detect;
mod devices;
mod ffi;
mod fps;
mod gfx;
mod history;
mod layout;
mod media;
mod notify;
mod ping;
mod power;
mod settings;
mod shelf;
mod sound;
mod spring;
mod sys;
mod timer;
mod ui;
mod util;
mod win;

use app::{App, Next};
use ffi::{DispatchMessageW, DwmFlush, GetMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT};
use std::time::{Duration, Instant};

/// Block until the next vsync (DWM composition tick). If DWM cannot pace us (remote session,
/// composition off) the call returns immediately, so fall back to a short sleep.
fn wait_vsync() {
    let t = Instant::now();
    unsafe { DwmFlush() };
    if t.elapsed() < Duration::from_millis(2) {
        std::thread::sleep(Duration::from_millis(4));
    }
}

fn main() {
    if win::already_running() {
        return;
    }
    win::set_dpi_aware();
    config::ensure_template();
    let Some(mut app) = App::new(config::load()) else {
        return;
    };

    let mut msg = MSG::default();
    'main: loop {
        // 1. Drain every pending OS message (they only fill the event queue).
        unsafe {
            while PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    break 'main;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // 2. Advance the application and see what it wants next.
        match app.update() {
            Next::Frame => wait_vsync(),
            Next::At(d) => {
                app.arm_timer(d);
                if !block_for_message(&mut msg) {
                    break 'main;
                }
            }
            Next::Idle => {
                app.disarm_timer();
                if !block_for_message(&mut msg) {
                    break 'main;
                }
            }
        }
    }
    app.shutdown();
}

/// Sleep until the next message (input, worker wake-up or the armed timer) and dispatch it.
/// Returns false on WM_QUIT / error.
fn block_for_message(msg: &mut MSG) -> bool {
    unsafe {
        let r = GetMessageW(msg, 0, 0, 0);
        if r <= 0 {
            return false;
        }
        TranslateMessage(msg);
        DispatchMessageW(msg);
    }
    true
}
