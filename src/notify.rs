//! Toast-notification peeks, through Windows' `UserNotificationListener`.
//!
//! Access is a per-user Windows privacy switch ("Settings > Privacy & security > Notifications >
//! Let desktop apps access your notifications"). While it is off the worker reports `Denied`
//! (the UI shows a one-time hint) and re-checks every 15 s, so flipping the switch works without
//! a restart. Unpackaged .exes usually cannot subscribe to `NotificationChanged` ("Element not
//! found"), so the worker falls back to polling the toast list every 2 s while notification
//! peeks are on - new toasts are recognised by their id. With the setting off it only sleeps.
//!
use crate::ffi::{PostMessageW, HANDLE};
use crate::win::WM_APP_NOTIFY;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

#[derive(Clone, Debug, Default)]
pub struct Notif {
    pub app: String,
    /// App User Model ID of the sender (opens the app when the peek is clicked).
    pub aumid: String,
    pub title: String,
    pub body: String,
}

pub enum Msg {
    /// Notification peeks switched on / off in Settings.
    Enable(bool),
    Quit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    #[default]
    Unknown,
    Allowed,
    /// The Windows privacy switch is off (or the user declined).
    Denied,
    /// No listener on this system.
    Unsupported,
}

#[derive(Default)]
pub struct Shared {
    /// Bumped on every new notification so the UI thread can tell two different peeks apart
    /// even if their text happens to repeat.
    pub generation: u64,
    pub latest: Option<Notif>,
    pub access: Access,
}

/// First two non-empty lines of `texts`: title, then body.
pub fn title_body<I: IntoIterator<Item = String>>(texts: I) -> (String, String) {
    let mut it = texts.into_iter().filter(|t| !t.trim().is_empty());
    (it.next().unwrap_or_default(), it.next().unwrap_or_default())
}

fn to_notif(n: &UserNotification) -> Option<Notif> {
    let info = n.AppInfo().ok();
    let app = info
        .as_ref()
        .and_then(|i| i.DisplayInfo().ok())
        .and_then(|d| d.DisplayName().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let aumid = info.as_ref().and_then(|i| i.AppUserModelId().ok()).map(|s| s.to_string()).unwrap_or_default();
    // A listener result is a `Notification`, read through its visual's ToastGeneric binding
    // (it is not a `ToastNotification`).
    let binding =
        n.Notification().ok()?.Visual().ok()?.GetBinding(&KnownNotificationBindings::ToastGeneric().ok()?).ok()?;
    let texts = binding.GetTextElements().ok()?;
    let (title, body) = title_body((&texts).into_iter().filter_map(|t| t.Text().ok()).map(|h| h.to_string()));
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(Notif { app, aumid, title, body })
}

/// Ids of the current toasts, plus the newest one that is not in `seen`.
fn poll(listener: &UserNotificationListener, seen: &[u32]) -> Option<(Vec<u32>, Option<Notif>)> {
    let list = listener.GetNotificationsAsync(NotificationKinds::Toast).ok()?.get().ok()?;
    let n = list.Size().ok()?;
    let mut ids = Vec::with_capacity(n as usize);
    let mut newest = None;
    for i in 0..n {
        let Ok(item) = list.GetAt(i) else { continue };
        let Ok(id) = item.Id() else { continue };
        ids.push(id);
        if !seen.contains(&id) {
            newest = Some(item); // the list is oldest-first
        }
    }
    Some((ids, newest.as_ref().and_then(to_notif)))
}

/// Should `fresh` ids produce a peek? Only after a first baseline poll, so toasts that were
/// already in the Action Center when peeks were switched on are not replayed.
pub fn is_new(baseline: bool, fresh: bool) -> bool {
    baseline && fresh
}

fn access_now(listener: &UserNotificationListener, ask: bool) -> Access {
    let status = if ask {
        listener.RequestAccessAsync().ok().and_then(|op| op.get().ok())
    } else {
        listener.GetAccessStatus().ok()
    };
    match status {
        Some(UserNotificationListenerAccessStatus::Allowed) => Access::Allowed,
        Some(UserNotificationListenerAccessStatus::Denied) => Access::Denied,
        Some(_) => Access::Unknown,
        None => Access::Unsupported,
    }
}

fn publish(shared: &Mutex<Shared>, hwnd: HANDLE, n: Notif) {
    let mut g = shared.lock().unwrap_or_else(|e| e.into_inner());
    g.generation += 1;
    g.latest = Some(n);
    drop(g);
    unsafe { PostMessageW(hwnd, WM_APP_NOTIFY, 0, 0) };
}

/// Returns the command sender and the shared slot the UI thread polls after each `Ev::Notify`
/// (posted for new toasts and for access changes).
pub fn spawn(hwnd: HANDLE, enabled: bool) -> (Sender<Msg>, Arc<Mutex<Shared>>) {
    let (tx, rx) = channel::<Msg>();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let shared2 = shared.clone();
    let _ = std::thread::Builder::new().name("notify".into()).stack_size(1 << 20).spawn(move || {
        unsafe { crate::ffi::CoInitializeEx(std::ptr::null_mut(), 0) };
        let set_access = |a: Access| {
            let mut g = shared2.lock().unwrap_or_else(|e| e.into_inner());
            if g.access != a {
                g.access = a;
                drop(g);
                unsafe { PostMessageW(hwnd, WM_APP_NOTIFY, 0, 0) };
            }
        };
        let listener = match UserNotificationListener::Current() {
            Ok(l) => l,
            Err(_) => {
                set_access(Access::Unsupported);
                return;
            }
        };

        let mut enabled = enabled;
        let mut access = Access::Unknown;
        let mut asked = false;
        let mut seen: Vec<u32> = Vec::new();
        let mut baseline = false;
        // `Some` once the push subscription works; then polling is not needed.
        let mut pushed: Option<(Arc<Mutex<Vec<u32>>>, EventRegistrationToken)> = None;
        let unsubscribe = |pushed: &mut Option<(Arc<Mutex<Vec<u32>>>, EventRegistrationToken)>| {
            if let Some((_, token)) = pushed.take() {
                let _ = listener.RemoveNotificationChanged(token);
            }
        };
        loop {
            if enabled {
                // Re-checked every round: a revoked permission makes the toast list silently empty.
                let now = access_now(&listener, !asked);
                asked = true;
                if now != access {
                    if access == Access::Allowed {
                        unsubscribe(&mut pushed);
                        baseline = false; // do not replay old toasts once access returns
                        seen.clear();
                    }
                    access = now;
                    set_access(access);
                }
                if access == Access::Allowed && pushed.is_none() && !baseline {
                    // Try the push event first; on unpackaged apps it usually fails.
                    let ids = Arc::new(Mutex::new(poll(&listener, &[]).map(|p| p.0).unwrap_or_default()));
                    let (ids2, l2, sh) = (ids.clone(), listener.clone(), shared2.clone());
                    let handler = TypedEventHandler::new(move |_, _| {
                        let mut seen = ids2.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some((now_ids, fresh)) = poll(&l2, &seen) {
                            *seen = now_ids;
                            if let Some(n) = fresh {
                                publish(&sh, hwnd, n);
                            }
                        }
                        Ok(())
                    });
                    if let Ok(token) = listener.NotificationChanged(&handler) {
                        pushed = Some((ids, token));
                    }
                }
                if access == Access::Allowed && pushed.is_none() {
                    if let Some((ids, fresh)) = poll(&listener, &seen) {
                        if let Some(n) = fresh.filter(|_| is_new(baseline, true)) {
                            publish(&shared2, hwnd, n);
                        }
                        seen = ids;
                        baseline = true;
                    }
                }
            }
            let wait = match (enabled, access) {
                (false, _) => Duration::from_secs(3600),
                (true, Access::Allowed) if pushed.is_none() => Duration::from_secs(2),
                (true, Access::Unsupported) => Duration::from_secs(3600),
                (true, _) => Duration::from_secs(15),
            };
            match rx.recv_timeout(wait) {
                Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Msg::Enable(on)) => {
                    if on && !enabled {
                        // Fresh baseline: do not replay what piled up while off.
                        baseline = false;
                        seen.clear();
                        access = Access::Unknown;
                    } else if !on {
                        unsubscribe(&mut pushed);
                    }
                    enabled = on;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        unsubscribe(&mut pushed);
    });
    (tx, shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_and_body_skip_blank_lines() {
        let t = |v: &[&str]| title_body(v.iter().map(|s| s.to_string()));
        assert_eq!(t(&["Reminder", " ", "Standup at 10:00", "extra"]), ("Reminder".into(), "Standup at 10:00".into()));
        assert_eq!(t(&["", "Only"]), ("Only".into(), String::new()));
        assert_eq!(t(&[]), (String::new(), String::new()));
    }

    #[test]
    fn first_poll_is_only_a_baseline() {
        assert!(!is_new(false, true));
        assert!(is_new(true, true));
        assert!(!is_new(true, false));
    }
}
