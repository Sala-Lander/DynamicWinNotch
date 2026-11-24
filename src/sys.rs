//! CPU / RAM / battery sampling through three cheap Win32 calls. Sampled only while the
//! System tab is on screen, so it costs nothing the rest of the time.

use crate::ffi::*;
use crate::power::Power;
use std::ptr::{null, null_mut};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub cpu: f32,
    pub ram_pct: f32,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    /// `None` on machines without a battery.
    pub battery: Option<u8>,
    pub charging: bool,
    pub on_ac: bool,
    pub secs_left: Option<u32>,
    pub threads: usize,
    /// `None` when no "GPU Engine" performance counters are available.
    pub gpu_pct: Option<f32>,
    pub net_down_bps: f64,
    pub net_up_bps: f64,
}

/// One wildcard PDH query (e.g. "\\GPU Engine(*)\\Utilization Percentage"): several matching
/// instances summed into a single number. `None` once opening or adding the counter fails, so a
/// system without these counters just reports nothing instead of crashing or spamming retries.
struct PdhSum {
    query: isize,
    counter: isize,
    buf: Vec<u8>,
}

impl PdhSum {
    fn open(counter_path: &str) -> Option<Self> {
        unsafe {
            let mut query = 0isize;
            if PdhOpenQueryW(null(), 0, &mut query) != 0 {
                return None;
            }
            let mut counter = 0isize;
            let path = wide(counter_path);
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                PdhCloseQuery(query);
                return None;
            }
            Some(Self { query, counter, buf: Vec::new() })
        }
    }

    /// Sum of the formatted value across every instance the wildcard expanded to.
    fn sample(&mut self) -> Option<f64> {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let (mut size, mut count) = (0u32, 0u32);
            let r = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, null_mut());
            if r as i32 != PDH_MORE_DATA || size == 0 {
                return None;
            }
            self.buf.resize(size as usize, 0);
            let items = self.buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                return None;
            }
            let mut total = 0.0;
            for i in 0..count as usize {
                let item = &*items.add(i);
                if item.fmt_value.c_status == 0 {
                    total += item.fmt_value.value.double_value;
                }
            }
            Some(total)
        }
    }
}

impl Drop for PdhSum {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

pub struct Sampler {
    prev: Option<(u64, u64)>, // (idle, total) from the last sample
    gpu: Option<PdhSum>,
    net_down: Option<PdhSum>,
    net_up: Option<PdhSum>,
    pub stats: Stats,
}

/// CPU load in percent from two (idle, total) tick pairs.
pub fn cpu_percent(prev: (u64, u64), cur: (u64, u64)) -> f32 {
    let d_total = cur.1.saturating_sub(prev.1);
    let d_idle = cur.0.saturating_sub(prev.0);
    if d_total == 0 {
        return 0.0;
    }
    (100.0 * (1.0 - d_idle as f64 / d_total as f64)).clamp(0.0, 100.0) as f32
}

/// Decode `SYSTEM_POWER_STATUS` fields: (percent, charging, on_ac, seconds_left).
pub fn decode_power(ac_line: u8, flag: u8, percent: u8, life_time: u32) -> (Option<u8>, bool, bool, Option<u32>) {
    let on_ac = ac_line == 1;
    if flag == 255 || flag & 128 != 0 || percent > 100 {
        // 128 = no system battery, 255/>100 = unknown
        return (None, false, on_ac, None);
    }
    let charging = flag & 8 != 0;
    let secs = if life_time == u32::MAX || on_ac { None } else { Some(life_time) };
    (Some(percent), charging, on_ac, secs)
}

/// One cheap read of the power state (used by the battery / charger alerts).
pub fn power_now() -> Power {
    let mut ps = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut ps) } != 0 {
        let (pct, charging, on_ac, _) = decode_power(ps.ACLineStatus, ps.BatteryFlag, ps.BatteryLifePercent, ps.BatteryLifeTime);
        Power { pct, on_ac, charging }
    } else {
        Power { pct: None, on_ac: true, charging: false }
    }
}

impl Sampler {
    pub fn new() -> Self {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let mut s = Self {
            prev: None,
            gpu: PdhSum::open("\\GPU Engine(*)\\Utilization Percentage"),
            net_down: PdhSum::open("\\Network Interface(*)\\Bytes Received/sec"),
            net_up: PdhSum::open("\\Network Interface(*)\\Bytes Sent/sec"),
            stats: Stats { threads, ..Stats::default() },
        };
        s.sample();
        s
    }

    fn times() -> Option<(u64, u64)> {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
        if ok == 0 {
            return None;
        }
        // `kernel` already includes idle time.
        Some((idle.as_u64(), kernel.as_u64() + user.as_u64()))
    }

    pub fn sample(&mut self) {
        if let Some(cur) = Self::times() {
            if let Some(prev) = self.prev {
                self.stats.cpu = cpu_percent(prev, cur);
            }
            self.prev = Some(cur);
        }

        let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if unsafe { GlobalMemoryStatusEx(&mut ms) } != 0 {
            let gb = 1024.0f64 * 1024.0 * 1024.0;
            let total = ms.ullTotalPhys as f64;
            let used = total - ms.ullAvailPhys as f64;
            self.stats.ram_total_gb = (total / gb) as f32;
            self.stats.ram_used_gb = (used / gb) as f32;
            self.stats.ram_pct = ms.dwMemoryLoad as f32;
        }

        let mut ps = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut ps) } != 0 {
            let (pct, charging, on_ac, secs) =
                decode_power(ps.ACLineStatus, ps.BatteryFlag, ps.BatteryLifePercent, ps.BatteryLifeTime);
            self.stats.battery = pct;
            self.stats.charging = charging;
            self.stats.on_ac = on_ac;
            self.stats.secs_left = secs;
        }

        // GPU utilization is a sum across every engine instance (3D, copy, video decode, ...);
        // clamped because a busy multi-GPU machine can genuinely add up past 100%.
        if let Some(gpu) = self.gpu.as_mut() {
            if let Some(v) = gpu.sample() {
                self.stats.gpu_pct = Some(v.clamp(0.0, 100.0) as f32);
            }
        }
        if let Some(c) = self.net_down.as_mut() {
            if let Some(v) = c.sample() {
                self.stats.net_down_bps = v.max(0.0);
            }
        }
        if let Some(c) = self.net_up.as_mut() {
            if let Some(v) = c.sample() {
                self.stats.net_up_bps = v.max(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_math() {
        assert_eq!(cpu_percent((0, 0), (50, 100)), 50.0);
        assert_eq!(cpu_percent((10, 10), (10, 10)), 0.0); // no time elapsed
        assert_eq!(cpu_percent((0, 0), (0, 100)), 100.0);
        assert_eq!(cpu_percent((100, 200), (50, 100)), 0.0); // counters going backwards never panic
    }

    #[test]
    fn power_decode() {
        assert_eq!(decode_power(1, 8, 55, u32::MAX), (Some(55), true, true, None));
        assert_eq!(decode_power(0, 0, 80, 7200), (Some(80), false, false, Some(7200)));
        assert_eq!(decode_power(1, 128, 255, u32::MAX), (None, false, true, None));
        assert_eq!(decode_power(0, 255, 255, 0), (None, false, false, None));
    }
}
