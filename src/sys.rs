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
    /// True while network sampling is on and the counters deliver data.
    pub net_ok: bool,
}

/// One wildcard PDH query (e.g. "\\GPU Engine(*)\\Utilization Percentage"), read as one value per
/// matching instance. `None` once opening or adding the counter fails, so a system without these
/// counters just reports nothing instead of crashing or spamming retries.
struct PdhSum {
    query: isize,
    counter: isize,
    // u64 backing store: the item array must be 8-aligned for its f64 values.
    buf: Vec<u64>,
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
            // Rate counters need a baseline; collect once so the first real sample has data.
            PdhCollectQueryData(query);
            Some(Self { query, counter, buf: Vec::new() })
        }
    }

    /// (instance name, value) for every instance with a valid value. `None` when PDH fails.
    fn sample(&mut self) -> Option<Vec<(String, f64)>> {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let (mut size, mut count) = (0u32, 0u32);
            let r = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, null_mut());
            if r as i32 != PDH_MORE_DATA || size == 0 {
                // No instance at all (e.g. no adapter up) is a valid, empty reading.
                return if r == 0 { Some(Vec::new()) } else { None };
            }
            self.buf.resize((size as usize).div_ceil(8), 0);
            let items = self.buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                return None;
            }
            let mut out = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                let item = &*items.add(i);
                if pdh_value_ok(item.fmt_value.c_status) {
                    out.push((from_wide_ptr(item.sz_name), item.fmt_value.value.double_value));
                }
            }
            Some(out)
        }
    }
}

/// Both "valid" and "new" (rate counters whose raw data changed) are successful readings.
pub fn pdh_value_ok(status: u32) -> bool {
    status == crate::ffi::PDH_CSTATUS_VALID_DATA || status == crate::ffi::PDH_CSTATUS_NEW_DATA
}

/// GPU load the way Task Manager reports it: per-process instances are summed per engine
/// ("luid_..._eng_N_engtype_3D"), and the busiest engine wins.
pub fn gpu_busiest_engine(items: &[(String, f64)]) -> f64 {
    let mut engines: Vec<(&str, f64)> = Vec::new();
    for (name, v) in items {
        let key = name.find("luid").map_or(name.as_str(), |i| &name[i..]);
        match engines.iter_mut().find(|(k, _)| *k == key) {
            Some(e) => e.1 += v,
            None => engines.push((key, *v)),
        }
    }
    engines.iter().map(|e| e.1).fold(0.0, f64::max)
}

/// Throughput of the busiest adapter. Taking the max instead of the sum avoids counting the same
/// traffic twice through virtual adapters (Hyper-V / WSL vEthernet bridges, VPN taps).
pub fn busiest_adapter(items: &[(String, f64)]) -> f64 {
    items.iter().map(|i| i.1).fold(0.0, f64::max)
}

impl Drop for PdhSum {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

pub struct Sampler {
    prev: Option<(u64, u64)>, // (idle, total) from the last sample
    gpu: Option<PdhSum>,
    net: Option<(PdhSum, PdhSum)>,
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
            net: None,
            stats: Stats { threads, ..Stats::default() },
        };
        s.sample();
        s
    }

    /// Network throughput is opt-in ("Network speed" in Settings): the counters are only open
    /// while enabled.
    pub fn set_net(&mut self, on: bool) {
        if on == self.net.is_some() {
            return;
        }
        self.net = if on {
            PdhSum::open("\\Network Interface(*)\\Bytes Received/sec")
                .zip(PdhSum::open("\\Network Interface(*)\\Bytes Sent/sec"))
        } else {
            None
        };
        self.stats.net_ok = false;
        self.stats.net_down_bps = 0.0;
        self.stats.net_up_bps = 0.0;
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

        if let Some(v) = self.gpu.as_mut().and_then(|g| g.sample()) {
            self.stats.gpu_pct = Some(gpu_busiest_engine(&v).clamp(0.0, 100.0) as f32);
        }
        if let Some((down, up)) = self.net.as_mut() {
            match (down.sample(), up.sample()) {
                (Some(d), Some(u)) => {
                    self.stats.net_down_bps = busiest_adapter(&d).max(0.0);
                    self.stats.net_up_bps = busiest_adapter(&u).max(0.0);
                    self.stats.net_ok = true;
                }
                _ => self.stats.net_ok = false,
            }
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn pdh_new_data_counts() {
        assert!(pdh_value_ok(0));
        assert!(pdh_value_ok(1));
        assert!(!pdh_value_ok(0xC000_0BBA)); // PDH_CSTATUS_NO_INSTANCE
    }

    use super::*;

    #[test]
    fn cpu_math() {
        assert_eq!(cpu_percent((0, 0), (50, 100)), 50.0);
        assert_eq!(cpu_percent((10, 10), (10, 10)), 0.0); // no time elapsed
        assert_eq!(cpu_percent((0, 0), (0, 100)), 100.0);
        assert_eq!(cpu_percent((100, 200), (50, 100)), 0.0); // counters going backwards never panic
    }

    #[test]
    fn gpu_groups_per_engine() {
        let items = vec![
            ("pid_1_luid_0x0_0xA_phys_0_eng_0_engtype_3D".to_string(), 30.0),
            ("pid_2_luid_0x0_0xA_phys_0_eng_0_engtype_3D".to_string(), 25.0),
            ("pid_1_luid_0x0_0xA_phys_0_eng_3_engtype_Copy".to_string(), 40.0),
        ];
        assert_eq!(gpu_busiest_engine(&items), 55.0);
        assert_eq!(gpu_busiest_engine(&[]), 0.0);
        assert_eq!(busiest_adapter(&[("a".into(), 5.0), ("b".into(), 9.0)]), 9.0);
    }

    #[test]
    fn power_decode() {
        assert_eq!(decode_power(1, 8, 55, u32::MAX), (Some(55), true, true, None));
        assert_eq!(decode_power(0, 0, 80, 7200), (Some(80), false, false, Some(7200)));
        assert_eq!(decode_power(1, 128, 255, u32::MAX), (None, false, true, None));
        assert_eq!(decode_power(0, 255, 255, 0), (None, false, false, None));
    }
}
