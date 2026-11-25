//! Small OS watchers that sleep until something happens and then post one message to the
//! window: master volume (COM endpoint callback), microphone / camera use (registry change
//! notifications on Windows' CapabilityAccessManager consent store) and Bluetooth connects
//! (WinRT `DeviceWatcher`). None of them polls.

use crate::ffi::wide;
use crate::ffi::{
    CreateEventW, PostMessageW, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
    WaitForMultipleObjects, HANDLE, HKEY_CURRENT_USER, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
    RRF_RT_REG_QWORD,
};
use crate::win::{WM_APP_BT, WM_APP_PRIVACY, WM_APP_VOLUME};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::core::{implement, IInspectable, HSTRING};
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice};
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher};
use windows::Foundation::Collections::IIterable;
use windows::Foundation::TypedEventHandler;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

fn co_init() {
    unsafe { crate::ffi::CoInitializeEx(null_mut(), 0) }; // multithreaded apartment
}

// ------------------------------------------------------------------ volume
#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback(HANDLE);

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if let Some(d) = unsafe { data.as_ref() } {
            let pct = (d.fMasterVolume.clamp(0.0, 1.0) * 100.0).round() as usize;
            unsafe { PostMessageW(self.0, WM_APP_VOLUME, pct, d.bMuted.as_bool() as isize) };
        }
        Ok(())
    }
}

/// Posts `WM_APP_VOLUME` whenever the default output's master volume or mute changes. Follows
/// the default device (headset plugged in, ...) with a cheap check every 10 s.
pub fn spawn_volume(hwnd: HANDLE) {
    let _ = std::thread::Builder::new().name("volume".into()).stack_size(256 << 10).spawn(move || unsafe {
        co_init();
        let Ok(devices) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return;
        };
        let callback: IAudioEndpointVolumeCallback = VolumeCallback(hwnd).into();
        let mut current: Option<(String, IAudioEndpointVolume)> = None;
        loop {
            if let Ok(dev) = devices.GetDefaultAudioEndpoint(eRender, eConsole) {
                let id = match dev.GetId() {
                    Ok(p) => {
                        let s = p.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(p.0 as *const c_void));
                        s
                    }
                    Err(_) => String::new(),
                };
                if current.as_ref().map(|c| c.0.as_str()) != Some(id.as_str()) {
                    if let Some((_, v)) = current.take() {
                        let _ = v.UnregisterControlChangeNotify(&callback);
                    }
                    if let Ok(v) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                        if v.RegisterControlChangeNotify(&callback).is_ok() {
                            current = Some((id, v));
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(10));
        }
    });
}

// ------------------------------------------------------- microphone / camera
const CONSENT: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\";

/// Which app (display name) currently uses the microphone / camera, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InUse {
    pub mic: Option<String>,
    pub cam: Option<String>,
}

/// Windows stamps `LastUsedTimeStart` when an app opens the device and `LastUsedTimeStop` (0
/// while still open) when it closes it.
pub fn consent_active(start: Option<u64>, stop: Option<u64>) -> bool {
    start.unwrap_or(0) != 0 && stop == Some(0)
}

/// Readable name from a consent-store key: `C:#Program Files#Zoom#bin#Zoom.exe` -> `Zoom`,
/// `Microsoft.WindowsCamera_8wekyb3d8bbwe` -> `WindowsCamera`.
pub fn consent_app_name(key: &str) -> String {
    if key.contains('#') {
        let file = key.rsplit('#').next().unwrap_or(key);
        let stem = if file.to_ascii_lowercase().ends_with(".exe") { &file[..file.len() - 4] } else { file };
        stem.to_string()
    } else {
        let family = key.split('_').next().unwrap_or(key);
        family.rsplit('.').next().unwrap_or(family).to_string()
    }
}

fn open_key(parent: HANDLE, sub: &str) -> HANDLE {
    let mut k: HANDLE = 0;
    let w = wide(sub);
    if unsafe { RegOpenKeyExW(parent, w.as_ptr(), 0, KEY_READ, &mut k) } == 0 {
        k
    } else {
        0
    }
}

fn subkeys(key: HANDLE) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = [0u16; 512];
    for i in 0.. {
        let mut n = buf.len() as u32;
        let r =
            unsafe { RegEnumKeyExW(key, i, buf.as_mut_ptr(), &mut n, null_mut(), null_mut(), null_mut(), null_mut()) };
        if r != 0 {
            break; // ERROR_NO_MORE_ITEMS (or anything else): done
        }
        out.push(String::from_utf16_lossy(&buf[..n as usize]));
    }
    out
}

fn qword(key: HANDLE, sub: &str, value: &str) -> Option<u64> {
    let (s, v) = (wide(sub), wide(value));
    let mut data = 0u64;
    let mut cb = 8u32;
    let r = unsafe {
        RegGetValueW(
            key,
            s.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_QWORD,
            null_mut(),
            &mut data as *mut u64 as *mut c_void,
            &mut cb,
        )
    };
    (r == 0).then_some(data)
}

fn active_app(root: HANDLE) -> Option<String> {
    let active =
        |k: HANDLE, sub: &str| consent_active(qword(k, sub, "LastUsedTimeStart"), qword(k, sub, "LastUsedTimeStop"));
    for name in subkeys(root) {
        if name == "NonPackaged" {
            let np = open_key(root, &name);
            if np == 0 {
                continue;
            }
            let hit = subkeys(np).into_iter().find(|n| active(np, n));
            unsafe { RegCloseKey(np) };
            if let Some(n) = hit {
                return Some(consent_app_name(&n));
            }
        } else if active(root, &name) {
            return Some(consent_app_name(&name));
        }
    }
    None
}

/// Watches the consent store and posts `WM_APP_PRIVACY` when microphone / camera use changes.
pub fn spawn_privacy(hwnd: HANDLE) -> Arc<Mutex<InUse>> {
    let shared = Arc::new(Mutex::new(InUse::default()));
    let out = shared.clone();
    let _ = std::thread::Builder::new().name("privacy".into()).stack_size(256 << 10).spawn(move || unsafe {
        let names = ["microphone", "webcam"];
        let mut keys: [HANDLE; 2] = [0; 2];
        // A watch is one-shot; re-arm a key only after its own event fired (re-arming a pending
        // watch would stack another wait operation).
        let mut armed = [false; 2];
        let events = [CreateEventW(null(), 0, 0, null()), CreateEventW(null(), 0, 0, null())];
        loop {
            for i in 0..2 {
                if keys[i] == 0 {
                    keys[i] = open_key(HKEY_CURRENT_USER, &format!("{CONSENT}{}", names[i]));
                }
                if keys[i] != 0 && !armed[i] {
                    // Armed before reading, so no change slips through.
                    let filter = REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET;
                    armed[i] = RegNotifyChangeKeyValue(keys[i], 1, filter, events[i], 1) == 0;
                }
            }
            let now = InUse { mic: active_app(keys[0]), cam: active_app(keys[1]) };
            {
                let mut g = out.lock().unwrap_or_else(|e| e.into_inner());
                if *g != now {
                    *g = now;
                    PostMessageW(hwnd, WM_APP_PRIVACY, 0, 0);
                }
            }
            // Missing keys (no app ever asked) or failed watches are retried once a minute.
            let wait = if armed.contains(&false) { 60_000 } else { u32::MAX };
            let r = WaitForMultipleObjects(2, events.as_ptr(), 0, wait);
            if (r as usize) < armed.len() {
                armed[r as usize] = false; // WAIT_OBJECT_0 + i: that watch completed
            }
        }
    });
    shared
}

// --------------------------------------------------------------- Bluetooth
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtEvent {
    pub name: String,
    pub connected: bool,
}

fn watch_bt(w: &DeviceWatcher, hwnd: HANDLE, out: &Arc<Mutex<Vec<BtEvent>>>) -> windows::core::Result<()> {
    // Devices already connected at start are reported as "Added" first: only announce changes
    // after the initial enumeration completed.
    let ready = Arc::new(AtomicBool::new(false));
    let names: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    let publish = move |out: &Arc<Mutex<Vec<BtEvent>>>, name: String, connected: bool| {
        if name.trim().is_empty() {
            return;
        }
        out.lock().unwrap_or_else(|e| e.into_inner()).push(BtEvent { name, connected });
        unsafe { PostMessageW(hwnd, WM_APP_BT, 0, 0) };
    };
    {
        let (ready, names, out) = (ready.clone(), names.clone(), out.clone());
        w.Added(&TypedEventHandler::new(move |_, info: &Option<DeviceInformation>| {
            if let Some(i) = info {
                let (id, name) = (i.Id()?.to_string(), i.Name()?.to_string());
                names.lock().unwrap_or_else(|e| e.into_inner()).insert(id, name.clone());
                if ready.load(Ordering::Relaxed) {
                    publish(&out, name, true);
                }
            }
            Ok(())
        }))?;
    }
    {
        let (ready, names, out) = (ready.clone(), names.clone(), out.clone());
        w.Removed(&TypedEventHandler::new(move |_, upd: &Option<DeviceInformationUpdate>| {
            if let Some(u) = upd {
                let name = names.lock().unwrap_or_else(|e| e.into_inner()).remove(&u.Id()?.to_string());
                if let Some(name) = name.filter(|_| ready.load(Ordering::Relaxed)) {
                    publish(&out, name, false);
                }
            }
            Ok(())
        }))?;
    }
    // A watcher only raises Added / Removed when Updated is handled too.
    w.Updated(&TypedEventHandler::new(|_, _: &Option<DeviceInformationUpdate>| Ok(())))?;
    w.EnumerationCompleted(&TypedEventHandler::new(move |_, _: &Option<IInspectable>| {
        ready.store(true, Ordering::Relaxed);
        Ok(())
    }))?;
    w.Start()
}

/// Posts `WM_APP_BT` and queues a `BtEvent` when a Bluetooth (classic or LE) device connects
/// or disconnects.
pub fn spawn_bluetooth(hwnd: HANDLE) -> Arc<Mutex<Vec<BtEvent>>> {
    let shared: Arc<Mutex<Vec<BtEvent>>> = Arc::default();
    let out = shared.clone();
    let _ = std::thread::Builder::new().name("bluetooth".into()).stack_size(256 << 10).spawn(move || {
        co_init();
        let selectors = [
            BluetoothDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected),
            BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected),
        ];
        let mut watchers = Vec::new();
        for sel in selectors.into_iter().flatten() {
            // The selectors describe association endpoints; ask for that kind explicitly.
            let none = IIterable::<HSTRING>::try_from(Vec::<HSTRING>::new());
            let w = none.and_then(|props| {
                DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
                    &sel,
                    &props,
                    DeviceInformationKind::AssociationEndpoint,
                )
            });
            if let Ok(w) = w {
                if watch_bt(&w, hwnd, &out).is_ok() {
                    watchers.push(w);
                }
            }
        }
        if watchers.is_empty() {
            return; // no Bluetooth stack
        }
        loop {
            std::thread::park(); // keeps the watchers alive; their events run on the thread pool
        }
    });
    shared
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_names() {
        assert_eq!(consent_app_name("C:#Program Files#Zoom#bin#Zoom.exe"), "Zoom");
        assert_eq!(consent_app_name("C:#Users#me#AppData#Local#Discord#app-1.0#Discord.EXE"), "Discord");
        assert_eq!(consent_app_name("Microsoft.WindowsCamera_8wekyb3d8bbwe"), "WindowsCamera");
    }

    #[test]
    fn consent_in_use_rules() {
        assert!(consent_active(Some(133_000_000_000_000_000), Some(0)));
        assert!(!consent_active(Some(133_000_000_000_000_000), Some(133_000_000_000_000_001)));
        assert!(!consent_active(Some(0), Some(0)));
        assert!(!consent_active(None, None));
    }
}
