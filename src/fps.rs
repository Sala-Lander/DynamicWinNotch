//! Game-bar frame counter: a private real-time ETW session listening to the DXGI and D3D9
//! "Present" events every Direct3D game emits (the same source PresentMon / CapFrameX use), so
//! it works for any game without injecting into it. Only runs while the game bar is on screen
//! and a frame readout is enabled. Starting an ETW session needs administrator rights or
//! membership of the "Performance Log Users" group; otherwise the bar shows a hint instead.

use crate::ffi::*;
use std::collections::VecDeque;
use std::ptr::null;
use std::sync::Mutex;

const SESSION: &str = "NotchIslandFPS";
/// Microsoft-Windows-DXGI
const DXGI: GUID =
    GUID { data1: 0xCA11_C036, data2: 0x0102, data3: 0x4A2D, data4: [0xA6, 0xAD, 0xF0, 0x3C, 0xFE, 0xD5, 0xD3, 0xC9] };
/// Microsoft-Windows-D3D9
const D3D9: GUID =
    GUID { data1: 0x783A_CA0A, data2: 0x790E, data3: 0x4D7F, data4: [0x84, 0x51, 0xAA, 0x85, 0x05, 0x11, 0xC6, 0xB9] };
const DXGI_PRESENT_START: u16 = 42;
const DXGI_PRESENT_MPO_START: u16 = 55;
const D3D9_PRESENT_START: u16 = 1;
const DXGI_PRESENT_TEST: u32 = 1;

/// 100 ns ticks (FILETIME units).
const SEC: i64 = 10_000_000;
/// History kept for the 1% / 0.1% lows.
const WINDOW: i64 = 10 * SEC;
const MAX_FRAMES: usize = 60_000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    pub fps: f32,
    /// FPS at the 99th / 99.9th percentile frame time of the last 10 s.
    pub low1: f32,
    pub low01: f32,
    /// Average frame time of the last second, in ms.
    pub frametime_ms: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Running,
    /// No rights to start an ETW session.
    Denied,
    Failed,
}

/// One present event: (process id, timestamp, came from DXGI).
type Frame = (u32, i64, bool);

static FRAMES: Mutex<VecDeque<Frame>> = Mutex::new(VecDeque::new());

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let rec = &*rec;
    let h = &rec.EventHeader;
    let id = h.EventDescriptor.Id;
    let dxgi = if h.ProviderId == DXGI && (id == DXGI_PRESENT_START || id == DXGI_PRESENT_MPO_START) {
        // Payload: pIDXGISwapChain, Flags, SyncInterval. Test presents draw nothing.
        let ptr = if h.Flags & EVENT_HEADER_FLAG_32_BIT_HEADER != 0 { 4 } else { 8 };
        if rec.UserDataLength as usize >= ptr + 4 && !rec.UserData.is_null() {
            let flags = std::ptr::read_unaligned((rec.UserData as *const u8).add(ptr) as *const u32);
            if flags & DXGI_PRESENT_TEST != 0 {
                return;
            }
        }
        true
    } else if h.ProviderId == D3D9 && id == D3D9_PRESENT_START {
        false
    } else {
        return;
    };
    if let Ok(mut q) = FRAMES.lock() {
        let t = h.TimeStamp;
        while q.front().is_some_and(|f| t - f.1 > WINDOW) || q.len() >= MAX_FRAMES {
            q.pop_front();
        }
        q.push_back((h.ProcessId, t, dxgi));
    }
}

fn now_ticks() -> i64 {
    let mut ft = FILETIME::default();
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    ft.as_u64() as i64
}

/// Process id owning the foreground window (the full-screen game).
pub fn foreground_pid() -> u32 {
    let mut pid = 0u32;
    unsafe {
        let w = GetForegroundWindow();
        if w != 0 {
            GetWindowThreadProcessId(w, &mut pid);
        }
    }
    pid
}

/// Pick the frames to measure: the foreground process if it presented recently, otherwise the
/// busiest presenter (games that render from a helper process). DXGI wins over D3D9 per process
/// so a D3D9-on-DXGI title is not counted twice.
pub fn select(frames: &[Frame], fg_pid: u32, now: i64) -> Vec<i64> {
    let recent = |p: u32| frames.iter().rev().take_while(|f| now - f.1 <= 2 * SEC).filter(|f| f.0 == p).count();
    let mut pid = fg_pid;
    if fg_pid == 0 || recent(fg_pid) == 0 {
        let mut best = (0usize, 0u32);
        for f in frames.iter().rev().take_while(|f| now - f.1 <= 2 * SEC) {
            if f.0 != best.1 {
                let n = recent(f.0);
                if n > best.0 {
                    best = (n, f.0);
                }
            }
        }
        pid = best.1;
    }
    let has_dxgi = frames.iter().any(|f| f.0 == pid && f.2);
    frames.iter().filter(|f| f.0 == pid && f.2 == has_dxgi).map(|f| f.1).collect()
}

/// FPS, lows and frame time from ascending present timestamps (100 ns). `None` when the game
/// has not presented for 2 s (paused, alt-tabbed) or there is too little data.
pub fn compute(times: &[i64], now: i64) -> Option<FrameStats> {
    let last = *times.last()?;
    if times.len() < 3 || now - last > 2 * SEC {
        return None;
    }
    let mut deltas: Vec<f64> =
        times.windows(2).map(|w| (w[1] - w[0]) as f64 / 10_000.0).filter(|d| *d > 0.0 && *d < 2000.0).collect();
    let recent: Vec<f64> = times
        .windows(2)
        .filter(|w| last - w[1] < SEC)
        .map(|w| (w[1] - w[0]) as f64 / 10_000.0)
        .filter(|d| *d > 0.0)
        .collect();
    if deltas.is_empty() || recent.is_empty() {
        return None;
    }
    let avg = recent.iter().sum::<f64>() / recent.len() as f64;
    deltas.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| {
        let i = ((deltas.len() as f64 * p).ceil() as usize).clamp(1, deltas.len()) - 1;
        (1000.0 / deltas[i]) as f32
    };
    Some(FrameStats { fps: (1000.0 / avg) as f32, low1: pct(0.99), low01: pct(0.999), frametime_ms: avg as f32 })
}

pub struct Tracker {
    pub state: State,
    session: u64,
    trace: u64,
    props: Vec<u64>,
    name: Vec<u16>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            state: State::Off,
            session: 0,
            trace: INVALID_PROCESSTRACE_HANDLE,
            props: Vec::new(),
            name: wide(SESSION),
            thread: None,
        }
    }

    fn props(&mut self) -> *mut EVENT_TRACE_PROPERTIES {
        let size = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
        self.props = vec![0u64; (size + 2 * (SESSION.len() + 1)).div_ceil(8) + 1];
        let p = self.props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        unsafe {
            let pr = &mut *p;
            pr.Wnode.BufferSize = (self.props.len() * 8) as u32;
            pr.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            pr.Wnode.ClientContext = 1; // QPC timestamps (converted to FILETIME on delivery)
            pr.BufferSize = 16; // KB
            pr.MinimumBuffers = 4;
            pr.MaximumBuffers = 32;
            pr.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
            pr.FlushTimer = 1;
            pr.LoggerNameOffset = size as u32;
        }
        p
    }

    /// Start or stop the session. Cheap to call every second.
    pub fn set(&mut self, on: bool) {
        match (on, self.state) {
            (true, State::Off) => self.start(),
            (false, State::Running) => self.stop(),
            (false, _) => self.state = State::Off,
            _ => {}
        }
    }

    fn start(&mut self) {
        unsafe {
            let mut r = StartTraceW(&mut self.session, self.name.as_ptr(), self.props());
            if r == ERROR_ALREADY_EXISTS {
                // Left over from a crashed run: stop it and start fresh.
                ControlTraceW(0, self.name.as_ptr(), self.props(), EVENT_TRACE_CONTROL_STOP);
                r = StartTraceW(&mut self.session, self.name.as_ptr(), self.props());
            }
            if r != 0 {
                self.state = if r == ERROR_ACCESS_DENIED { State::Denied } else { State::Failed };
                return;
            }
            for g in [DXGI, D3D9] {
                EnableTraceEx2(
                    self.session,
                    &g,
                    EVENT_CONTROL_CODE_ENABLE_PROVIDER,
                    TRACE_LEVEL_INFORMATION,
                    u64::MAX,
                    0,
                    0,
                    null(),
                );
            }
            let mut lf: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
            lf.LoggerName = self.name.as_mut_ptr();
            lf.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
            lf.EventRecordCallback = Some(on_event);
            self.trace = OpenTraceW(&mut lf);
            if self.trace == INVALID_PROCESSTRACE_HANDLE {
                ControlTraceW(self.session, null(), self.props(), EVENT_TRACE_CONTROL_STOP);
                self.state = State::Failed;
                return;
            }
            let h = self.trace;
            // ProcessTrace blocks, delivering events, until the session is stopped.
            self.thread = std::thread::Builder::new()
                .name("fps-etw".into())
                .spawn(move || {
                    ProcessTrace(&h, 1, null(), null());
                })
                .ok();
            self.state = State::Running;
        }
    }

    fn stop(&mut self) {
        unsafe {
            ControlTraceW(self.session, null(), self.props(), EVENT_TRACE_CONTROL_STOP);
            CloseTrace(self.trace);
        }
        self.trace = INVALID_PROCESSTRACE_HANDLE;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Ok(mut q) = FRAMES.lock() {
            q.clear();
        }
        self.state = State::Off;
    }

    /// Current numbers for the foreground game, if it is presenting.
    pub fn stats(&self) -> Option<FrameStats> {
        if self.state != State::Running {
            return None;
        }
        let frames: Vec<Frame> = FRAMES.lock().ok()?.iter().copied().collect();
        compute(&select(&frames, foreground_pid(), now_ticks()), now_ticks())
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        if self.state == State::Running {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steady(n: usize, ms: i64, start: i64) -> Vec<i64> {
        (0..n as i64).map(|i| start + i * ms * 10_000).collect()
    }

    #[test]
    fn steady_60() {
        let t = steady(600, 16, 0); // ~62.5 fps for 9.6 s
        let s = compute(&t, *t.last().unwrap()).unwrap();
        assert!((s.fps - 62.5).abs() < 0.5, "{s:?}");
        assert!((s.frametime_ms - 16.0).abs() < 0.1);
        assert!((s.low1 - 62.5).abs() < 0.5);
    }

    #[test]
    fn stutter_shows_in_lows() {
        let mut t = steady(1000, 10, 0); // 100 fps
                                         // Insert 10 hitches of 50 ms.
        for k in 1..=10 {
            let i = k * 90;
            for v in t.iter_mut().skip(i) {
                *v += 400_000;
            }
        }
        let s = compute(&t, *t.last().unwrap()).unwrap();
        assert!(s.fps > 95.0, "{s:?}");
        assert!(s.low1 < 25.0 && s.low01 < 25.0, "{s:?}");
    }

    #[test]
    fn stale_or_empty_is_none() {
        let t = steady(100, 16, 0);
        assert!(compute(&t, t.last().unwrap() + 3 * SEC).is_none());
        assert!(compute(&[], 0).is_none());
        assert!(compute(&t[..2], t[1]).is_none());
    }

    #[test]
    fn selects_foreground_then_busiest_and_prefers_dxgi() {
        let mut f: Vec<Frame> = Vec::new();
        for i in 0..100i64 {
            f.push((7, i * 100_000, true)); // game, DXGI
            f.push((7, i * 100_000 + 1, false)); // same game also emitting D3D9
            if i % 4 == 0 {
                f.push((9, i * 100_000 + 2, true)); // browser
            }
        }
        let now = 100 * 100_000;
        assert_eq!(select(&f, 7, now).len(), 100);
        assert_eq!(select(&f, 9, now).len(), 25);
        assert_eq!(select(&f, 1234, now).len(), 100); // unknown foreground -> busiest
    }
}
