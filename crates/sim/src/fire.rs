//! Fire-rate, bullet-limit, hold-fire and autofire toggle.

/// Tunables for a tank's weapon (data-driven later).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FireConfig {
    /// Minimum seconds between shots.
    pub fire_interval: f32,
    /// Max live bullets owned by this tank.
    pub max_bullets: usize,
}

impl Default for FireConfig {
    fn default() -> Self {
        Self {
            fire_interval: 0.35,
            max_bullets: 1,
        }
    }
}

/// Tracks cooldown + hold/toggle state for one shooter.
#[derive(Debug, Clone, PartialEq)]
pub struct FireController {
    pub config: FireConfig,
    /// Seconds until next shot is allowed (counts down).
    pub cooldown: f32,
    /// Physical fire button held.
    pub fire_held: bool,
    /// Toggle autofire (classic ports convenience).
    pub autofire: bool,
}

impl FireController {
    pub fn new(config: FireConfig) -> Self {
        Self {
            config,
            cooldown: 0.0,
            fire_held: false,
            autofire: false,
        }
    }

    pub fn toggle_autofire(&mut self) {
        self.autofire = !self.autofire;
    }

    pub fn set_fire_held(&mut self, held: bool) {
        self.fire_held = held;
    }

    /// Whether the weapon wants to shoot this frame (ignoring cooldown/limit).
    pub fn wants_fire(&self) -> bool {
        self.fire_held || self.autofire
    }

    /// Advance cooldown by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        if self.cooldown > 0.0 {
            self.cooldown = (self.cooldown - dt).max(0.0);
        }
    }

    /// Try to authorize a shot given current live bullet count.
    ///
    /// Returns `true` and starts the cooldown if allowed. Does **not** bypass
    /// fire-rate or the per-tank bullet limit (autofire is not a cheat).
    pub fn try_fire(&mut self, live_bullets: usize) -> bool {
        if !self.wants_fire() {
            return false;
        }
        if self.cooldown > 0.0 {
            return false;
        }
        if live_bullets >= self.config.max_bullets {
            return false;
        }
        self.cooldown = self.config.fire_interval;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_fire_respects_interval() {
        let mut fc = FireController::new(FireConfig {
            fire_interval: 0.5,
            max_bullets: 2,
        });
        fc.set_fire_held(true);
        assert!(fc.try_fire(0));
        assert!(!fc.try_fire(0)); // cooldown
        fc.tick(0.49);
        assert!(!fc.try_fire(0));
        fc.tick(0.02);
        assert!(fc.try_fire(0));
    }

    #[test]
    fn autofire_respects_bullet_limit() {
        let mut fc = FireController::new(FireConfig {
            fire_interval: 0.0,
            max_bullets: 1,
        });
        fc.toggle_autofire();
        assert!(fc.autofire);
        assert!(fc.try_fire(0));
        assert!(!fc.try_fire(1)); // at limit
        assert!(fc.try_fire(0)); // again when previous gone
    }

    #[test]
    fn toggle_without_hold_still_fires() {
        let mut fc = FireController::new(FireConfig::default());
        assert!(!fc.wants_fire());
        fc.toggle_autofire();
        assert!(fc.wants_fire());
        assert!(fc.try_fire(0));
    }
}
