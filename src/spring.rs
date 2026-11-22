//! Damped-spring physics (unit mass).
//!
//! Semi-implicit Euler with fixed 240 Hz sub-steps, so the motion is frame-rate independent
//! and stays stable even after a long frame (e.g. the first frame after the app was idle).

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub pos: f32,
    pub vel: f32,
    pub target: f32,
    k: f32,
    c: f32,
}

impl Spring {
    /// `stiffness` is k (1/s^2); `ratio` is the damping ratio: < 1 bounces, 1 is critical.
    pub fn new(pos: f32, stiffness: f32, ratio: f32) -> Self {
        Self { pos, vel: 0.0, target: pos, k: stiffness, c: 2.0 * ratio * stiffness.sqrt() }
    }

    pub fn set_target(&mut self, t: f32) {
        self.target = t;
    }

    /// Jump to `v` with no motion.
    #[allow(dead_code)]
    pub fn snap(&mut self, v: f32) {
        self.pos = v;
        self.target = v;
        self.vel = 0.0;
    }

    /// Add an instantaneous velocity (for "pop" effects).
    #[allow(dead_code)]
    pub fn kick(&mut self, v: f32) {
        self.vel += v;
    }

    pub fn at_rest(&self) -> bool {
        self.pos == self.target && self.vel == 0.0
    }

    pub fn step(&mut self, dt: f32) {
        if self.at_rest() {
            return;
        }
        let dt = dt.clamp(0.0, 0.1);
        let n = ((dt * 240.0).ceil() as u32).max(1);
        let h = dt / n as f32;
        for _ in 0..n {
            let a = -self.k * (self.pos - self.target) - self.c * self.vel;
            self.vel += a * h;
            self.pos += self.vel * h;
        }
        if (self.pos - self.target).abs() < 0.005 && self.vel.abs() < 0.02 {
            self.pos = self.target;
            self.vel = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut Spring, secs: f32) -> f32 {
        let mut peak = s.pos;
        let mut t = 0.0;
        while t < secs {
            s.step(1.0 / 60.0);
            peak = peak.max(s.pos);
            t += 1.0 / 60.0;
        }
        peak
    }

    #[test]
    fn converges_and_comes_to_rest() {
        let mut s = Spring::new(0.0, 380.0, 0.7);
        s.set_target(100.0);
        run(&mut s, 3.0);
        assert!(s.at_rest());
        assert_eq!(s.pos, 100.0);
    }

    #[test]
    fn underdamped_overshoots() {
        let mut s = Spring::new(0.0, 380.0, 0.5);
        s.set_target(1.0);
        assert!(run(&mut s, 2.0) > 1.05);
    }

    #[test]
    fn critically_damped_does_not_overshoot() {
        let mut s = Spring::new(0.0, 380.0, 1.0);
        s.set_target(1.0);
        assert!(run(&mut s, 2.0) <= 1.0001);
    }

    #[test]
    fn huge_dt_stays_finite() {
        let mut s = Spring::new(0.0, 900.0, 0.4);
        s.set_target(50.0);
        for _ in 0..20 {
            s.step(10.0);
            assert!(s.pos.is_finite() && s.vel.is_finite());
        }
    }

    #[test]
    fn snap_is_at_rest() {
        let mut s = Spring::new(5.0, 300.0, 0.8);
        s.set_target(9.0);
        s.snap(7.0);
        assert!(s.at_rest());
        assert_eq!(s.pos, 7.0);
    }
}
