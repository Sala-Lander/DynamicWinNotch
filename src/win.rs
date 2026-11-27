//! The Win32 side: the layered overlay window, the DIB surface it is presented from, the
//! thread-local event queue the window procedure feeds, the tray icon / menu and autostart.

use crate::ffi::*;
use crate::layout::{WIN_H, WIN_W};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const WM_APP_MEDIA: u32 = WM_APP + 1;
pub const WM_APP_CLIP: u32 = WM_APP + 2;
pub const WM_APP_TRAY: u32 = WM_APP + 3;
pub const WM_APP_NOTIFY: u32 = WM_APP + 4;
/// wParam = volume 0..=100, lParam = muted.
pub const WM_APP_VOLUME: u32 = WM_APP + 5;
pub const WM_APP_PRIVACY: u32 = WM_APP + 6;
pub const WM_APP_BT: u32 = WM_APP + 7;
pub const TIMER_ID: usize = 1;
const HOTKEY_ID: i32 = 1;

// ------------------------------------------------------------------ events
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Left,
    Right,
    Middle,
}

/// Everything the window procedure hands to the application. The procedure never touches
/// application state itself; the main loop drains this queue, so there is no re-entrancy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ev {
    /// The pointer moved somewhere (raw input or a client-area move). Position is re-read on demand.
    Mouse,
    /// Button pressed at client pixel (x, y).
    Down(Btn, i32, i32),
    /// Wheel notches in 1/120 units, positive = away from the user.
    Wheel(i32),
    MediaChanged,
    /// The OS says the clipboard content changed.
    ClipUpdate,
    /// The clipboard worker has text for us.
    ClipReady,
    /// Mouse message sent by the tray icon (WM_LBUTTONUP / WM_RBUTTONUP).
    Tray(u32),
    Hotkey,
    DisplayChange,
    TaskbarCreated,
    /// Charger plugged / unplugged or the battery level changed.
    Power,
    /// A character typed while the clipboard search box has focus.
    Char(char),
    /// A control key relevant to text entry: VK_BACK, VK_ESCAPE or VK_RETURN.
    Key(usize),
    /// The window lost keyboard focus (e.g. the user alt-tabbed away): stop "typing".
    LostFocus,
    /// A fresh toast notification, if notification-listener access was granted.
    Notify,
    /// Laptop panel brightness changed (0..=100).
    Brightness(u32),
    /// Master volume changed: (0..=100, muted).
    Volume(u32, bool),
    /// Caps Lock (VK_CAPITAL) or Num Lock (VK_NUMLOCK) was pressed.
    Lock(u16),
    /// Microphone / camera use changed (see `devices::spawn_privacy`).
    Privacy,
    /// A Bluetooth device connected or disconnected (see `devices::spawn_bluetooth`).
    Bluetooth,
    /// A file drag entered / left the island (see the OLE drop target below).
    DragEnter,
    DragLeave,
    /// Files were dropped on the island; fetch them with `take_dropped()`.
    Dropped,
}

thread_local! {
    static QUEUE: RefCell<VecDeque<Ev>> = const { RefCell::new(VecDeque::new()) };
}
static TASKBAR_MSG: AtomicU32 = AtomicU32::new(0);
/// Lock keys currently held (bit 0 = Caps, bit 1 = Num), so auto-repeat does not peek again.
static LOCK_HELD: AtomicU32 = AtomicU32::new(0);

/// `Some(vk)` on the first key-down of Caps / Num Lock; key-ups and repeats give `None`.
pub fn lock_edge(held: &mut u32, vk: u16, msg: u32) -> Option<u16> {
    let bit = match vk {
        VK_CAPITAL => 1,
        VK_NUMLOCK => 2,
        _ => return None,
    };
    if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
        let first = *held & bit == 0;
        *held |= bit;
        first.then_some(vk)
    } else {
        *held &= !bit;
        None
    }
}

// ------------------------------------------------------------------ shelf: OLE drag and drop
//
// Dropping: the window is registered as an OLE drop target (hand-written COM object, no crate).
// It accepts file drags (CF_HDROP) only, always as a *copy*, so the source never deletes anything.
// While click-through the window is invisible to OLE, so `App` makes it hit-testable as soon as
// the pointer is over the island with the button held (see `App::update`).
//
// Dragging out: a shell data object for the file (`SHCreateDataObject`) plus a tiny drop source
// run inside `DoDragDrop`, which blocks until the user drops or cancels.
thread_local! {
    static DROPPED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}
/// The drag now over the window carries files.
static DRAG_OK: AtomicBool = AtomicBool::new(false);
/// A drag *out of* the shelf is running: the shelf must not accept its own files back.
static DRAGGING_OUT: AtomicBool = AtomicBool::new(false);

/// Paths of the last drop (empties the list).
pub fn take_dropped() -> Vec<String> {
    DROPPED.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

/// Is the left mouse button down right now (anywhere on screen)?
pub fn lmb_down() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON) < 0 }
}

#[repr(C)]
struct DropTargetVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    drag_enter: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINTL, *mut u32) -> i32,
    drag_over: unsafe extern "system" fn(*mut c_void, u32, POINTL, *mut u32) -> i32,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> i32,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINTL, *mut u32) -> i32,
}

#[repr(C)]
struct DropSourceVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    query_continue_drag: unsafe extern "system" fn(*mut c_void, i32, u32) -> i32,
    give_feedback: unsafe extern "system" fn(*mut c_void, u32) -> i32,
}

/// A COM object is just a pointer to its vtable; ours are immortal statics (no refcounting).
#[repr(C)]
struct ComObj<T>(*const T);
unsafe impl<T> Sync for ComObj<T> {}

unsafe extern "system" fn com_add_ref(_: *mut c_void) -> u32 {
    1
}
unsafe extern "system" fn com_release(_: *mut c_void) -> u32 {
    1
}

unsafe fn com_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void, own: &GUID) -> i32 {
    if !riid.is_null() && (*riid == IID_IUNKNOWN || *riid == *own) {
        *out = this;
        S_OK
    } else {
        *out = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn dt_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> i32 {
    com_query(this, riid, out, &IID_IDROPTARGET)
}

/// Does the data object offer CF_HDROP (a list of files)?
unsafe fn has_files(data: *mut c_void) -> bool {
    if data.is_null() {
        return false;
    }
    type QueryGetData = unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> i32;
    let vtbl = *(data as *const *const usize);
    let query: QueryGetData = std::mem::transmute(*vtbl.add(5)); // IDataObject::QueryGetData
    let fmt = FORMATETC { cfFormat: CF_HDROP as u16, ptd: null_mut(), dwAspect: DVASPECT_CONTENT, lindex: -1, tymed: TYMED_HGLOBAL };
    query(data, &fmt) == S_OK
}

/// The file paths inside a CF_HDROP data object.
unsafe fn read_files(data: *mut c_void) -> Vec<String> {
    let mut out = Vec::new();
    if data.is_null() {
        return out;
    }
    type GetData = unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> i32;
    let vtbl = *(data as *const *const usize);
    let get: GetData = std::mem::transmute(*vtbl.add(3)); // IDataObject::GetData
    let fmt = FORMATETC { cfFormat: CF_HDROP as u16, ptd: null_mut(), dwAspect: DVASPECT_CONTENT, lindex: -1, tymed: TYMED_HGLOBAL };
    let mut med = STGMEDIUM { tymed: 0, handle: null_mut(), pUnkForRelease: null_mut() };
    if get(data, &fmt, &mut med) >= 0 {
        let hdrop = med.handle as isize;
        let n = DragQueryFileW(hdrop, u32::MAX, null_mut(), 0);
        for i in 0..n {
            let len = DragQueryFileW(hdrop, i, null_mut(), 0) as usize;
            let mut buf = vec![0u16; len + 1];
            let got = DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32) as usize;
            if got > 0 {
                out.push(String::from_utf16_lossy(&buf[..got.min(len)]));
            }
        }
        ReleaseStgMedium(&mut med);
    }
    out
}

unsafe extern "system" fn dt_drag_enter(_: *mut c_void, data: *mut c_void, _key: u32, _pt: POINTL, effect: *mut u32) -> i32 {
    let ok = !DRAGGING_OUT.load(Ordering::Relaxed) && has_files(data);
    DRAG_OK.store(ok, Ordering::Relaxed);
    if !effect.is_null() {
        *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
    }
    if ok {
        push(Ev::DragEnter);
    }
    S_OK
}

unsafe extern "system" fn dt_drag_over(_: *mut c_void, _key: u32, _pt: POINTL, effect: *mut u32) -> i32 {
    if !effect.is_null() {
        *effect = if DRAG_OK.load(Ordering::Relaxed) { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
    }
    S_OK
}

unsafe extern "system" fn dt_drag_leave(_: *mut c_void) -> i32 {
    if DRAG_OK.swap(false, Ordering::Relaxed) {
        push(Ev::DragLeave);
    }
    S_OK
}

unsafe extern "system" fn dt_drop(_: *mut c_void, data: *mut c_void, _key: u32, _pt: POINTL, effect: *mut u32) -> i32 {
    let ok = DRAG_OK.swap(false, Ordering::Relaxed);
    if !effect.is_null() {
        *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
    }
    if ok {
        let files = read_files(data);
        if !files.is_empty() {
            DROPPED.with(|d| d.borrow_mut().extend(files));
        }
        push(Ev::Dropped);
    }
    S_OK
}

static DROP_TARGET_VTBL: DropTargetVtbl = DropTargetVtbl {
    query_interface: dt_query,
    add_ref: com_add_ref,
    release: com_release,
    drag_enter: dt_drag_enter,
    drag_over: dt_drag_over,
    drag_leave: dt_drag_leave,
    drop: dt_drop,
};
static DROP_TARGET: ComObj<DropTargetVtbl> = ComObj(&DROP_TARGET_VTBL);

unsafe extern "system" fn ds_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> i32 {
    com_query(this, riid, out, &IID_IDROPSOURCE)
}

/// Esc cancels, releasing the left button drops, anything else keeps dragging.
unsafe extern "system" fn ds_query_continue(_: *mut c_void, escape: i32, keys: u32) -> i32 {
    if escape != 0 {
        DRAGDROP_S_CANCEL
    } else if keys & MK_LBUTTON == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn ds_give_feedback(_: *mut c_void, _effect: u32) -> i32 {
    DRAGDROP_S_USEDEFAULTCURSORS
}

static DROP_SOURCE_VTBL: DropSourceVtbl = DropSourceVtbl {
    query_interface: ds_query,
    add_ref: com_add_ref,
    release: com_release,
    query_continue_drag: ds_query_continue,
    give_feedback: ds_give_feedback,
};
static DROP_SOURCE: ComObj<DropSourceVtbl> = ComObj(&DROP_SOURCE_VTBL);

/// Drag `path` out of the island into whatever app is under the pointer. Blocks until the user
/// drops or cancels (the left button must be down). Returns false if the drag could not start.
pub fn drag_out(path: &str) -> bool {
    unsafe {
        let name = wide(path);
        let mut pidl: *mut c_void = null_mut();
        if SHParseDisplayName(name.as_ptr(), null_mut(), &mut pidl, 0, null_mut()) < 0 || pidl.is_null() {
            return false;
        }
        let list = [pidl as *const c_void];
        let mut data: *mut c_void = null_mut();
        let hr = SHCreateDataObject(null(), 1, list.as_ptr(), null_mut(), &IID_IDATAOBJECT, &mut data);
        ILFree(pidl);
        if hr < 0 || data.is_null() {
            return false;
        }
        DRAGGING_OUT.store(true, Ordering::Relaxed);
        let mut effect = 0u32;
        // Copy only: dragging out of the shelf never moves or deletes the original.
        DoDragDrop(data, &DROP_SOURCE as *const _ as *mut c_void, DROPEFFECT_COPY, &mut effect);
        DRAGGING_OUT.store(false, Ordering::Relaxed);
        // IUnknown::Release (vtable slot 2) on the data object we created.
        type Release = unsafe extern "system" fn(*mut c_void) -> u32;
        let vtbl = *(data as *const *const usize);
        let release: Release = std::mem::transmute(*vtbl.add(2));
        release(data);
        true
    }
}

/// Raw input: keyboard packets only matter for the lock keys; anything else is pointer movement.
unsafe fn on_raw_input(lp: isize) {
    let mut buf = [0u64; 8];
    let mut size = std::mem::size_of_val(&buf) as u32;
    let hs = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let n = GetRawInputData(lp, RID_INPUT, buf.as_mut_ptr() as *mut c_void, &mut size, hs);
    let hdr = &*(buf.as_ptr() as *const RAWINPUTHEADER);
    if n != u32::MAX && n >= hs + 16 && hdr.dwType == RIM_TYPEKEYBOARD {
        let kb = &*((buf.as_ptr() as *const u8).add(hs as usize) as *const RAWKEYBOARD);
        let mut held = LOCK_HELD.load(Ordering::Relaxed);
        let edge = lock_edge(&mut held, kb.VKey, kb.Message);
        LOCK_HELD.store(held, Ordering::Relaxed);
        if let Some(vk) = edge {
            push(Ev::Lock(vk));
        }
    } else {
        push(Ev::Mouse);
    }
}

pub fn push(ev: Ev) {
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        // Raw input can fire at 1 kHz; one pending "pointer moved" is all anybody needs.
        if ev == Ev::Mouse && q.back() == Some(&Ev::Mouse) {
            return;
        }
        q.push_back(ev);
    });
}

pub fn pop() -> Option<Ev> {
    QUEUE.with(|q| q.borrow_mut().pop_front())
}

unsafe extern "system" fn wndproc(hwnd: HANDLE, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        // Never steal focus from the app the user is working in.
        WM_MOUSEACTIVATE => MA_NOACTIVATE,
        // Raw mouse input (registered with RIDEV_INPUTSINK) keeps arriving while the window is
        // click-through; this is how hover is detected with zero polling. DefWindowProc must
        // still run so the system can free the raw-input buffer.
        WM_INPUT => {
            on_raw_input(lp);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_MOUSEMOVE => {
            push(Ev::Mouse);
            0
        }
        WM_LBUTTONDOWN => {
            push(Ev::Down(Btn::Left, loword(lp), hiword(lp)));
            0
        }
        WM_RBUTTONDOWN => {
            push(Ev::Down(Btn::Right, loword(lp), hiword(lp)));
            0
        }
        WM_MBUTTONDOWN => {
            push(Ev::Down(Btn::Middle, loword(lp), hiword(lp)));
            0
        }
        WM_MOUSEWHEEL => {
            push(Ev::Wheel(hiword(wp as isize)));
            0
        }
        WM_CLIPBOARDUPDATE => {
            push(Ev::ClipUpdate);
            0
        }
        WM_APP_MEDIA => {
            push(Ev::MediaChanged);
            0
        }
        WM_APP_CLIP => {
            push(Ev::ClipReady);
            0
        }
        WM_APP_TRAY => {
            push(Ev::Tray((lp as u32) & 0xFFFF));
            0
        }
        WM_HOTKEY => {
            push(Ev::Hotkey);
            0
        }
        WM_POWERBROADCAST => {
            if wp == PBT_APMPOWERSTATUSCHANGE {
                push(Ev::Power);
            } else if wp == PBT_POWERSETTINGCHANGE && lp != 0 {
                // Data is a variable-length tail behind the struct: read it through the raw pointer.
                let setting = lp as *const POWERBROADCAST_SETTING;
                if (*setting).PowerSetting == GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS && (*setting).DataLength >= 4 {
                    let pct = std::ptr::addr_of!((*setting).Data).cast::<u32>().read_unaligned();
                    push(Ev::Brightness(pct.min(100)));
                }
            }
            1 // TRUE: the broadcast is handled
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wp as u32) {
                if !c.is_control() {
                    push(Ev::Char(c));
                }
            }
            0
        }
        WM_KEYDOWN => {
            if matches!(wp, VK_BACK | VK_ESCAPE | VK_RETURN) {
                push(Ev::Key(wp));
            }
            0
        }
        WM_KILLFOCUS => {
            push(Ev::LostFocus);
            0
        }
        WM_APP_NOTIFY => {
            push(Ev::Notify);
            0
        }
        WM_APP_VOLUME => {
            push(Ev::Volume(wp.min(100) as u32, lp != 0));
            0
        }
        WM_APP_PRIVACY => {
            push(Ev::Privacy);
            0
        }
        WM_APP_BT => {
            push(Ev::Bluetooth);
            0
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            push(Ev::DisplayChange);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => {
            let tb = TASKBAR_MSG.load(Ordering::Relaxed);
            if tb != 0 && msg == tb {
                push(Ev::TaskbarCreated);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

// ----------------------------------------------------------------- process
pub fn already_running() -> bool {
    unsafe {
        // The handle is intentionally leaked: the mutex must live as long as the process.
        let _h = CreateMutexW(null(), 0, wide("Local\\NotchIsland.Singleton").as_ptr());
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

/// Per-monitor-v2 DPI awareness, so the pill is crisp at 125/150/200 % scaling.
pub fn set_dpi_aware() {
    unsafe {
        SetProcessDpiAwarenessContext(-4); // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
    }
}

/// Drop the process working set; Windows pages back only what is actually touched.
pub fn trim_memory() {
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

/// FILETIME ticks (100 ns since 1601) right now.
pub fn filetime_now() -> i64 {
    let mut ft = FILETIME::default();
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    ft.as_u64() as i64
}

/// True when a full-screen app, game or presentation owns the screen.
pub fn fullscreen_app_active() -> bool {
    let mut st = 0i32;
    let hr = unsafe { SHQueryUserNotificationState(&mut st) };
    hr >= 0 && matches!(st, QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
}

/// Toggle state of Caps / Num Lock as the foreground app sees it. `GetKeyState` only reflects
/// the calling thread's input state, so the read borrows the foreground thread's for a moment.
pub fn lock_key_on(vk: u16) -> bool {
    unsafe {
        let me = GetCurrentThreadId();
        let fg = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
        let attached = fg != 0 && fg != me && AttachThreadInput(me, fg, 1) != 0;
        let on = GetKeyState(vk as i32) & 1 != 0;
        if attached {
            AttachThreadInput(me, fg, 0);
        }
        on
    }
}

/// Start / focus an app by its App User Model ID (what a toast notification carries).
pub fn open_app(aumid: &str) {
    if aumid.is_empty() || aumid.contains('"') {
        return;
    }
    let args = wide(&format!("shell:AppsFolder\\{aumid}"));
    unsafe {
        ShellExecuteW(0, wide("open").as_ptr(), wide("explorer.exe").as_ptr(), args.as_ptr(), null(), SW_SHOWNORMAL);
    }
}

pub fn open_in_editor(path: &std::path::Path) {
    let file = wide(&path.to_string_lossy());
    unsafe {
        ShellExecuteW(0, wide("open").as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

// ------------------------------------------------------------------ autostart
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "NotchIsland";

pub fn autostart_enabled() -> bool {
    let (k, v) = (wide(RUN_KEY), wide(RUN_VALUE));
    let mut cb = 0u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut cb) == 0 }
}

pub fn set_autostart(on: bool) {
    let (k, v) = (wide(RUN_KEY), wide(RUN_VALUE));
    unsafe {
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let data = wide(&format!("\"{}\"", exe.display()));
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    k.as_ptr(),
                    v.as_ptr(),
                    REG_SZ,
                    data.as_ptr() as *const c_void,
                    (data.len() * 2) as u32,
                );
            }
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr());
        }
    }
}

// -------------------------------------------------------------------- surface
/// A top-down 32-bit BGRA DIB section selected into a memory DC; the source for UpdateLayeredWindow.
struct Surface {
    dc: HANDLE,
    bmp: HANDLE,
    old: HANDLE,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Surface {
    fn new(w: i32, h: i32) -> Option<Surface> {
        unsafe {
            let dc = CreateCompatibleDC(0);
            if dc == 0 {
                return None;
            }
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h; // negative = top-down rows
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut c_void = null_mut();
            let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, 0, 0);
            if bmp == 0 || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bmp);
            Some(Surface { dc, bmp, old, bits: bits as *mut u32, w, h })
        }
    }

    /// Premultiplied RGBA (tiny-skia) -> premultiplied BGRA (GDI layered window).
    fn upload(&mut self, rgba: &[u8]) {
        let n = (self.w * self.h) as usize;
        if rgba.len() < n * 4 {
            return;
        }
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, n) };
        for (d, s) in dst.iter_mut().zip(rgba.chunks_exact(4)) {
            let p = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            *d = (p & 0xFF00_FF00) | ((p & 0x0000_00FF) << 16) | ((p >> 16) & 0x0000_00FF);
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}

// --------------------------------------------------------------------- window
pub struct Win {
    pub hwnd: HANDLE,
    surf: Option<Surface>,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// Screen position of the window's top-left corner, physical pixels.
    pub origin: (i32, i32),
    pub size_px: (i32, i32),
    through: bool,
    visible: bool,
}

impl Win {
    pub fn create() -> Option<Win> {
        unsafe {
            let hinst = GetModuleHandleW(null());
            let class = wide("NotchIslandOverlay");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinst,
                hIcon: 0,
                hCursor: LoadCursorW(0, IDC_ARROW as *const u16),
                hbrBackground: 0,
                lpszMenuName: null(),
                lpszClassName: class.as_ptr(),
                hIconSm: 0,
            };
            if RegisterClassExW(&wc) == 0 {
                return None;
            }
            TASKBAR_MSG.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);

            // Layered (per-pixel alpha) + topmost + tool window (no taskbar button, no Alt-Tab)
            // + no-activate + transparent (click-through until the pointer reaches the pill).
            let ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
            let hwnd = CreateWindowExW(
                ex,
                class.as_ptr(),
                wide("Notch Island").as_ptr(),
                WS_POPUP,
                0,
                0,
                WIN_W as i32,
                WIN_H as i32,
                0,
                0,
                hinst,
                null_mut(),
            );
            if hwnd == 0 {
                return None;
            }

            // Pointer movement while click-through: raw mouse input delivered even in the background.
            let rid = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
            RegisterRawInputDevices(&rid, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
            // Panel brightness changes arrive as WM_POWERBROADCAST (no-op on desktop monitors).
            RegisterPowerSettingNotification(hwnd, &GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, DEVICE_NOTIFY_WINDOW_HANDLE);

            let mut w = Win { hwnd, surf: None, scale: 1.0, origin: (0, 0), size_px: (0, 0), through: true, visible: true };
            w.reposition();
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Some(w)
        }
    }

    /// Recompute DPI scale, size and the top-centre position on the primary monitor.
    /// Returns true when the pixel size changed (the caller must resize its canvas).
    pub fn reposition(&mut self) -> bool {
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd);
            let dpi = if dpi == 0 { GetDpiForSystem().max(96) } else { dpi };
            let scale = dpi as f32 / 96.0;
            let (w, h) = ((WIN_W * scale).round() as i32, (WIN_H * scale).round() as i32);

            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let (x, y) = if GetMonitorInfoW(mon, &mut mi) != 0 {
                (mi.rcMonitor.left + ((mi.rcMonitor.right - mi.rcMonitor.left) - w) / 2, mi.rcMonitor.top)
            } else {
                (0, 0)
            };

            let changed = self.size_px != (w, h) || self.surf.is_none();
            self.scale = scale;
            self.origin = (x, y);
            self.size_px = (w, h);
            if changed {
                self.surf = Surface::new(w, h);
            }
            SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
            changed
        }
    }

    /// Cursor position in logical pixels relative to the window's top-left corner.
    pub fn cursor_logical(&self) -> (f32, f32) {
        let (cx, cy) = cursor_pos();
        ((cx - self.origin.0) as f32 / self.scale, (cy - self.origin.1) as f32 / self.scale)
    }

    /// Toggle WS_EX_TRANSPARENT: while set, every mouse event falls through to the windows below.
    pub fn set_click_through(&mut self, on: bool) {
        if on == self.through {
            return;
        }
        self.through = on;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let new = if on { ex | WS_EX_TRANSPARENT as isize } else { ex & !(WS_EX_TRANSPARENT as isize) };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, new);
        }
    }

    pub fn set_visible(&mut self, on: bool) {
        if on != self.visible {
            self.visible = on;
            unsafe { ShowWindow(self.hwnd, if on { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Re-assert topmost (full-screen apps and other topmost windows can bury us).
    pub fn raise(&self) {
        unsafe {
            SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// Push a finished frame (premultiplied RGBA, `size_px` big) to the screen.
    pub fn present(&mut self, rgba: &[u8]) {
        let Some(surf) = self.surf.as_mut() else { return };
        surf.upload(rgba);
        let pt_dst = POINT { x: self.origin.0, y: self.origin.1 };
        let size = SIZE { cx: surf.w, cy: surf.h };
        let pt_src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA };
        unsafe {
            UpdateLayeredWindow(self.hwnd, 0, &pt_dst, &size, surf.dc, &pt_src, 0, &blend, ULW_ALPHA);
        }
    }

    pub fn arm_timer(&self, ms: u32) {
        unsafe { SetTimer(self.hwnd, TIMER_ID, ms.max(10), null()) };
    }

    pub fn disarm_timer(&self) {
        unsafe { KillTimer(self.hwnd, TIMER_ID) };
    }

    /// Raw keyboard input (background too) for the Caps / Num Lock peek; only while it is on.
    pub fn watch_lock_keys(&self, on: bool) {
        let rid = RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: 6,
            dwFlags: if on { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
            hwndTarget: if on { self.hwnd } else { 0 },
        };
        unsafe { RegisterRawInputDevices(&rid, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) };
    }

    pub fn register_hotkey(&self, on: bool) {
        unsafe {
            UnregisterHotKey(self.hwnd, HOTKEY_ID);
            if on {
                RegisterHotKey(self.hwnd, HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, b'N' as u32);
            }
        }
    }

    pub fn listen_clipboard(&self) {
        unsafe { AddClipboardFormatListener(self.hwnd) };
    }

    /// Briefly take keyboard focus so the clipboard search box can receive typed characters.
    /// The window is otherwise `WS_EX_NOACTIVATE` (it never steals focus on its own), so this is
    /// a deliberate exception: clicking the search box visibly takes over the keyboard the way a
    /// Spotlight / PowerToys-Run style search field does, until Escape, Enter or a click
    /// elsewhere, or until the user switches away (which sends `WM_KILLFOCUS` -> `Ev::LostFocus`).
    pub fn focus_for_typing(&self) {
        unsafe {
            SetForegroundWindow(self.hwnd);
            SetFocus(self.hwnd);
        }
    }

    /// Make the window a target for file drops (OLE). Must run on the thread that owns the window.
    pub fn register_drop_target(&self) {
        unsafe {
            if OleInitialize(null_mut()) >= 0 {
                RegisterDragDrop(self.hwnd, &DROP_TARGET as *const _ as *mut c_void);
            }
        }
    }

    pub fn destroy(&mut self) {
        unsafe {
            RevokeDragDrop(self.hwnd);
            OleUninitialize();
            UnregisterHotKey(self.hwnd, HOTKEY_ID);
            RemoveClipboardFormatListener(self.hwnd);
            DestroyWindow(self.hwnd);
        }
        self.surf = None;
    }
}

// ----------------------------------------------------------------------- tray
/// A 32x32 icon drawn at start-up: a black pill with a white rim and an accent dot.
fn make_icon() -> HANDLE {
    use crate::gfx::{circle, rgba, rrect};
    use crate::layout::Rect;
    use tiny_skia::{FillRule, Paint, Pixmap, Stroke, Transform};

    let Some(mut pm) = Pixmap::new(32, 32) else { return 0 };
    let pill = Rect::new(2.5, 9.5, 27.0, 13.0);
    if let Some(p) = rrect(pill, 6.5) {
        let mut paint = Paint::default();
        paint.set_color(rgba((0, 0, 0), 1.0));
        paint.anti_alias = true;
        pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
        paint.set_color(rgba((255, 255, 255), 1.0));
        let st = Stroke { width: 1.5, ..Stroke::default() };
        pm.stroke_path(&p, &paint, &st, Transform::identity(), None);
    }
    if let Some(p) = circle(22.5, 16.0, 2.4) {
        let mut paint = Paint::default();
        paint.set_color(rgba((122, 168, 255), 1.0));
        paint.anti_alias = true;
        pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
    }
    // RGBA -> BGRA, top-down rows.
    let mut xor = Vec::with_capacity(32 * 32 * 4);
    for px in pm.data().chunks_exact(4) {
        xor.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    let and = vec![0u8; 32 * 4]; // 1 bpp mask, 4 bytes per row, all zero = use the colour bitmap alpha
    unsafe { CreateIcon(GetModuleHandleW(null()), 32, 32, 1, 32, and.as_ptr(), xor.as_ptr()) }
}

pub struct Tray {
    hwnd: HANDLE,
    icon: HANDLE,
}

impl Tray {
    pub fn add(hwnd: HANDLE) -> Tray {
        let t = Tray { hwnd, icon: make_icon() };
        t.notify(NIM_ADD);
        t
    }

    /// Explorer restarted: the icon vanished and must be added again.
    pub fn re_add(&self) {
        self.notify(NIM_ADD);
    }

    fn notify(&self, msg: u32) {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: self.icon,
            szTip: [0; 128],
            dwState: 0,
            dwStateMask: 0,
            szInfo: [0; 256],
            uVersion: 0,
            szInfoTitle: [0; 64],
            dwInfoFlags: 0,
            guidItem: [0; 4],
            hBalloonIcon: 0,
        };
        fill_wide(&mut nid.szTip, "Notch Island");
        unsafe { Shell_NotifyIconW(msg, &mut nid) };
    }

    pub fn remove(&self) {
        self.notify(NIM_DELETE);
        if self.icon != 0 {
            unsafe { DestroyIcon(self.icon) };
        }
    }
}

/// Show a popup menu at the cursor and return the chosen command id (0 = dismissed).
/// Items are `(id, label, checked)`; `id == 0` makes a separator.
pub fn popup_menu(hwnd: HANDLE, items: &[(usize, &str, bool)]) -> usize {
    unsafe {
        let menu = CreatePopupMenu();
        if menu == 0 {
            return 0;
        }
        for &(id, label, checked) in items {
            if id == 0 {
                AppendMenuW(menu, MF_SEPARATOR, 0, null());
            } else {
                let text = wide(label);
                AppendMenuW(menu, MF_STRING | if checked { MF_CHECKED } else { 0 }, id, text.as_ptr());
            }
        }
        let (x, y) = cursor_pos();
        // The owner must be foreground or the menu will not close when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD, x, y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd.max(0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_keys_peek_once_per_press() {
        let mut held = 0;
        assert_eq!(lock_edge(&mut held, VK_CAPITAL, WM_KEYDOWN), Some(VK_CAPITAL));
        assert_eq!(lock_edge(&mut held, VK_CAPITAL, WM_KEYDOWN), None); // auto-repeat
        assert_eq!(lock_edge(&mut held, VK_NUMLOCK, WM_KEYDOWN), Some(VK_NUMLOCK));
        assert_eq!(lock_edge(&mut held, VK_CAPITAL, WM_KEYUP), None);
        assert_eq!(lock_edge(&mut held, VK_CAPITAL, WM_KEYDOWN), Some(VK_CAPITAL));
        assert_eq!(lock_edge(&mut held, 0x41, WM_KEYDOWN), None); // 'A' is never looked at
    }
}
