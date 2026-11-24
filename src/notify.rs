//! Toast-notification peeks, through Windows' `UserNotificationListener`.
//!
//! **This is best-effort and may simply not work.** `RequestAccessAsync` on this listener is
//! designed for apps with package identity (MSIX); a plain unpackaged .exe like this one is
//! commonly denied access outright, with no further event to react to - see the README. When
//! that happens this module quietly does nothing; it never retries, pops a dialog or blocks
//! startup. If it *is* granted (observed on some systems for unpackaged exes that already have a
//! taskbar presence), peeks work the same way a track-change or clipboard peek does.
//!
//! A worker thread subscribes once and sleeps on its message channel otherwise, exactly like the
//! media worker; the UI thread is woken with a posted message and reads the latest notification
//! out of a mutex-protected slot.

use crate::ffi::{PostMessageW, HANDLE};
use crate::win::WM_APP_NOTIFY;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::core::Interface;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{NotificationKinds, ToastNotification};

#[derive(Clone, Debug, Default)]
pub struct Notif {
    pub app: String,
    pub title: String,
    pub body: String,
}

pub enum Msg {
    Quit,
}

#[derive(Default)]
pub struct Shared {
    /// Bumped on every new notification so the UI thread can tell two different peeks apart
    /// even if their text happens to repeat.
    pub generation: u64,
    pub latest: Option<Notif>,
}

/// First two non-empty text lines of a toast's generic visual binding: title, then body.
fn extract_text(toast_xml: &XmlDocument) -> (String, String) {
    let mut lines = Vec::new();
    if let Ok(nodes) = toast_xml.GetElementsByTagName(&windows::core::HSTRING::from("text")) {
        for i in 0..nodes.Length().unwrap_or(0) {
            if let Ok(node) = nodes.Item(i) {
                if let Ok(text) = node.InnerText() {
                    let t = text.to_string();
                    if !t.trim().is_empty() {
                        lines.push(t);
                    }
                }
            }
            if lines.len() >= 2 {
                break;
            }
        }
    }
    let title = lines.first().cloned().unwrap_or_default();
    let body = lines.get(1).cloned().unwrap_or_default();
    (title, body)
}

fn read_latest(listener: &UserNotificationListener) -> Option<Notif> {
    let notifs = listener.GetNotificationsAsync(NotificationKinds::Toast).ok()?.get().ok()?;
    let n = notifs.Size().ok()?;
    if n == 0 {
        return None;
    }
    let newest = notifs.GetAt(n - 1).ok()?;
    let app = newest.AppInfo().ok().and_then(|i| i.DisplayInfo().ok()).and_then(|d| d.DisplayName().ok()).map(|s| s.to_string()).unwrap_or_default();
    let notification: ToastNotification = newest.Notification().ok()?.cast().ok()?;
    let xml = notification.Content().ok()?;
    let (title, body) = extract_text(&xml);
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(Notif { app, title, body })
}

/// Returns the command sender and the shared slot the UI thread polls after each `Ev::Notify`.
/// Does nothing (and the shared slot stays empty forever) if access is denied or the listener
/// cannot be created at all.
pub fn spawn(hwnd: HANDLE) -> (Sender<Msg>, Arc<Mutex<Shared>>) {
    let (tx, rx) = channel::<Msg>();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let shared2 = shared.clone();
    let _ = std::thread::Builder::new().name("notify".into()).stack_size(1 << 20).spawn(move || {
        unsafe { crate::ffi::CoInitializeEx(std::ptr::null_mut(), 0) };
        let Ok(listener) = UserNotificationListener::Current() else { return };
        let Ok(op) = listener.RequestAccessAsync() else { return };
        let Ok(status) = op.get() else { return };
        if status != UserNotificationListenerAccessStatus::Allowed {
            return; // denied, or not supported on this system - give up quietly, do not retry
        }

        let shared3 = shared2.clone();
        let listener2 = listener.clone();
        let handler = TypedEventHandler::new(move |_, _| {
            if let Some(n) = read_latest(&listener2) {
                let mut g = shared3.lock().unwrap_or_else(|e| e.into_inner());
                g.generation += 1;
                g.latest = Some(n);
                drop(g);
                unsafe { PostMessageW(hwnd, WM_APP_NOTIFY, 0, 0) };
            }
            Ok(())
        });
        let Ok(_token) = listener.NotificationChanged(&handler) else { return };

        // Nothing left to do but wait for Quit; everything else happens on the event callback,
        // which WinRT invokes on its own worker thread.
        loop {
            match rx.recv_timeout(Duration::from_secs(3600)) {
                Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    });
    (tx, shared)
}
