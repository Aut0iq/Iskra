//! Deterministic elapsed-time roaming. Slow walks along the usable floor
//! alternate with longer quiet pauses; dragging pins, pauses stop the clock,
//! and every advance re-clamps into bounds.

use crate::hyprland::Bounds;

/// Logical pixels per second.
pub const SPEED: f64 = 24.0;
/// Frame gaps longer than this are capped so sleep/resume never teleports.
pub const MAX_STEP_SECS: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Rest { remaining: f64 },
    Walk { target_x: f64 },
}

#[derive(Debug)]
pub struct Motion {
    /// Window top-left in logical pixels relative to the monitor origin.
    pub x: f64,
    pub y: f64,
    phase: Phase,
    rng: u64,
    pub pinned: bool,
    pub enabled: bool,
}

impl Motion {
    /// Starts in the lower-right region of `bounds`.
    pub fn new(bounds: Bounds) -> Self {
        Motion {
            x: bounds.max_x as f64,
            y: bounds.max_y as f64,
            phase: Phase::Rest { remaining: 2.0 },
            rng: 0x9E3779B97F4A7C15,
            pinned: false,
            enabled: true,
        }
    }

    fn next_f64(&mut self) -> f64 {
        // xorshift64*; deterministic, no external entropy needed.
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Advance the walk clock. `dt` is capped to MAX_STEP_SECS.
    /// `may_roam` is false for hidden surfaces and stationary emotions;
    /// paused/pinned/hidden time does not advance motion. Returns true when
    /// the sprite is moving this tick (walk animation).
    pub fn advance(&mut self, dt: f64, bounds: Bounds, may_roam: bool) -> bool {
        // Re-clamp on every call so resizes/unplugs can never strand the pet.
        self.x = self.x.clamp(bounds.min_x as f64, bounds.max_x as f64);
        // A drag may park the pet above the floor while pinned; on unpin it
        // returns to the floor of the usable area.
        self.y = if self.pinned {
            self.y.clamp(bounds.min_y as f64, bounds.max_y as f64)
        } else {
            bounds.max_y as f64
        };
        if !(self.enabled && may_roam) || self.pinned {
            return false;
        }
        let dt = dt.clamp(0.0, MAX_STEP_SECS);
        match self.phase {
            Phase::Rest { remaining } => {
                let left = remaining - dt;
                if left > 0.0 {
                    self.phase = Phase::Rest { remaining: left };
                    false
                } else {
                    let span = (bounds.max_x - bounds.min_x).max(0) as f64;
                    let target = bounds.min_x as f64 + self.next_f64() * span;
                    if (target - self.x).abs() < 2.0 {
                        self.phase = Phase::Rest {
                            remaining: 2.0 + self.next_f64() * 5.0,
                        };
                        false
                    } else {
                        self.phase = Phase::Walk { target_x: target };
                        true
                    }
                }
            }
            Phase::Walk { target_x } => {
                let step = SPEED * dt;
                let dx = target_x - self.x;
                if dx.abs() <= step {
                    self.x = target_x;
                    self.phase = Phase::Rest {
                        remaining: 2.0 + self.next_f64() * 5.0,
                    };
                } else {
                    self.x += step * dx.signum();
                }
                self.x = self.x.clamp(bounds.min_x as f64, bounds.max_x as f64);
                true
            }
        }
    }

    /// Drag-pin: set the position directly and hold it until unpinned.
    pub fn drag_to(&mut self, x: f64, y: f64, bounds: Bounds) {
        self.x = x.clamp(bounds.min_x as f64, bounds.max_x as f64);
        self.y = y.clamp(bounds.min_y as f64, bounds.max_y as f64);
        self.pinned = true;
        self.phase = Phase::Rest { remaining: 2.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Bounds {
        Bounds {
            min_x: 8,
            min_y: 40,
            max_x: 1800,
            max_y: 900,
        }
    }

    #[test]
    fn walks_within_bounds_and_alternates_rest() {
        let mut m = Motion::new(bounds());
        let b = bounds();
        let mut saw_move = false;
        for _ in 0..20_000 {
            let moving = m.advance(0.05, b, true);
            assert!(m.x >= b.min_x as f64 && m.x <= b.max_x as f64);
            saw_move |= moving;
        }
        assert!(saw_move);
    }

    #[test]
    fn pinned_and_paused_do_not_advance() {
        let mut m = Motion::new(bounds());
        let b = bounds();
        let x0 = m.x;
        for _ in 0..1000 {
            m.advance(0.05, b, false);
        }
        assert_eq!(m.x, x0);
        m.pinned = true;
        for _ in 0..1000 {
            m.advance(0.05, b, true);
        }
        assert_eq!(m.x, x0);
        m.enabled = false;
        m.pinned = false;
        for _ in 0..1000 {
            m.advance(0.05, b, true);
        }
        assert_eq!(m.x, x0);
    }

    #[test]
    fn long_gap_is_capped() {
        let mut m = Motion::new(bounds());
        let b = bounds();
        // Force into a walk and feed an enormous dt.
        for _ in 0..2000 {
            if m.advance(0.05, b, true) {
                break;
            }
        }
        let before = m.x;
        m.advance(3600.0, b, true);
        assert!((m.x - before).abs() <= SPEED * MAX_STEP_SECS + 0.01);
    }

    #[test]
    fn tiny_output_stays_clamped() {
        let b = Bounds {
            min_x: 0,
            min_y: 0,
            max_x: 10,
            max_y: 5,
        };
        let mut m = Motion::new(b);
        for _ in 0..5000 {
            m.advance(0.1, b, true);
        }
        assert!(m.x >= 0.0 && m.x <= 10.0 && m.y == 5.0);
    }

    #[test]
    fn drag_pins_and_clamps() {
        let mut m = Motion::new(bounds());
        let b = bounds();
        m.drag_to(5000.0, 20.0, b);
        assert!(m.pinned && m.x == 1800.0);
        m.pinned = false;
        m.enabled = true;
        let _ = m.advance(0.05, b, true);
        assert_eq!(m.y, 900.0);
    }
}
