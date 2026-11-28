//! The Win32 side: the layered overlay window, the DIB surface it is presented from, the
//! thread-local event queue the window procedure feeds, the tray icon / menu and autostart.

use crate::ffi::*;
use crate::layout::{WIN_H, WIN_W};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

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
    /// Horizontal wheel / touchpad swipe, same units as `Wheel` (positive = right).
    HWheel(i32),
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
// Dragging out: a hand-written `IDataObject` carrying a real `CF_HDROP` (a `DROPFILES` struct in
// global memory, the same format Explorer and most apps read) plus a tiny drop source, run inside
// `DoDragDrop`, which blocks until the user drops or cancels.
thread_local! {
    static DROPPED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}
/// The drag now over the window carries files.
static DRAG_OK: AtomicBool = AtomicBool::new(false);
/// A drag *out of* the shelf is running: the shelf must not accept its own files back.
static DRAGGING_OUT: AtomicBool = AtomicBool::new(false);
/// The island's own window handle, set once in `register_drop_target`. `SHDoDragDrop` wants an
/// owner HWND to anchor the shell's drag helper window to; 0 until the window exists.
static OWNER_HWND: AtomicIsize = AtomicIsize::new(0);

/// Drag/drop diagnostics, switched off now that drag-out is confirmed working: kept as a no-op
/// (instead of ripping out every call site) in case it's needed again. Used to append one line to
/// `%APPDATA%\NotchIsland\drag.log`, restarting the file once it passes 256 KB.
fn drag_log(_msg: &str) {}

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
    drag_log(&format!("island drag_enter files={ok} out_running={}", DRAGGING_OUT.load(Ordering::Relaxed)));
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
    drag_log(&format!("island drop accepted={ok}"));
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
        drag_log("out: cancelled (Esc)");
        DRAGDROP_S_CANCEL
    } else if keys & MK_LBUTTON == 0 {
        drag_log("out: button released -> drop");
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

/// The last effect the target offered (`u32::MAX` = none yet). Logged only when it changes.
static LAST_EFFECT: AtomicU32 = AtomicU32::new(u32::MAX);

unsafe extern "system" fn ds_give_feedback(_: *mut c_void, effect: u32) -> i32 {
    if LAST_EFFECT.swap(effect, Ordering::Relaxed) != effect {
        // 0 = no target accepts the drop here, 1 = copy: the target under the pointer decides this.
        drag_log(&format!("out: target effect={effect}"));
    }
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

// ---- a real IDataObject carrying CF_HDROP --------------------------------------------------
//
// `SHCreateDataObject` turned out to hand out a data object some drop targets (Explorer's view,
// several browsers) do not fully accept: `DoDragDrop` reported `DRAGDROP_S_DROP` but the target's
// offered effect was "scroll", never "copy", so nothing landed (see drag.log). A hand-built
// `CF_HDROP` - the same bytes `GetOpenFileName`/Explorer itself produces - is the one format every
// Windows app that accepts dropped files is written to read, so we build that directly instead.
#[repr(C)]
struct DataObjectVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> i32,
    get_data_here: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> i32,
    query_get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> i32,
    get_canonical_format_etc: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut FORMATETC) -> i32,
    set_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM, i32) -> i32,
    enum_format_etc: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32,
    d_advise: unsafe extern "system" fn(*mut c_void, *const FORMATETC, u32, *mut c_void, *mut u32) -> i32,
    d_unadvise: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    enum_d_advise: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
}

/// One `FileDataObject` per drag; freed by its own `Release` when the last reference drops.
#[repr(C)]
struct FileDataObject {
    vtbl: *const DataObjectVtbl,
    refs: AtomicU32,
    path: String,
}

const DV_E_FORMATETC: i32 = 0x8004_0064u32 as i32;
const OLE_E_ADVISENOTSUPPORTED: i32 = 0x8004_0003u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const S_FALSE: i32 = 1;

fn is_hdrop(fmt: &FORMATETC) -> bool {
    fmt.cfFormat == CF_HDROP as u16 && (fmt.tymed & TYMED_HGLOBAL) != 0 && fmt.dwAspect == DVASPECT_CONTENT
}

/// A `DROPFILES` header followed by `path` as a double-NUL-terminated wide string, in a global
/// memory block a receiver takes ownership of (and frees with `GlobalFree`, via `ReleaseStgMedium`).
unsafe fn hdrop_global(path: &str) -> *mut c_void {
    let wide_path = wide(path); // already NUL-terminated
    let header = std::mem::size_of::<DROPFILES>();
    let list_bytes = (wide_path.len() + 1) * 2; // + the list's own terminating NUL
    let mem = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, header + list_bytes);
    if mem.is_null() {
        return null_mut();
    }
    let base = GlobalLock(mem);
    if base.is_null() {
        GlobalFree(mem);
        return null_mut();
    }
    let hdr = &mut *(base as *mut DROPFILES);
    hdr.p_files = header as u32;
    hdr.pt_x = 0;
    hdr.pt_y = 0;
    hdr.f_nc = 0;
    hdr.f_wide = 1;
    let dst = (base as *mut u8).add(header) as *mut u16;
    std::ptr::copy_nonoverlapping(wide_path.as_ptr(), dst, wide_path.len());
    *dst.add(wide_path.len()) = 0; // second NUL: ends the (single-entry) list
    GlobalUnlock(mem);
    mem
}

/// The last unsupported interface we logged a QueryInterface miss for (dedup; `data1` is enough
/// to tell interfaces apart here). Reset at the start of every drag.
static LAST_UNK_RIID: AtomicU32 = AtomicU32::new(u32::MAX);

unsafe extern "system" fn fdo_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> i32 {
    if !riid.is_null() && (*riid == IID_IUNKNOWN || *riid == IID_IDATAOBJECT) {
        fdo_add_ref(this);
        *out = this;
        S_OK
    } else {
        *out = null_mut();
        let id = if riid.is_null() { 0 } else { (*riid).data1 };
        if LAST_UNK_RIID.swap(id, Ordering::Relaxed) != id {
            drag_log(&format!("out: data QueryInterface refused riid.data1={id:#010x}"));
        }
        E_NOINTERFACE
    }
}

unsafe extern "system" fn fdo_add_ref(this: *mut c_void) -> u32 {
    (*(this as *mut FileDataObject)).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn fdo_release(this: *mut c_void) -> u32 {
    let obj = this as *mut FileDataObject;
    let left = (*obj).refs.fetch_sub(1, Ordering::Relaxed) - 1;
    if left == 0 {
        drop(Box::from_raw(obj));
    }
    left
}

/// The last refused format we logged (so a target probing 20 formats a frame doesn't flood the
/// log with the same lines); reset at the start of every drag.
static LAST_REFUSED: AtomicU32 = AtomicU32::new(u32::MAX);

unsafe extern "system" fn fdo_get_data(this: *mut c_void, fmt: *const FORMATETC, med: *mut STGMEDIUM) -> i32 {
    let obj = &*(this as *const FileDataObject);
    if fmt.is_null() || !is_hdrop(&*fmt) {
        let cf = if fmt.is_null() { 0 } else { (*fmt).cfFormat } as u32;
        if LAST_REFUSED.swap(cf, Ordering::Relaxed) != cf {
            drag_log(&format!("out: data GetData refused cf={cf}"));
        }
        return DV_E_FORMATETC;
    }
    LAST_REFUSED.store(u32::MAX, Ordering::Relaxed);
    let mem = hdrop_global(&obj.path);
    if mem.is_null() {
        drag_log("out: data GetData: GlobalAlloc failed");
        return E_OUTOFMEMORY;
    }
    *med = STGMEDIUM { tymed: TYMED_HGLOBAL, handle: mem, pUnkForRelease: null_mut() };
    drag_log("out: data GetData CF_HDROP ok");
    S_OK
}

unsafe extern "system" fn fdo_get_data_here(_: *mut c_void, _: *const FORMATETC, _: *mut STGMEDIUM) -> i32 {
    E_NOTIMPL
}

unsafe extern "system" fn fdo_query_get_data(_: *mut c_void, fmt: *const FORMATETC) -> i32 {
    let ok = !fmt.is_null() && is_hdrop(&*fmt);
    let cf = if fmt.is_null() { 0 } else { (*fmt).cfFormat } as u32;
    if ok {
        drag_log(&format!("out: data QueryGetData cf={cf} -> true"));
    } else if LAST_REFUSED.swap(cf, Ordering::Relaxed) != cf {
        drag_log(&format!("out: data QueryGetData cf={cf} -> false"));
    }
    if ok {
        S_OK
    } else {
        DV_E_FORMATETC
    }
}

unsafe extern "system" fn fdo_canonical(_: *mut c_void, _: *const FORMATETC, _: *mut FORMATETC) -> i32 {
    E_NOTIMPL
}
unsafe extern "system" fn fdo_set_data(_: *mut c_void, _: *const FORMATETC, _: *mut STGMEDIUM, _: i32) -> i32 {
    E_NOTIMPL
}
unsafe extern "system" fn fdo_enum_format_etc(_: *mut c_void, direction: u32, out: *mut *mut c_void) -> i32 {
    // Explorer's own shell views (Desktop, folder windows) probe `EnumFormatEtc` at drag-enter to
    // learn what the source offers *before* ever calling `GetData`; a source that only answers
    // `QueryGetData`/`GetData` (as this one used to) looks format-less to them and they silently
    // never ask for CF_HDROP at all - matching logs where the drop target never queries cf=15 and
    // the drag ends in DROPEFFECT_NONE. Simpler targets (a browser's drop handler, an app shortcut's
    // own "open with dropped file") query CF_HDROP directly and never needed this. DATADIR_GET (1)
    // gets the one format we offer; DATADIR_SET (2) gets an empty enumerator, we accept nothing.
    const DATADIR_GET: u32 = 1;
    drag_log(&format!("out: data EnumFormatEtc direction={direction}"));
    if out.is_null() {
        return E_INVALIDARG;
    }
    let obj = Box::into_raw(Box::new(FormatEnum { vtbl: &FORMAT_ENUM_VTBL, refs: AtomicU32::new(1), pos: AtomicU32::new(0), len: if direction == DATADIR_GET { 1 } else { 0 } }));
    *out = obj as *mut c_void;
    S_OK
}

/// One `FORMATETC` entry: `CF_HDROP`/`TYMED_HGLOBAL`/`DVASPECT_CONTENT`, `lindex` -1 (whole object).
fn hdrop_format_etc() -> FORMATETC {
    FORMATETC { cfFormat: CF_HDROP as u16, ptd: null_mut(), dwAspect: DVASPECT_CONTENT, lindex: -1, tymed: TYMED_HGLOBAL }
}

#[repr(C)]
struct EnumVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    next: unsafe extern "system" fn(*mut c_void, u32, *mut FORMATETC, *mut u32) -> i32,
    skip: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    reset: unsafe extern "system" fn(*mut c_void) -> i32,
    clone: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
}

/// `IEnumFORMATETC` over a fixed list of 0 or 1 entries (just `CF_HDROP`). One per `EnumFormatEtc`
/// call (and one more per `Clone`); refcounted and freed the same way as `FileDataObject`.
#[repr(C)]
struct FormatEnum {
    vtbl: *const EnumVtbl,
    refs: AtomicU32,
    pos: AtomicU32,
    len: u32,
}

unsafe extern "system" fn fe_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> i32 {
    if !riid.is_null() && (*riid == IID_IUNKNOWN || *riid == IID_IENUMFORMATETC) {
        fe_add_ref(this);
        *out = this;
        S_OK
    } else {
        *out = null_mut();
        E_NOINTERFACE
    }
}
unsafe extern "system" fn fe_add_ref(this: *mut c_void) -> u32 {
    (*(this as *mut FormatEnum)).refs.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "system" fn fe_release(this: *mut c_void) -> u32 {
    let obj = this as *mut FormatEnum;
    let left = (*obj).refs.fetch_sub(1, Ordering::Relaxed) - 1;
    if left == 0 {
        drop(Box::from_raw(obj));
    }
    left
}
unsafe extern "system" fn fe_next(this: *mut c_void, celt: u32, rg: *mut FORMATETC, fetched: *mut u32) -> i32 {
    let obj = &*(this as *const FormatEnum);
    let mut n = 0u32;
    while n < celt {
        let pos = obj.pos.load(Ordering::Relaxed);
        if pos >= obj.len {
            break;
        }
        obj.pos.store(pos + 1, Ordering::Relaxed);
        if !rg.is_null() {
            *rg.add(n as usize) = hdrop_format_etc();
        }
        n += 1;
    }
    if !fetched.is_null() {
        *fetched = n;
    }
    drag_log(&format!("out: data EnumFormatEtc.Next celt={celt} got={n}"));
    if n == celt {
        S_OK
    } else {
        S_FALSE
    }
}
unsafe extern "system" fn fe_skip(this: *mut c_void, celt: u32) -> i32 {
    let obj = &*(this as *const FormatEnum);
    let pos = obj.pos.load(Ordering::Relaxed);
    let new_pos = pos.saturating_add(celt);
    obj.pos.store(new_pos.min(obj.len), Ordering::Relaxed);
    if new_pos > obj.len {
        S_FALSE
    } else {
        S_OK
    }
}
unsafe extern "system" fn fe_reset(this: *mut c_void) -> i32 {
    (*(this as *const FormatEnum)).pos.store(0, Ordering::Relaxed);
    S_OK
}
unsafe extern "system" fn fe_clone(this: *mut c_void, out: *mut *mut c_void) -> i32 {
    let obj = &*(this as *const FormatEnum);
    let clone = Box::into_raw(Box::new(FormatEnum {
        vtbl: obj.vtbl,
        refs: AtomicU32::new(1),
        pos: AtomicU32::new(obj.pos.load(Ordering::Relaxed)),
        len: obj.len,
    }));
    *out = clone as *mut c_void;
    S_OK
}

static FORMAT_ENUM_VTBL: EnumVtbl =
    EnumVtbl { query_interface: fe_query, add_ref: fe_add_ref, release: fe_release, next: fe_next, skip: fe_skip, reset: fe_reset, clone: fe_clone };
unsafe extern "system" fn fdo_d_advise(_: *mut c_void, _: *const FORMATETC, _: u32, _: *mut c_void, _: *mut u32) -> i32 {
    OLE_E_ADVISENOTSUPPORTED
}
unsafe extern "system" fn fdo_d_unadvise(_: *mut c_void, _: u32) -> i32 {
    OLE_E_ADVISENOTSUPPORTED
}
unsafe extern "system" fn fdo_enum_d_advise(_: *mut c_void, out: *mut *mut c_void) -> i32 {
    if !out.is_null() {
        *out = null_mut();
    }
    OLE_E_ADVISENOTSUPPORTED
}

static DATA_OBJECT_VTBL: DataObjectVtbl = DataObjectVtbl {
    query_interface: fdo_query,
    add_ref: fdo_add_ref,
    release: fdo_release,
    get_data: fdo_get_data,
    get_data_here: fdo_get_data_here,
    query_get_data: fdo_query_get_data,
    get_canonical_format_etc: fdo_canonical,
    set_data: fdo_set_data,
    enum_format_etc: fdo_enum_format_etc,
    d_advise: fdo_d_advise,
    d_unadvise: fdo_d_unadvise,
    enum_d_advise: fdo_enum_d_advise,
};

/// Drag `path` out of the island into whatever app is under the pointer. Blocks until the user
/// drops or cancels (the left button must be down). Returns false if the drag could not start.
pub fn drag_out(path: &str) -> bool {
    unsafe {
        drag_log(&format!("out: start path={path} lmb={}", lmb_down()));
        let obj = Box::into_raw(Box::new(FileDataObject {
            vtbl: &DATA_OBJECT_VTBL,
            refs: AtomicU32::new(1),
            path: path.to_string(),
        }));
        DRAGGING_OUT.store(true, Ordering::Relaxed);
        LAST_EFFECT.store(u32::MAX, Ordering::Relaxed);
        LAST_REFUSED.store(u32::MAX, Ordering::Relaxed);
        LAST_UNK_RIID.store(u32::MAX, Ordering::Relaxed);
        let mut effect = 0u32;
        let owner = OWNER_HWND.load(Ordering::Relaxed);
        // Copy only: dragging out of the shelf never moves or deletes the original.
        // SHDoDragDrop (not the raw ole32 DoDragDrop) is the shell-aware wrapper Explorer expects:
        // it negotiates the shell's private drag-image/helper clipboard formats, which is why a
        // plain DoDragDrop drag got stuck offering DROPEFFECT_SCROLL only and never DROPEFFECT_COPY.
        let hr = SHDoDragDrop(owner, obj as *mut c_void, &DROP_SOURCE as *const _ as *mut c_void, DROPEFFECT_COPY, &mut effect);
        drag_log(&format!("out: SHDoDragDrop owner={owner:#x} hr={:#010x} effect={effect}", hr as u32));
        DRAGGING_OUT.store(false, Ordering::Relaxed);
        fdo_release(obj as *mut c_void); // our own reference; the object frees itself at refcount 0
        hr == DRAGDROP_S_DROP && effect != DROPEFFECT_NONE
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
        WM_MOUSEHWHEEL => {
            push(Ev::HWheel(hiword(wp as isize)));
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
            OWNER_HWND.store(self.hwnd, Ordering::Relaxed);
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
