//! Clipboard watcher. The OS tells the window (WM_CLIPBOARDUPDATE) when the clipboard changes;
//! a worker thread then reads it, so a slow clipboard owner can never stall the UI thread.
//!
//! What is captured, in priority order: a list of files (Explorer copy), text, an image.
//! Text wins over an image on purpose: copying cells from a spreadsheet puts both on the
//! clipboard, and the text is what people want back. Images are compressed (deflate) and given
//! a small thumbnail right here on the worker, so the UI thread never touches big buffers.

use crate::ffi::{
    wide, CloseClipboard, DragQueryFileW, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
    OpenClipboard, PostMessageW, RegisterClipboardFormatW, CF_HDROP, HANDLE,
};
use crate::gfx::resample_box;
use crate::history::ImageData;
use crate::util::{fit_dims, fnv1a, premultiply_rgba};
use crate::win::WM_APP_CLIP;
use std::borrow::Cow;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// Thumbnail bounds in pixels (the list shows them at roughly half that size).
pub const THUMB_MAX: (u32, u32) = (96, 60);
/// Images bigger than this (uncompressed RGBA) are ignored instead of copied around.
pub const MAX_RAW_BYTES: usize = 128 * 1024 * 1024;
/// At most this many paths are kept from one file copy.
const MAX_FILES: u32 = 64;

/// Settings the worker reads live, so the Settings tab takes effect without a restart.
pub struct ClipFlags {
    pub images: AtomicBool,
    pub privacy: AtomicBool,
}

impl ClipFlags {
    pub fn new(images: bool, privacy: bool) -> Arc<Self> {
        Arc::new(Self { images: AtomicBool::new(images), privacy: AtomicBool::new(privacy) })
    }
}

pub enum ClipCmd {
    /// The clipboard changed: read it and report what is on it.
    Read,
    /// Put this text on the clipboard (used when the user clicks a history row).
    Write(String),
    /// Put this image back on the clipboard.
    WriteImage(Arc<ImageData>),
}

pub enum ClipItem {
    Text(String),
    Files(Vec<String>),
    Image(ImageData),
}

/// Compress and thumbnail straight RGBA pixels. `None` for empty, inconsistent or huge images.
fn make_image(w: usize, h: usize, bytes: &[u8]) -> Option<ImageData> {
    let raw_len = w.checked_mul(h)?.checked_mul(4)?;
    if w == 0 || h == 0 || bytes.len() != raw_len || raw_len > MAX_RAW_BYTES {
        return None;
    }
    let (tw, th) = fit_dims(w as u32, h as u32, THUMB_MAX.0, THUMB_MAX.1);
    let mut thumb = resample_box(bytes, w as u32, h as u32, tw, th);
    premultiply_rgba(&mut thumb);
    Some(ImageData {
        w: w as u32,
        h: h as u32,
        packed: miniz_oxide::deflate::compress_to_vec(bytes, 1),
        raw_len,
        hash: fnv1a(bytes),
        thumb_w: tw,
        thumb_h: th,
        thumb,
    })
}

/// Paths of the files on the clipboard (CF_HDROP), or an empty list.
fn read_files() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP) == 0 {
            return out;
        }
        // Another program may hold the clipboard for a moment.
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(0) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !opened {
            return out;
        }
        let drop = GetClipboardData(CF_HDROP);
        if drop != 0 {
            let n = DragQueryFileW(drop, 0xFFFF_FFFF, null_mut(), 0).min(MAX_FILES);
            for i in 0..n {
                let len = DragQueryFileW(drop, i, null_mut(), 0) as usize;
                let mut buf = vec![0u16; len + 1];
                let got = DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32) as usize;
                out.push(String::from_utf16_lossy(&buf[..got.min(len)]));
            }
        }
        CloseClipboard();
    }
    out
}

/// Returns the command sender and a receiver that yields newly copied items.
pub fn spawn(hwnd: HANDLE, flags: Arc<ClipFlags>) -> (Sender<ClipCmd>, Receiver<ClipItem>) {
    let (cmd_tx, cmd_rx) = channel::<ClipCmd>();
    let (item_tx, item_rx) = channel::<ClipItem>();
    let _ = std::thread::Builder::new().name("clipboard".into()).stack_size(1024 * 1024).spawn(move || {
        // Password managers set this format so clipboard tools skip their content.
        let exclude_fmt = unsafe { RegisterClipboardFormatW(wide("ExcludeClipboardContentFromMonitorProcessing").as_ptr()) };
        let mut clipboard = arboard::Clipboard::new().ok();
        // Sequence number of our own last write, so we do not read our own clicks back as "new".
        let mut own_seq = 0u32;

        let write = |clipboard: &mut Option<arboard::Clipboard>, cmd: ClipCmd, own_seq: &mut u32| {
            if clipboard.is_none() {
                *clipboard = arboard::Clipboard::new().ok();
            }
            let Some(cb) = clipboard.as_mut() else { return };
            let ok = match cmd {
                ClipCmd::Write(text) => cb.set_text(text).is_ok(),
                ClipCmd::WriteImage(img) => match miniz_oxide::inflate::decompress_to_vec(&img.packed) {
                    Ok(raw) if raw.len() == img.raw_len => cb
                        .set_image(arboard::ImageData { width: img.w as usize, height: img.h as usize, bytes: Cow::Owned(raw) })
                        .is_ok(),
                    _ => false,
                },
                ClipCmd::Read => false,
            };
            if ok {
                *own_seq = unsafe { GetClipboardSequenceNumber() };
            }
        };

        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                ClipCmd::Read => {
                    // Several updates often arrive back to back; one read covers them all.
                    std::thread::sleep(Duration::from_millis(40));
                    let mut pending_writes = Vec::new();
                    while let Ok(c) = cmd_rx.try_recv() {
                        if !matches!(c, ClipCmd::Read) {
                            pending_writes.push(c);
                        }
                    }
                    let seq = unsafe { GetClipboardSequenceNumber() };
                    let sensitive = flags.privacy.load(Ordering::Relaxed)
                        && exclude_fmt != 0
                        && unsafe { IsClipboardFormatAvailable(exclude_fmt) } != 0;
                    if seq != own_seq && !sensitive {
                        if clipboard.is_none() {
                            clipboard = arboard::Clipboard::new().ok();
                        }
                        let item = read_item(clipboard.as_mut(), flags.images.load(Ordering::Relaxed));
                        if let Some(item) = item {
                            if item_tx.send(item).is_ok() {
                                unsafe { PostMessageW(hwnd, WM_APP_CLIP, 0, 0) };
                            }
                        }
                    }
                    for c in pending_writes {
                        write(&mut clipboard, c, &mut own_seq);
                    }
                }
                other => write(&mut clipboard, other, &mut own_seq),
            }
        }
    });
    (cmd_tx, item_rx)
}

fn read_item(cb: Option<&mut arboard::Clipboard>, want_images: bool) -> Option<ClipItem> {
    let files = read_files();
    if !files.is_empty() {
        return Some(ClipItem::Files(files));
    }
    let cb = cb?;
    if let Ok(text) = cb.get_text() {
        if !text.trim().is_empty() {
            return Some(ClipItem::Text(text));
        }
    }
    if want_images {
        if let Ok(img) = cb.get_image() {
            return make_image(img.width, img.height, &img.bytes).map(ClipItem::Image);
        }
    }
    None
}
