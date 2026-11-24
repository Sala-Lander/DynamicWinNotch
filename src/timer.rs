//! Countdown timer logic. Time is passed in (milliseconds on any monotonic clock), so the
//! state machine has no clock of its own and is unit-tested without sleeping.

pub const MIN_MS: u64 = 60_000;
pub const MAX_MS: u64 = 180 * 60_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timer {
    /// The length the ring is measured against and the timer resets to.
    total_ms: u64,
    /// Time left when not running (or when the current run began).
    left_ms: u64,
    run_start: Option<u64>,
    finished: bool,
}

impl Default for Timer {
    fn default() -> Self {
        Self { total_ms: 25 * MIN_MS, left_ms: 25 * MIN_MS, run_start: None, finished: false }
    }
}

impl Timer {
    pub fn running(&self) -> bool {
        self.run_start.is_some()
    }

    /// True after the countdown reached zero, until the next start / reset.
    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn total_ms(&self) -> u64 {
        self.total_ms
    }

    pub fn remaining(&self, now: u64) -> u64 {
        match self.run_start {
            Some(t0) => self.left_ms.saturating_sub(now.saturating_sub(t0)),
            None => self.left_ms,
        }
    }

    /// 1.0 = full, 0.0 = done.
    pub fn fraction(&self, now: u64) -> f32 {
        if self.total_ms == 0 {
            return 0.0;
        }
        (self.remaining(now) as f32 / self.total_ms as f32).clamp(0.0, 1.0)
    }

    /// Whole seconds to display: rounds up, so "00:00" appears only when it is really over.
    pub fn display_secs(&self, now: u64) -> u64 {
        self.remaining(now).div_ceil(1000)
    }

    /// Choose a length (minutes) and start counting right away.
    pub fn start_preset(&mut self, minutes: u32, now: u64) {
        self.total_ms = (minutes as u64 * MIN_MS).clamp(MIN_MS, MAX_MS);
        self.left_ms = self.total_ms;
        self.finished = false;
        self.run_start = Some(now);
    }

    /// Start / pause. After a finish this restarts from the full length.
    pub fn toggle(&mut self, now: u64) {
        if self.finished || (self.run_start.is_none() && self.left_ms == 0) {
            self.left_ms = self.total_ms;
            self.finished = false;
            self.run_start = Some(now);
        } else if let Some(_t0) = self.run_start {
            self.left_ms = self.remaining(now);
            self.run_start = None;
        } else {
            self.run_start = Some(now);
        }
    }

    pub fn reset(&mut self) {
        self.left_ms = self.total_ms;
        self.run_start = None;
        self.finished = false;
    }

    /// Add / remove whole minutes (running or not). The length stays within 1 min .. 3 h.
    pub fn adjust(&mut self, delta_min: i32, now: u64) {
        let delta = delta_min as i64 * MIN_MS as i64;
        let rem = self.remaining(now) as i64;
        let new_total = (self.total_ms as i64 + delta).clamp(MIN_MS as i64, MAX_MS as i64) as u64;
        // Keep the elapsed part: only the unfinished part moves with the length.
        let new_left = (rem + (new_total as i64 - self.total_ms as i64)).clamp(1000, new_total as i64) as u64;
        self.total_ms = new_total;
        self.left_ms = new_left;
        if self.run_start.is_some() {
            self.run_start = Some(now);
        }
        self.finished = false;
    }

    /// Call regularly while running; returns true exactly once, when the countdown ends.
    pub fn tick(&mut self, now: u64) -> bool {
        if self.run_start.is_some() && self.remaining(now) == 0 {
            self.run_start = None;
            self.left_ms = 0;
            self.finished = true;
            return true;
        }
        false
    }

    /// Milliseconds until the displayed second changes (None when stopped). Never 0.
    pub fn next_wake_ms(&self, now: u64) -> Option<u64> {
        self.run_start?;
        let rem = self.remaining(now);
        if rem == 0 {
            return Some(1);
        }
        let to_boundary = match rem % 1000 {
            0 => 1000,
            r => r,
        };
        Some(to_boundary + 2) // +2 ms so the wake-up lands after the boundary
    }
}

pub const MAX_REMINDERS: usize = 5;
/// Minutes offered by the three reminder quick-add buttons.
pub const REMIND_PRESETS: [u32; 3] = [10, 30, 60];

/// A simple stopwatch: counts up, pause/resume keeps the accumulated time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stopwatch {
    running_since: Option<u64>,
    accumulated: u64,
}

impl Stopwatch {
    pub fn running(&self) -> bool {
        self.running_since.is_some()
    }

    pub fn elapsed(&self, now: u64) -> u64 {
        match self.running_since {
            Some(t0) => self.accumulated + now.saturating_sub(t0),
            None => self.accumulated,
        }
    }

    /// Start / pause, keeping the elapsed time either way.
    pub fn toggle(&mut self, now: u64) {
        match self.running_since {
            Some(t0) => {
                self.accumulated += now.saturating_sub(t0);
                self.running_since = None;
            }
            None => self.running_since = Some(now),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Add a reminder `minutes` from now. Dropped silently once `MAX_REMINDERS` are already pending.
pub fn add_reminder(list: &mut Vec<u64>, now: u64, minutes: u32) {
    if list.len() < MAX_REMINDERS {
        list.push(now + minutes as u64 * MIN_MS);
        list.sort_unstable();
    }
}

/// Remove and return every reminder that is due by `now` (soonest first).
pub fn take_due(list: &mut Vec<u64>, now: u64) -> Vec<u64> {
    let split = list.partition_point(|&t| t <= now);
    list.drain(..split).collect()
}

/// Milliseconds until the next reminder fires, if any are pending.
pub fn next_reminder_wake(list: &[u64], now: u64) -> Option<u64> {
    list.iter().min().map(|&t| t.saturating_sub(now).max(1))
}

/// "25:00", "04:07", "1:05:00".
pub fn fmt_clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_down_and_finishes_once() {
        let mut t = Timer::default();
        t.start_preset(1, 1000);
        assert!(t.running() && !t.finished());
        assert_eq!(t.remaining(1000), 60_000);
        assert_eq!(t.remaining(31_000), 30_000);
        assert!(!t.tick(31_000));
        assert!(t.tick(61_000));
        assert!(t.finished() && !t.running());
        assert_eq!(t.remaining(99_999), 0);
        assert!(!t.tick(70_000)); // only once
    }

    #[test]
    fn pause_resume_keeps_remaining() {
        let mut t = Timer::default();
        t.start_preset(5, 0);
        t.toggle(100_000); // pause with 200 s left
        assert!(!t.running());
        assert_eq!(t.remaining(500_000), 200_000);
        t.toggle(500_000); // resume
        assert_eq!(t.remaining(530_000), 170_000);
    }

    #[test]
    fn toggle_after_finish_restarts_full_length() {
        let mut t = Timer::default();
        t.start_preset(1, 0);
        assert!(t.tick(60_000));
        t.toggle(70_000);
        assert!(t.running() && !t.finished());
        assert_eq!(t.remaining(70_000), 60_000);
    }

    #[test]
    fn reset_and_adjust() {
        let mut t = Timer::default(); // 25 min
        assert_eq!(t.remaining(0), 25 * MIN_MS);
        t.adjust(5, 0);
        assert_eq!(t.total_ms(), 30 * MIN_MS);
        assert_eq!(t.remaining(0), 30 * MIN_MS);
        t.adjust(-100, 0);
        assert_eq!(t.total_ms(), MIN_MS); // clamped
        t.adjust(1000, 0);
        assert_eq!(t.total_ms(), MAX_MS);

        let mut t = Timer::default();
        t.start_preset(10, 0);
        t.adjust(1, 120_000); // 2 min elapsed, add a minute: 9 min left of 11
        assert_eq!(t.total_ms(), 11 * MIN_MS);
        assert_eq!(t.remaining(120_000), 9 * MIN_MS);
        assert!((t.fraction(120_000) - 9.0 / 11.0).abs() < 1e-5);
        t.adjust(-1, 120_000);
        t.adjust(-5, 120_000); // never below a second left, never finished by adjusting
        assert!(t.remaining(120_000) >= 1000 && t.running());
        t.reset();
        assert!(!t.running());
        assert_eq!(t.remaining(0), t.total_ms());
    }

    #[test]
    fn display_rounds_up_and_wakes_on_second_boundaries() {
        let mut t = Timer::default();
        t.start_preset(1, 0);
        assert_eq!(t.display_secs(0), 60);
        assert_eq!(t.display_secs(500), 60);
        assert_eq!(t.display_secs(59_001), 1);
        assert_eq!(t.display_secs(60_000), 0);
        assert_eq!(t.next_wake_ms(0), Some(1002)); // exactly on a boundary: next one in 1 s
        assert_eq!(t.next_wake_ms(250), Some(752));
        assert_eq!(t.next_wake_ms(60_000), Some(1));
        t.toggle(1);
        assert_eq!(t.next_wake_ms(1), None); // paused: no wake-ups at all
    }

    #[test]
    fn clock_format() {
        assert_eq!(fmt_clock(0), "00:00");
        assert_eq!(fmt_clock(247), "04:07");
        assert_eq!(fmt_clock(1500), "25:00");
        assert_eq!(fmt_clock(3905), "1:05:05");
    }

    #[test]
    fn stopwatch_counts_up_and_pauses() {
        let mut sw = Stopwatch::default();
        assert!(!sw.running());
        sw.toggle(1000);
        assert!(sw.running());
        assert_eq!(sw.elapsed(5000), 4000);
        sw.toggle(5000); // pause
        assert!(!sw.running());
        assert_eq!(sw.elapsed(9000), 4000); // time stands still while paused
        sw.toggle(9000); // resume
        assert_eq!(sw.elapsed(10_000), 5000);
        sw.reset();
        assert!(!sw.running());
        assert_eq!(sw.elapsed(10_000), 0);
    }

    #[test]
    fn reminders_add_sorted_cap_and_fire_once() {
        let mut list = Vec::new();
        add_reminder(&mut list, 0, 30);
        add_reminder(&mut list, 0, 10);
        add_reminder(&mut list, 0, 60);
        assert_eq!(list, [10 * MIN_MS, 30 * MIN_MS, 60 * MIN_MS]); // sorted soonest first
        assert_eq!(next_reminder_wake(&list, 0), Some(10 * MIN_MS));
        assert!(take_due(&mut list, 5 * MIN_MS).is_empty());
        let due = take_due(&mut list, 31 * MIN_MS);
        assert_eq!(due, [10 * MIN_MS, 30 * MIN_MS]);
        assert_eq!(list, [60 * MIN_MS]);
        assert!(take_due(&mut list, 31 * MIN_MS).is_empty()); // already removed, no re-fire
    }

    #[test]
    fn reminders_cap_at_max() {
        let mut list = Vec::new();
        for i in 0..MAX_REMINDERS + 3 {
            add_reminder(&mut list, 0, 1 + i as u32);
        }
        assert_eq!(list.len(), MAX_REMINDERS);
    }
}
