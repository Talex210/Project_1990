//! Player lives, score thresholds → extra life, invulnerability after respawn.

/// Data-driven lives / score-life rules (see `docs/SCORE.md`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LivesConfig {
    pub starting_lives: u32,
    /// Hard cap after threshold bonuses (`0` = no cap).
    pub max_lives: u32,
    /// Every this many run points → +1 life (classic draft: 20_000).
    pub life_every_points: u32,
    /// Seconds of invulnerability after respawn.
    pub invuln_secs: f32,
    /// Stub points per enemy kill until full score table (phase 2.4).
    pub points_per_enemy: u32,
}

impl Default for LivesConfig {
    fn default() -> Self {
        Self {
            starting_lives: 3,
            max_lives: 5,
            life_every_points: 20_000,
            invuln_secs: 3.0,
            points_per_enemy: 100,
        }
    }
}

/// Runtime player progress for the current run.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerProgress {
    pub config: LivesConfig,
    pub lives: u32,
    pub score: u32,
    /// Thresholds already paid out: `floor(score / life_every_points)` after awards.
    pub score_life_tiers_claimed: u32,
    /// Seconds of invulnerability left (`0` = vulnerable).
    pub invuln_remaining: f32,
}

impl PlayerProgress {
    pub fn new(config: LivesConfig) -> Self {
        Self {
            lives: config.starting_lives,
            score: 0,
            score_life_tiers_claimed: 0,
            invuln_remaining: 0.0,
            config,
        }
    }

    pub fn is_invulnerable(&self) -> bool {
        self.invuln_remaining > 0.0
    }

    pub fn tick_invuln(&mut self, dt: f32) {
        if self.invuln_remaining > 0.0 {
            self.invuln_remaining = (self.invuln_remaining - dt).max(0.0);
        }
    }

    /// Grant invulnerability after respawn.
    pub fn start_invuln(&mut self) {
        self.invuln_remaining = self.config.invuln_secs;
    }

    /// Add score and award +1 life for each newly crossed threshold. Returns lives gained.
    pub fn add_score(&mut self, points: u32) -> u32 {
        if points == 0 || self.config.life_every_points == 0 {
            self.score = self.score.saturating_add(points);
            return 0;
        }
        self.score = self.score.saturating_add(points);
        let earned_tiers = self.score / self.config.life_every_points;
        let new_tiers = earned_tiers.saturating_sub(self.score_life_tiers_claimed);
        self.score_life_tiers_claimed = earned_tiers;
        let mut gained = 0u32;
        for _ in 0..new_tiers {
            if self.try_add_life() {
                gained += 1;
            }
        }
        gained
    }

    /// +1 life respecting optional cap. Returns whether a life was actually added.
    pub fn try_add_life(&mut self) -> bool {
        if self.config.max_lives > 0 && self.lives >= self.config.max_lives {
            return false;
        }
        self.lives = self.lives.saturating_add(1);
        true
    }

    /// Called when the active player tank is destroyed.
    /// Decrements life stock; returns `true` if a respawn should happen.
    pub fn on_tank_destroyed(&mut self) -> bool {
        if self.lives == 0 {
            return false;
        }
        self.lives -= 1;
        // starting 3 → three tanks total; after last decrement to 0, no respawn.
        self.lives > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_awards_one_life_per_crossing() {
        let mut p = PlayerProgress::new(LivesConfig {
            starting_lives: 3,
            max_lives: 10,
            life_every_points: 100,
            invuln_secs: 1.0,
            points_per_enemy: 100,
        });
        assert_eq!(p.add_score(99), 0);
        assert_eq!(p.lives, 3);
        assert_eq!(p.add_score(1), 1);
        assert_eq!(p.lives, 4);
        assert_eq!(p.add_score(200), 2); // crosses 200 and 300
        assert_eq!(p.lives, 6);
    }

    #[test]
    fn max_lives_caps_threshold_awards() {
        let mut p = PlayerProgress::new(LivesConfig {
            starting_lives: 4,
            max_lives: 5,
            life_every_points: 50,
            invuln_secs: 1.0,
            points_per_enemy: 100,
        });
        assert_eq!(p.add_score(200), 1); // only one fits under cap
        assert_eq!(p.lives, 5);
    }
}
