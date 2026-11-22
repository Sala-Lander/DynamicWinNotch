//! Turns raw power-status snapshots into the few moments worth a peek: charger plugged in,
//! charger removed, battery low / critical, fully charged. Pure logic, unit-tested.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Power {
    /// `None` on machines without a battery.
    pub pct: Option<u8>,
    pub on_ac: bool,
    pub charging: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerEvent {
    Plugged(Option<u8>),
    Unplugged(Option<u8>),
    Low(u8),
    Critical(u8),
    Full,
}

pub const LOW_PCT: u8 = 20;
pub const CRITICAL_PCT: u8 = 10;

pub struct Watcher {
    last: Power,
    low_sent: bool,
    crit_sent: bool,
    full_sent: bool,
}

impl Watcher {
    pub fn new(initial: Power) -> Self {
        Self { last: initial, low_sent: false, crit_sent: false, full_sent: false }
    }

    /// Feed the latest snapshot; returns an event when something notable just happened.
    pub fn update(&mut self, cur: Power) -> Option<PowerEvent> {
        let prev = self.last;
        self.last = cur;
        let pct = cur.pct?; // no battery: nothing to announce
        if cur.on_ac != prev.on_ac && prev.pct.is_some() {
            self.low_sent = false;
            self.crit_sent = false;
            self.full_sent = cur.on_ac && pct >= 100;
            return Some(if cur.on_ac { PowerEvent::Plugged(Some(pct)) } else { PowerEvent::Unplugged(Some(pct)) });
        }
        if cur.on_ac {
            self.low_sent = false;
            self.crit_sent = false;
            if pct >= 100 {
                if !self.full_sent {
                    self.full_sent = true;
                    return Some(PowerEvent::Full);
                }
            } else if pct < 95 {
                self.full_sent = false;
            }
            return None;
        }
        self.full_sent = false;
        if pct <= CRITICAL_PCT {
            if !self.crit_sent {
                self.crit_sent = true;
                self.low_sent = true;
                return Some(PowerEvent::Critical(pct));
            }
        } else if pct <= LOW_PCT {
            if !self.low_sent {
                self.low_sent = true;
                return Some(PowerEvent::Low(pct));
            }
        } else if pct > LOW_PCT + 5 {
            // Recovered (e.g. a different power plan reporting differently): arm the alerts again.
            self.low_sent = false;
            self.crit_sent = false;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pct: u8, on_ac: bool) -> Power {
        Power { pct: Some(pct), on_ac, charging: on_ac && pct < 100 }
    }

    #[test]
    fn plug_and_unplug() {
        let mut w = Watcher::new(p(80, false));
        assert_eq!(w.update(p(80, false)), None);
        assert_eq!(w.update(p(80, true)), Some(PowerEvent::Plugged(Some(80))));
        assert_eq!(w.update(p(81, true)), None);
        assert_eq!(w.update(p(81, false)), Some(PowerEvent::Unplugged(Some(81))));
    }

    #[test]
    fn low_then_critical_fire_once_each() {
        let mut w = Watcher::new(p(30, false));
        assert_eq!(w.update(p(21, false)), None);
        assert_eq!(w.update(p(20, false)), Some(PowerEvent::Low(20)));
        assert_eq!(w.update(p(19, false)), None);
        assert_eq!(w.update(p(10, false)), Some(PowerEvent::Critical(10)));
        assert_eq!(w.update(p(9, false)), None);
        // Charging and unplugging again re-arms them.
        assert_eq!(w.update(p(9, true)), Some(PowerEvent::Plugged(Some(9))));
        assert_eq!(w.update(p(15, false)), Some(PowerEvent::Unplugged(Some(15))));
        assert_eq!(w.update(p(14, false)), Some(PowerEvent::Low(14)));
    }

    #[test]
    fn dropping_straight_below_critical_skips_low() {
        let mut w = Watcher::new(p(40, false));
        assert_eq!(w.update(p(8, false)), Some(PowerEvent::Critical(8)));
        assert_eq!(w.update(p(7, false)), None);
    }

    #[test]
    fn full_charge_announced_once() {
        let mut w = Watcher::new(p(97, true));
        assert_eq!(w.update(p(99, true)), None);
        assert_eq!(w.update(p(100, true)), Some(PowerEvent::Full));
        assert_eq!(w.update(p(100, true)), None);
        // Starting already full and plugged in is not news.
        let mut w = Watcher::new(p(100, false));
        assert_eq!(w.update(p(100, true)), Some(PowerEvent::Plugged(Some(100))));
        assert_eq!(w.update(p(100, true)), None);
    }

    #[test]
    fn desktops_never_alert() {
        let d = |on_ac| Power { pct: None, on_ac, charging: false };
        let mut w = Watcher::new(d(true));
        assert_eq!(w.update(d(false)), None);
        assert_eq!(w.update(d(true)), None);
        // A battery appearing (docking a laptop-like device) does not claim a plug event.
        assert_eq!(w.update(p(50, false)), None);
    }
}
