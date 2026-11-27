//! Minimal hand-written Win32 bindings (64-bit Windows only).
//!
//! Handles are plain `isize` (HWND, HDC, HBITMAP, HICON, HMENU, HINSTANCE, HKEY...), which has the
//! same ABI as a pointer on x64/ARM64. Writing the ~60 declarations we need by hand avoids
//! depending on the churn of a bindings crate; the `windows` crate is used only for WinRT.
#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::upper_case_acronyms)]

use std::ffi::c_void;

pub type HANDLE = isize;
pub type WndProc = unsafe extern "system" fn(HANDLE, u32, usize, isize) -> isize;

// ---------------------------------------------------------------- structs
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SIZE {
    pub cx: i32,
    pub cy: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
#[repr(C)]
#[derive(Default)]
pub struct MSG {
    pub hwnd: HANDLE,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub pt: POINT,
}
#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: u32,
    pub style: u32,
    pub lpfnWndProc: Option<WndProc>,
    pub cbClsExtra: i32,
    pub cbWndExtra: i32,
    pub hInstance: HANDLE,
    pub hIcon: HANDLE,
    pub hCursor: HANDLE,
    pub hbrBackground: HANDLE,
    pub lpszMenuName: *const u16,
    pub lpszClassName: *const u16,
    pub hIconSm: HANDLE,
}
#[repr(C)]
pub struct BLENDFUNCTION {
    pub BlendOp: u8,
    pub BlendFlags: u8,
    pub SourceConstantAlpha: u8,
    pub AlphaFormat: u8,
}
#[repr(C)]
#[derive(Default)]
pub struct BITMAPINFOHEADER {
    pub biSize: u32,
    pub biWidth: i32,
    pub biHeight: i32,
    pub biPlanes: u16,
    pub biBitCount: u16,
    pub biCompression: u32,
    pub biSizeImage: u32,
    pub biXPelsPerMeter: i32,
    pub biYPelsPerMeter: i32,
    pub biClrUsed: u32,
    pub biClrImportant: u32,
}
#[repr(C)]
#[derive(Default)]
pub struct BITMAPINFO {
    pub bmiHeader: BITMAPINFOHEADER,
    pub bmiColors: [u32; 1],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}
impl FILETIME {
    pub fn as_u64(&self) -> u64 {
        ((self.dwHighDateTime as u64) << 32) | self.dwLowDateTime as u64
    }
}
#[repr(C)]
#[derive(Default)]
pub struct MEMORYSTATUSEX {
    pub dwLength: u32,
    pub dwMemoryLoad: u32,
    pub ullTotalPhys: u64,
    pub ullAvailPhys: u64,
    pub ullTotalPageFile: u64,
    pub ullAvailPageFile: u64,
    pub ullTotalVirtual: u64,
    pub ullAvailVirtual: u64,
    pub ullAvailExtendedVirtual: u64,
}
#[repr(C)]
#[derive(Default)]
pub struct SYSTEM_POWER_STATUS {
    pub ACLineStatus: u8,
    pub BatteryFlag: u8,
    pub BatteryLifePercent: u8,
    pub SystemStatusFlag: u8,
    pub BatteryLifeTime: u32,
    pub BatteryFullLifeTime: u32,
}
#[repr(C)]
#[derive(Default)]
pub struct MONITORINFO {
    pub cbSize: u32,
    pub rcMonitor: RECT,
    pub rcWork: RECT,
    pub dwFlags: u32,
}
#[repr(C)]
pub struct RAWINPUTDEVICE {
    pub usUsagePage: u16,
    pub usUsage: u16,
    pub dwFlags: u32,
    pub hwndTarget: HANDLE,
}
/// Full (Vista+) layout of NOTIFYICONDATAW. The GUID is modelled as `[u32; 4]` (4-byte aligned, like GUID).
#[repr(C)]
pub struct NOTIFYICONDATAW {
    pub cbSize: u32,
    pub hWnd: HANDLE,
    pub uID: u32,
    pub uFlags: u32,
    pub uCallbackMessage: u32,
    pub hIcon: HANDLE,
    pub szTip: [u16; 128],
    pub dwState: u32,
    pub dwStateMask: u32,
    pub szInfo: [u16; 256],
    pub uVersion: u32,
    pub szInfoTitle: [u16; 64],
    pub dwInfoFlags: u32,
    pub guidItem: [u32; 4],
    pub hBalloonIcon: HANDLE,
}

// -------------------------------------------------------------- constants
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_EX_TOPMOST: u32 = 0x0000_0008;
pub const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
pub const WS_EX_LAYERED: u32 = 0x0008_0000;
pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
pub const GWL_EXSTYLE: i32 = -20;

pub const SW_HIDE: i32 = 0;
pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOWNOACTIVATE: i32 = 4;

pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOACTIVATE: u32 = 0x0010;
pub const HWND_TOPMOST: HANDLE = -1;
pub const HWND_MESSAGE: HANDLE = -3;

pub const ULW_ALPHA: u32 = 2;
pub const AC_SRC_OVER: u8 = 0;
pub const AC_SRC_ALPHA: u8 = 1;
pub const BI_RGB: u32 = 0;
pub const DIB_RGB_COLORS: u32 = 0;

pub const PM_REMOVE: u32 = 1;
pub const MA_NOACTIVATE: isize = 3;
pub const IDC_ARROW: usize = 32512;

pub const WM_NULL: u32 = 0x0000;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_QUIT: u32 = 0x0012;
pub const WM_MOUSEACTIVATE: u32 = 0x0021;
pub const WM_INPUT: u32 = 0x00FF;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_MOUSEHWHEEL: u32 = 0x020E;
pub const WM_DPICHANGED: u32 = 0x02E0;
pub const WM_DISPLAYCHANGE: u32 = 0x007E;
pub const WM_HOTKEY: u32 = 0x0312;
pub const WM_POWERBROADCAST: u32 = 0x0218;
pub const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
pub const CF_HDROP: u32 = 15;
pub const WM_SETFOCUS: u32 = 0x0007;
pub const WM_KILLFOCUS: u32 = 0x0008;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_CHAR: u32 = 0x0102;
pub const VK_BACK: usize = 0x08;
pub const VK_ESCAPE: usize = 0x1B;
pub const VK_RETURN: usize = 0x0D;
pub const WM_CLIPBOARDUPDATE: u32 = 0x031D;
pub const WM_APP: u32 = 0x8000;

pub const MF_STRING: u32 = 0x0000;
pub const MF_CHECKED: u32 = 0x0008;
pub const MF_SEPARATOR: u32 = 0x0800;
pub const TPM_RIGHTBUTTON: u32 = 0x0002;
pub const TPM_BOTTOMALIGN: u32 = 0x0020;
pub const TPM_RETURNCMD: u32 = 0x0100;

pub const NIM_ADD: u32 = 0;
pub const NIM_MODIFY: u32 = 1;
pub const NIM_DELETE: u32 = 2;
pub const NIF_MESSAGE: u32 = 1;
pub const NIF_ICON: u32 = 2;
pub const NIF_TIP: u32 = 4;

pub const MOD_ALT: u32 = 1;
pub const MOD_CONTROL: u32 = 2;
pub const MOD_NOREPEAT: u32 = 0x4000;

pub const RIDEV_INPUTSINK: u32 = 0x100;
pub const MONITOR_DEFAULTTOPRIMARY: u32 = 1;

pub const QUNS_BUSY: i32 = 2;
pub const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;
pub const QUNS_PRESENTATION_MODE: i32 = 4;

pub const HKEY_CURRENT_USER: HANDLE = 0x8000_0001u32 as i32 as isize;
pub const REG_SZ: u32 = 1;
pub const RRF_RT_REG_SZ: u32 = 0x0000_0002;
pub const ERROR_ALREADY_EXISTS: u32 = 183;

// ---------------------------------------------------------------- imports
#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassExW(wc: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HANDLE,
        menu: HANDLE,
        hinst: HANDLE,
        param: *mut c_void,
    ) -> HANDLE;
    pub fn DefWindowProcW(hwnd: HANDLE, msg: u32, wp: usize, lp: isize) -> isize;
    pub fn DestroyWindow(hwnd: HANDLE) -> i32;
    pub fn GetMessageW(msg: *mut MSG, hwnd: HANDLE, min: u32, max: u32) -> i32;
    pub fn PeekMessageW(msg: *mut MSG, hwnd: HANDLE, min: u32, max: u32, remove: u32) -> i32;
    pub fn TranslateMessage(msg: *const MSG) -> i32;
    pub fn DispatchMessageW(msg: *const MSG) -> isize;
    pub fn PostMessageW(hwnd: HANDLE, msg: u32, wp: usize, lp: isize) -> i32;
    pub fn PostQuitMessage(code: i32);
    pub fn ShowWindow(hwnd: HANDLE, cmd: i32) -> i32;
    pub fn SetWindowPos(hwnd: HANDLE, after: HANDLE, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    pub fn UpdateLayeredWindow(
        hwnd: HANDLE,
        hdc_dst: HANDLE,
        ppt_dst: *const POINT,
        psize: *const SIZE,
        hdc_src: HANDLE,
        ppt_src: *const POINT,
        crkey: u32,
        pblend: *const BLENDFUNCTION,
        flags: u32,
    ) -> i32;
    pub fn GetWindowLongPtrW(hwnd: HANDLE, index: i32) -> isize;
    pub fn SetWindowLongPtrW(hwnd: HANDLE, index: i32, value: isize) -> isize;
    pub fn GetCursorPos(pt: *mut POINT) -> i32;
    pub fn LoadCursorW(hinst: HANDLE, name: *const u16) -> HANDLE;
    pub fn SetTimer(hwnd: HANDLE, id: usize, ms: u32, proc_: *const c_void) -> usize;
    pub fn KillTimer(hwnd: HANDLE, id: usize) -> i32;
    pub fn CreatePopupMenu() -> HANDLE;
    pub fn AppendMenuW(menu: HANDLE, flags: u32, id: usize, text: *const u16) -> i32;
    pub fn TrackPopupMenu(menu: HANDLE, flags: u32, x: i32, y: i32, reserved: i32, hwnd: HANDLE, rc: *const RECT) -> i32;
    pub fn DestroyMenu(menu: HANDLE) -> i32;
    pub fn SetForegroundWindow(hwnd: HANDLE) -> i32;
    pub fn SetFocus(hwnd: HANDLE) -> HANDLE;
    pub fn RegisterHotKey(hwnd: HANDLE, id: i32, modifiers: u32, vk: u32) -> i32;
    pub fn UnregisterHotKey(hwnd: HANDLE, id: i32) -> i32;
    pub fn RegisterRawInputDevices(devs: *const RAWINPUTDEVICE, count: u32, size: u32) -> i32;
    pub fn RegisterWindowMessageW(name: *const u16) -> u32;
    pub fn CreateIcon(
        hinst: HANDLE,
        w: i32,
        h: i32,
        planes: u8,
        bpp: u8,
        and_bits: *const u8,
        xor_bits: *const u8,
    ) -> HANDLE;
    pub fn DestroyIcon(icon: HANDLE) -> i32;
    pub fn MonitorFromPoint(pt: POINT, flags: u32) -> HANDLE;
    pub fn GetMonitorInfoW(monitor: HANDLE, mi: *mut MONITORINFO) -> i32;
    pub fn AddClipboardFormatListener(hwnd: HANDLE) -> i32;
    pub fn RemoveClipboardFormatListener(hwnd: HANDLE) -> i32;
    pub fn IsClipboardFormatAvailable(format: u32) -> i32;
    pub fn GetClipboardSequenceNumber() -> u32;
    pub fn RegisterClipboardFormatW(name: *const u16) -> u32;
    pub fn OpenClipboard(hwnd: HANDLE) -> i32;
    pub fn CloseClipboard() -> i32;
    pub fn GetClipboardData(format: u32) -> HANDLE;
    pub fn GetDpiForSystem() -> u32;
    pub fn GetDpiForWindow(hwnd: HANDLE) -> u32;
    pub fn SetProcessDpiAwarenessContext(ctx: isize) -> i32;
    pub fn GetForegroundWindow() -> HANDLE;
    pub fn GetWindowThreadProcessId(hwnd: HANDLE, pid: *mut u32) -> u32;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateCompatibleDC(hdc: HANDLE) -> HANDLE;
    pub fn CreateDIBSection(
        hdc: HANDLE,
        bmi: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HANDLE;
    pub fn SelectObject(hdc: HANDLE, obj: HANDLE) -> HANDLE;
    pub fn DeleteObject(obj: HANDLE) -> i32;
    pub fn DeleteDC(hdc: HANDLE) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> HANDLE;
    pub fn GetLastError() -> u32;
    pub fn CreateMutexW(attrs: *const c_void, initial_owner: i32, name: *const u16) -> HANDLE;
    pub fn GetSystemTimes(idle: *mut FILETIME, kernel: *mut FILETIME, user: *mut FILETIME) -> i32;
    pub fn GetSystemTimeAsFileTime(ft: *mut FILETIME);
    pub fn GlobalMemoryStatusEx(ms: *mut MEMORYSTATUSEX) -> i32;
    pub fn GetSystemPowerStatus(sps: *mut SYSTEM_POWER_STATUS) -> i32;
    pub fn GetTickCount64() -> u64;
    pub fn GetCurrentProcess() -> HANDLE;
    pub fn SetProcessWorkingSetSize(process: HANDLE, min: usize, max: usize) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    pub fn Shell_NotifyIconW(msg: u32, data: *mut NOTIFYICONDATAW) -> i32;
    pub fn SHQueryUserNotificationState(state: *mut i32) -> i32;
    pub fn DragQueryFileW(hdrop: HANDLE, index: u32, buf: *mut u16, cch: u32) -> u32;
    pub fn ShellExecuteW(
        hwnd: HANDLE,
        verb: *const u16,
        file: *const u16,
        params: *const u16,
        dir: *const u16,
        show: i32,
    ) -> isize;
}

#[link(name = "dwmapi")]
extern "system" {
    pub fn DwmFlush() -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    pub fn RegSetKeyValueW(
        key: HANDLE,
        subkey: *const u16,
        value: *const u16,
        ty: u32,
        data: *const c_void,
        cb: u32,
    ) -> i32;
    pub fn RegDeleteKeyValueW(key: HANDLE, subkey: *const u16, value: *const u16) -> i32;
    pub fn RegGetValueW(
        key: HANDLE,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        ty: *mut u32,
        data: *mut c_void,
        cb: *mut u32,
    ) -> i32;
}

#[link(name = "ole32")]
extern "system" {
    pub fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
}

// PDH: used for the GPU and network-speed gauges (wildcard counters summed across instances).
pub const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
pub const PDH_CSTATUS_VALID_DATA: u32 = 0;
pub const PDH_CSTATUS_NEW_DATA: u32 = 1;
pub const PDH_MORE_DATA: i32 = 0x8000_07D2u32 as i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub union PDH_VALUE {
    pub long_value: i32,
    pub double_value: f64,
    pub large_value: i64,
    pub str_value: *mut u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PDH_FMT_COUNTERVALUE {
    pub c_status: u32,
    pub value: PDH_VALUE,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PDH_FMT_COUNTERVALUE_ITEM_W {
    pub sz_name: *mut u16,
    pub fmt_value: PDH_FMT_COUNTERVALUE,
}

#[link(name = "pdh")]
extern "system" {
    pub fn PdhOpenQueryW(data_source: *const u16, user_data: usize, query: *mut isize) -> u32;
    pub fn PdhAddEnglishCounterW(query: isize, counter_path: *const u16, user_data: usize, counter: *mut isize) -> u32;
    pub fn PdhCollectQueryData(query: isize) -> u32;
    pub fn PdhGetFormattedCounterArrayW(
        counter: isize,
        format: u32,
        buffer_size: *mut u32,
        item_count: *mut u32,
        items: *mut PDH_FMT_COUNTERVALUE_ITEM_W,
    ) -> u32;
    pub fn PdhCloseQuery(query: isize) -> u32;
}

// ETW: real-time trace session for the game bar's FPS counter (DXGI / D3D9 present events).
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WNODE_HEADER {
    pub BufferSize: u32,
    pub ProviderId: u32,
    pub HistoricalContext: u64,
    pub TimeStamp: i64,
    pub Guid: GUID,
    pub ClientContext: u32,
    pub Flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EVENT_TRACE_PROPERTIES {
    pub Wnode: WNODE_HEADER,
    pub BufferSize: u32,
    pub MinimumBuffers: u32,
    pub MaximumBuffers: u32,
    pub MaximumFileSize: u32,
    pub LogFileMode: u32,
    pub FlushTimer: u32,
    pub EnableFlags: u32,
    pub AgeLimit: i32,
    pub NumberOfBuffers: u32,
    pub FreeBuffers: u32,
    pub EventsLost: u32,
    pub BuffersWritten: u32,
    pub LogBuffersLost: u32,
    pub RealTimeBuffersLost: u32,
    pub LoggerThreadId: HANDLE,
    pub LogFileNameOffset: u32,
    pub LoggerNameOffset: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EVENT_DESCRIPTOR {
    pub Id: u16,
    pub Version: u8,
    pub Channel: u8,
    pub Level: u8,
    pub Opcode: u8,
    pub Task: u16,
    pub Keyword: u64,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EVENT_HEADER {
    pub Size: u16,
    pub HeaderType: u16,
    pub Flags: u16,
    pub EventProperty: u16,
    pub ThreadId: u32,
    pub ProcessId: u32,
    pub TimeStamp: i64,
    pub ProviderId: GUID,
    pub EventDescriptor: EVENT_DESCRIPTOR,
    pub ProcessorTime: u64,
    pub ActivityId: GUID,
}
#[repr(C)]
pub struct EVENT_RECORD {
    pub EventHeader: EVENT_HEADER,
    pub BufferContext: u32,
    pub ExtendedDataCount: u16,
    pub UserDataLength: u16,
    pub ExtendedData: *mut c_void,
    pub UserData: *mut c_void,
    pub UserContext: *mut c_void,
}
pub type EventRecordCallback = unsafe extern "system" fn(*mut EVENT_RECORD);
/// EVENT_TRACE_LOGFILEW. `CurrentEvent` (EVENT_TRACE, 88 bytes) and `LogfileHeader`
/// (TRACE_LOGFILE_HEADER, 280 bytes) are output-only and modelled as opaque, 8-aligned blobs.
#[repr(C)]
pub struct EVENT_TRACE_LOGFILEW {
    pub LogFileName: *mut u16,
    pub LoggerName: *mut u16,
    pub CurrentTime: i64,
    pub BuffersRead: u32,
    pub ProcessTraceMode: u32,
    pub CurrentEvent: [u64; 11],
    pub LogfileHeader: [u64; 35],
    pub BufferCallback: *mut c_void,
    pub BufferSize: u32,
    pub Filled: u32,
    pub EventsLost: u32,
    pub EventRecordCallback: Option<EventRecordCallback>,
    pub IsKernelTrace: u32,
    pub Context: *mut c_void,
}
pub const WNODE_FLAG_TRACED_GUID: u32 = 0x0002_0000;
pub const EVENT_TRACE_REAL_TIME_MODE: u32 = 0x0000_0100;
pub const EVENT_TRACE_CONTROL_STOP: u32 = 1;
pub const EVENT_CONTROL_CODE_ENABLE_PROVIDER: u32 = 1;
pub const TRACE_LEVEL_INFORMATION: u8 = 4;
pub const PROCESS_TRACE_MODE_REAL_TIME: u32 = 0x0000_0100;
pub const PROCESS_TRACE_MODE_EVENT_RECORD: u32 = 0x1000_0000;
pub const EVENT_HEADER_FLAG_32_BIT_HEADER: u16 = 0x0020;
pub const INVALID_PROCESSTRACE_HANDLE: u64 = u64::MAX;
pub const ERROR_ACCESS_DENIED: u32 = 5;

#[link(name = "advapi32")]
extern "system" {
    pub fn StartTraceW(handle: *mut u64, name: *const u16, props: *mut EVENT_TRACE_PROPERTIES) -> u32;
    pub fn ControlTraceW(handle: u64, name: *const u16, props: *mut EVENT_TRACE_PROPERTIES, code: u32) -> u32;
    pub fn EnableTraceEx2(
        handle: u64,
        provider: *const GUID,
        code: u32,
        level: u8,
        match_any: u64,
        match_all: u64,
        timeout: u32,
        params: *const c_void,
    ) -> u32;
    pub fn OpenTraceW(logfile: *mut EVENT_TRACE_LOGFILEW) -> u64;
    pub fn ProcessTrace(handles: *const u64, count: u32, start: *const FILETIME, end: *const FILETIME) -> u32;
    pub fn CloseTrace(handle: u64) -> u32;
}

// ICMP echo, for the game bar's ping readout (no admin rights needed).
#[link(name = "iphlpapi")]
extern "system" {
    pub fn IcmpCreateFile() -> HANDLE;
    pub fn IcmpCloseHandle(h: HANDLE) -> i32;
    pub fn IcmpSendEcho(
        h: HANDLE,
        dest: u32,
        data: *const c_void,
        data_len: u16,
        options: *const c_void,
        reply: *mut c_void,
        reply_len: u32,
        timeout_ms: u32,
    ) -> u32;
}

// ---------------------------------------------------------------- helpers
/// NUL-terminated UTF-16 for passing strings to the `W` APIs.
/// Read a NUL-terminated UTF-16 string (empty for null).
pub unsafe fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copy `s` into a fixed UTF-16 buffer (truncating), always leaving a trailing NUL.
pub fn fill_wide<const N: usize>(dst: &mut [u16; N], s: &str) {
    for (i, u) in s.encode_utf16().take(N - 1).enumerate() {
        dst[i] = u;
    }
}

pub fn loword(v: isize) -> i32 {
    (v & 0xFFFF) as u16 as i16 as i32
}

pub fn hiword(v: isize) -> i32 {
    ((v >> 16) & 0xFFFF) as u16 as i16 as i32
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

// Raw keyboard input (Caps / Num Lock peeks), registry change notifications (mic / camera use).
pub const VK_CAPITAL: u16 = 0x14;
pub const VK_NUMLOCK: u16 = 0x90;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const RID_INPUT: u32 = 0x1000_0003;
pub const RIM_TYPEKEYBOARD: u32 = 1;
pub const RIDEV_REMOVE: u32 = 0x1;
pub const KEY_READ: u32 = 0x2_0019;
pub const RRF_RT_REG_QWORD: u32 = 0x0000_0048;
pub const REG_NOTIFY_CHANGE_NAME: u32 = 0x1;
pub const REG_NOTIFY_CHANGE_LAST_SET: u32 = 0x4;

#[repr(C)]
#[allow(non_snake_case)]
pub struct RAWINPUTHEADER {
    pub dwType: u32,
    pub dwSize: u32,
    pub hDevice: HANDLE,
    pub wParam: usize,
}

#[repr(C)]
#[allow(non_snake_case)]
pub struct RAWKEYBOARD {
    pub MakeCode: u16,
    pub Flags: u16,
    pub Reserved: u16,
    pub VKey: u16,
    pub Message: u32,
    pub ExtraInformation: u32,
}

#[link(name = "user32")]
extern "system" {
    pub fn GetRawInputData(raw: HANDLE, cmd: u32, data: *mut c_void, size: *mut u32, header_size: u32) -> u32;
    pub fn GetKeyState(vk: i32) -> i16;
}

#[link(name = "advapi32")]
extern "system" {
    pub fn RegOpenKeyExW(key: HANDLE, subkey: *const u16, options: u32, sam: u32, out: *mut HANDLE) -> i32;
    pub fn RegCloseKey(key: HANDLE) -> i32;
    pub fn RegEnumKeyExW(
        key: HANDLE,
        index: u32,
        name: *mut u16,
        cch_name: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        cch_class: *mut u32,
        last_write: *mut FILETIME,
    ) -> i32;
    pub fn RegNotifyChangeKeyValue(key: HANDLE, subtree: i32, filter: u32, event: HANDLE, asynchronous: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateEventW(attrs: *const c_void, manual_reset: i32, initial: i32, name: *const u16) -> HANDLE;
    pub fn WaitForMultipleObjects(count: u32, handles: *const HANDLE, wait_all: i32, ms: u32) -> u32;
}

// Brightness peek: power-setting notification for the current monitor brightness.
pub const PBT_POWERSETTINGCHANGE: usize = 0x8013;
pub const DEVICE_NOTIFY_WINDOW_HANDLE: u32 = 0;
/// GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS {8ffee2c6-2d01-46be-adb9-398addc5b4ff}: percent.
pub const GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS: GUID =
    GUID { data1: 0x8FFE_E2C6, data2: 0x2D01, data3: 0x46BE, data4: [0xAD, 0xB9, 0x39, 0x8A, 0xDD, 0xC5, 0xB4, 0xFF] };

#[repr(C)]
#[allow(non_snake_case)]
pub struct POWERBROADCAST_SETTING {
    pub PowerSetting: GUID,
    pub DataLength: u32,
    pub Data: [u8; 1],
}

#[link(name = "user32")]
extern "system" {
    pub fn RegisterPowerSettingNotification(recipient: HANDLE, setting: *const GUID, flags: u32) -> HANDLE;
    pub fn AttachThreadInput(attach: u32, attach_to: u32, on: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetCurrentThreadId() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_win64_abi() {
        // Values taken from the Windows SDK headers for x64.
        assert_eq!(std::mem::size_of::<POINT>(), 8);
        assert_eq!(std::mem::size_of::<RECT>(), 16);
        assert_eq!(std::mem::size_of::<MSG>(), 48);
        assert_eq!(std::mem::size_of::<WNDCLASSEXW>(), 80);
        assert_eq!(std::mem::size_of::<BITMAPINFOHEADER>(), 40);
        assert_eq!(std::mem::size_of::<MEMORYSTATUSEX>(), 64);
        assert_eq!(std::mem::size_of::<SYSTEM_POWER_STATUS>(), 12);
        assert_eq!(std::mem::size_of::<MONITORINFO>(), 40);
        assert_eq!(std::mem::size_of::<RAWINPUTDEVICE>(), 16);
        assert_eq!(std::mem::size_of::<RAWINPUTHEADER>(), 24);
        assert_eq!(std::mem::size_of::<RAWKEYBOARD>(), 16);
        assert_eq!(std::mem::size_of::<NOTIFYICONDATAW>(), 976);
        assert_eq!(std::mem::size_of::<EVENT_TRACE_PROPERTIES>(), 120);
        assert_eq!(std::mem::size_of::<EVENT_HEADER>(), 80);
        assert_eq!(std::mem::size_of::<EVENT_RECORD>(), 112);
        assert_eq!(std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>(), 24);
        assert_eq!(std::mem::size_of::<EVENT_TRACE_LOGFILEW>(), 448);
    }

    #[test]
    fn words() {
        assert_eq!(loword(0x0001_FFFEu32 as isize), -2);
        assert_eq!(hiword(0xFF88_0001u32 as isize), -120);
    }

    #[test]
    fn wide_is_nul_terminated() {
        assert_eq!(wide("ab"), vec![97, 98, 0]);
        let mut b = [0u16; 4];
        fill_wide(&mut b, "abcdef");
        assert_eq!(b, [97, 98, 99, 0]);
    }
}

// ---- sounds (sound.rs): play a WAV straight from memory ----------------------------------
pub const SND_ASYNC: u32 = 0x0001;
pub const SND_NODEFAULT: u32 = 0x0002;
pub const SND_MEMORY: u32 = 0x0004;

#[link(name = "winmm")]
extern "system" {
    pub fn PlaySoundW(sound: *const c_void, module: HANDLE, flags: u32) -> i32;
}

// ---- shelf (shelf.rs / win.rs): OLE drag and drop of files ---------------------------------
pub const VK_LBUTTON: i32 = 0x01;
pub const TYMED_HGLOBAL: u32 = 1;
pub const DVASPECT_CONTENT: u32 = 1;
pub const DROPEFFECT_NONE: u32 = 0;
pub const DROPEFFECT_COPY: u32 = 1;
pub const DRAGDROP_S_DROP: i32 = 0x0004_0100;
pub const DRAGDROP_S_CANCEL: i32 = 0x0004_0101;
pub const DRAGDROP_S_USEDEFAULTCURSORS: i32 = 0x0004_0102;
pub const MK_LBUTTON: u32 = 0x0001;
pub const S_OK: i32 = 0;
pub const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;

pub const IID_IUNKNOWN: GUID =
    GUID { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub const IID_IDROPTARGET: GUID =
    GUID { data1: 0x0000_0122, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub const IID_IDROPSOURCE: GUID =
    GUID { data1: 0x0000_0121, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub const IID_IDATAOBJECT: GUID =
    GUID { data1: 0x0000_010E, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINTL {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct FORMATETC {
    pub cfFormat: u16,
    pub ptd: *mut c_void,
    pub dwAspect: u32,
    pub lindex: i32,
    pub tymed: u32,
}

#[repr(C)]
pub struct STGMEDIUM {
    pub tymed: u32,
    /// The union: for `TYMED_HGLOBAL` the HGLOBAL (for CF_HDROP that is the HDROP).
    pub handle: *mut c_void,
    pub pUnkForRelease: *mut c_void,
}

#[link(name = "ole32")]
extern "system" {
    pub fn OleInitialize(reserved: *mut c_void) -> i32;
    pub fn OleUninitialize();
    pub fn RegisterDragDrop(hwnd: HANDLE, target: *mut c_void) -> i32;
    pub fn RevokeDragDrop(hwnd: HANDLE) -> i32;
    pub fn ReleaseStgMedium(medium: *mut STGMEDIUM);
    pub fn DoDragDrop(data: *mut c_void, source: *mut c_void, ok_effects: u32, effect: *mut u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    pub fn SHParseDisplayName(
        name: *const u16,
        bind_ctx: *mut c_void,
        pidl: *mut *mut c_void,
        attrs_in: u32,
        attrs_out: *mut u32,
    ) -> i32;
    pub fn SHCreateDataObject(
        pidl_folder: *const c_void,
        count: u32,
        pidls: *const *const c_void,
        inner: *mut c_void,
        riid: *const GUID,
        out: *mut *mut c_void,
    ) -> i32;
    pub fn ILFree(pidl: *mut c_void);
}

#[link(name = "user32")]
extern "system" {
    pub fn GetAsyncKeyState(vk: i32) -> i16;
}

#[cfg(test)]
mod shelf_abi_tests {
    use super::*;

    #[test]
    fn ole_struct_sizes_match_win64_abi() {
        assert_eq!(std::mem::size_of::<POINTL>(), 8);
        assert_eq!(std::mem::size_of::<FORMATETC>(), 32);
        assert_eq!(std::mem::size_of::<STGMEDIUM>(), 24);
        assert_eq!(std::mem::size_of::<GUID>(), 16);
    }
}
