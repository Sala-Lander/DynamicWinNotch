//! Clipboard watcher. The OS tells the window (WM_CLIPBOARDUPDATE) when the clipboard changes;
//! a worker thread then reads the text through `arboard`, so a slow clipboard owner can never
//! stall the UI thread.

use crate::ffi::{
    wide, GetClipboardSequenceNumber, IsClipboardFormatAvailable, PostMessageW, RegisterClipboardFormatW, HANDLE,
};
use crate::win::WM_APP_CLIP;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

pub enum ClipCmd {
    /// The clipboard changed: read it and report the text.
    Read,
    /// Put this text on the clipboard (used when the user clicks a history row).
    Write(String),
}

/// Returns the command sender and a receiver that yields newly copied text.
pub fn spawn(hwnd: HANDLE, privacy_filter: bool) -> (Sender<ClipCmd>, Receiver<String>) {
    let (cmd_tx, cmd_rx) = channel::<ClipCmd>();
    let (text_tx, text_rx) = channel::<String>();
    let _ = std::thread::Builder::new().name("clipboard".into()).stack_size(512 * 1024).spawn(move || {
        // Password managers set this format so clipboard tools skip their content.
        let exclude_fmt = unsafe { RegisterClipboardFormatW(wide("ExcludeClipboardContentFromMonitorProcessing").as_ptr()) };
        let mut clipboard = arboard::Clipboard::new().ok();
        // Sequence number of our own last write, so we do not read our own clicks back as "new".
        let mut own_seq = 0u32;

        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                ClipCmd::Write(text) => {
                    if clipboard.is_none() {
                        clipboard = arboard::Clipboard::new().ok();
                    }
                    if let Some(cb) = clipboard.as_mut() {
                        if cb.set_text(text).is_ok() {
                            own_seq = unsafe { GetClipboardSequenceNumber() };
                        }
                    }
                }
                ClipCmd::Read => {
                    // Several updates often arrive back to back; one read covers them all.
                    std::thread::sleep(Duration::from_millis(40));
                    let mut more = Vec::new();
                    while let Ok(c) = cmd_rx.try_recv() {
                        match c {
                            ClipCmd::Read => {}
                            other => more.push(other),
                        }
                    }
                    let seq = unsafe { GetClipboardSequenceNumber() };
                    let sensitive = privacy_filter && exclude_fmt != 0 && unsafe { IsClipboardFormatAvailable(exclude_fmt) } != 0;
                    if seq != own_seq && !sensitive {
                        if clipboard.is_none() {
                            clipboard = arboard::Clipboard::new().ok();
                        }
                        if let Some(cb) = clipboard.as_mut() {
                            if let Ok(text) = cb.get_text() {
                                if text_tx.send(text).is_ok() {
                                    unsafe { PostMessageW(hwnd, WM_APP_CLIP, 0, 0) };
                                }
                            }
                        }
                    }
                    for c in more {
                        if let ClipCmd::Write(text) = c {
                            if let Some(cb) = clipboard.as_mut() {
                                if cb.set_text(text).is_ok() {
                                    own_seq = unsafe { GetClipboardSequenceNumber() };
                                }
                            }
                        }
                    }
                }
            }
        }
    });
    (cmd_tx, text_rx)
}
