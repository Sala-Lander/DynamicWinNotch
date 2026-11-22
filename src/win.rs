//! The Win32 side: the layered overlay window, the DIB surface it is presented from, the
//! thread-local event queue the window procedure feeds, the tray icon / menu and autostart.

use crate::ffi::*;
use crate::layout::{WIN_H, WIN_W};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};

pub const WM_APP_MEDIA: u32 = WM_APP + 1;
pub const WM_APP_CLIP: u32 = WM_APP + 2;
pub const WM_APP_TRAY: u32 = WM_APP + 3;
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
}

thread_local! {
    static QUEUE: RefCell<VecDeque<Ev>> = const { RefCell::new(VecDeque::new()) };
}
static TASKBAR_MSG: AtomicU32 = AtomicU32::new(0);

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
            push(Ev::Mouse);
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

    pub fn destroy(&mut self) {
        unsafe {
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
