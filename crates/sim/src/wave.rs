//! Enemy wave / on-field tank limit (Battle City–style).

/// How many enemies in the level wave and how many may be alive at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveConfig {
    /// Total enemies that will appear this level (queue + eventually on field).
    pub total_enemies: u32,
    /// Hard cap of enemy tanks simultaneously on the map (classic: 4).
    pub max_on_field: u32,
    /// Minimum seconds between spawns when a slot is free.
    pub spawn_interval: f32,
}

impl Default for WaveConfig {
    fn default() -> Self {
        Self {
            total_enemies: 8,
            max_on_field: 4,
            // Pause after spawn *and* after a kill so the death is readable.
            spawn_interval: 2.0,
        }
    }
}

/// Runtime counters for one level wave.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveState {
    pub config: WaveConfig,
    /// Enemies not yet spawned onto the map.
    pub remaining_to_spawn: u32,
    /// Seconds until the next spawn attempt is allowed.
    pub spawn_cooldown: f32,
    /// Enemies destroyed this level (for later win condition).
    pub destroyed: u32,
}

impl WaveState {
    pub fn new(config: WaveConfig) -> Self {
        Self {
            remaining_to_spawn: config.total_enemies,
            spawn_cooldown: 0.0,
            destroyed: 0,
            config,
        }
    }

    pub fn tick_cooldown(&mut self, dt: f32) {
        if self.spawn_cooldown > 0.0 {
            self.spawn_cooldown = (self.spawn_cooldown - dt).max(0.0);
        }
    }

    /// Whether we may try to place another enemy given `on_field` live enemies.
    pub fn can_spawn(&self, on_field: u32) -> bool {
        self.remaining_to_spawn > 0
            && on_field < self.config.max_on_field
            && self.spawn_cooldown <= 0.0
    }

    /// Record a successful spawn: consume one from the queue and start cooldown.
    pub fn on_spawned(&mut self) {
        debug_assert!(self.remaining_to_spawn > 0);
        self.remaining_to_spawn = self.remaining_to_spawn.saturating_sub(1);
        self.spawn_cooldown = self.config.spawn_interval;
    }

    pub fn on_destroyed(&mut self) {
        self.destroyed += 1;
        // Freeing a slot must not instantly refill — start (or refresh) the delay.
        self.spawn_cooldown = self.spawn_cooldown.max(self.config.spawn_interval);
    }

    /// Enemies still “in the wave” for HUD: waiting + currently on field.
    pub fn remaining_in_wave(&self, on_field: u32) -> u32 {
        self.remaining_to_spawn + on_field
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_max_on_field() {
        let mut w = WaveState::new(WaveConfig {
            total_enemies: 10,
            max_on_field: 4,
            spawn_interval: 0.0,
        });
        assert!(w.can_spawn(0));
        assert!(w.can_spawn(3));
        assert!(!w.can_spawn(4));
        w.on_spawned();
        assert_eq!(w.remaining_to_spawn, 9);
    }
}
