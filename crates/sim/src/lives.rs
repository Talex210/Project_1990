//! Player lives, score thresholds → extra life, invulnerability after respawn.

use std::fmt;

/// Data-driven lives / score-life rules (see `docs/SCORE.md`, `data/score.cfg`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LivesConfig {
    pub starting_lives: u32,
    /// Hard cap after threshold bonuses (`0` = no cap).
    pub max_lives: u32,
    /// Every this many run points → +1 life (classic draft: 20_000).
    pub life_every_points: u32,
    /// Seconds of invulnerability after respawn.
    pub invuln_secs: f32,
}

impl Default for LivesConfig {
    fn default() -> Self {
        Self {
            starting_lives: 3,
            max_lives: 9,
            life_every_points: 20_000,
            invuln_secs: 3.0,
        }
    }
}

impl LivesConfig {
    /// Parse lives keys from `data/score.cfg` (score keys are ignored).
    pub fn parse(text: &str) -> Result<Self, LivesParseError> {
        let mut cfg = Self::default();
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, val) = line
                .split_once('=')
                .ok_or(LivesParseError::BadLine { line: line_no + 1 })?;
            let key = key.trim();
            let val = val.trim();
            match key {
                "starting_lives" => {
                    cfg.starting_lives = parse_u32(val, line_no + 1, key)?;
                }
                "max_lives" => {
                    cfg.max_lives = parse_u32(val, line_no + 1, key)?;
                }
                "life_every_points" => {
                    cfg.life_every_points = parse_u32(val, line_no + 1, key)?;
                }
                "invuln_secs" => {
                    cfg.invuln_secs = parse_f32(val, line_no + 1, key)?.max(0.0);
                }
                // Shared file with ScoreConfig — ignore point table keys.
                "enemy_basic"
                | "enemy_armor"
                | "enemy_boss"
                | "streak_bonus_per_step"
                | "clear_bonus"
                | "clear_no_death_bonus"
                | "clear_base_safe_bonus" => {}
                other => {
                    return Err(LivesParseError::UnknownKey {
                        line: line_no + 1,
                        key: other.to_string(),
                    });
                }
            }
        }
        Ok(cfg)
    }
}

fn parse_u32(val: &str, line: usize, key: &str) -> Result<u32, LivesParseError> {
    val.parse().map_err(|_| LivesParseError::BadValue {
        line,
        key: key.to_string(),
    })
}

fn parse_f32(val: &str, line: usize, key: &str) -> Result<f32, LivesParseError> {
    val.parse().map_err(|_| LivesParseError::BadValue {
        line,
        key: key.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LivesParseError {
    BadLine { line: usize },
    BadValue { line: usize, key: String },
    UnknownKey { line: usize, key: String },
}

impl fmt::Display for LivesParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LivesParseError::BadLine { line } => {
                write!(f, "score.cfg lives line {line}: expected key=value")
            }
            LivesParseError::BadValue { line, key } => {
                write!(f, "score.cfg lives line {line}: bad value for {key}")
            }
            LivesParseError::UnknownKey { line, key } => {
                write!(f, "score.cfg lives line {line}: unknown key {key}")
            }
        }
    }
}

impl std::error::Error for LivesParseError {}

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
    /// Star / gun level carried across stages until death (0…4; ★4 = forest).
    pub tank_upgrade: u8,
    /// Ship carried across stages until hit by an enemy bullet (no timer).
    pub ship: bool,
}

impl PlayerProgress {
    pub fn new(config: LivesConfig) -> Self {
        Self {
            lives: config.starting_lives,
            score: 0,
            score_life_tiers_claimed: 0,
            invuln_remaining: 0.0,
            tank_upgrade: 0,
            ship: false,
            config,
        }
    }

    /// Stars / ship are lost on death.
    pub fn reset_tank_power(&mut self) {
        self.tank_upgrade = 0;
        self.ship = false;
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
        if points == 0 {
            return 0;
        }
        if self.config.life_every_points == 0 {
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
        self.reset_tank_power();
        if self.lives == 0 {
            return false;
        }
        self.lives -= 1;
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
        });
        assert_eq!(p.add_score(99), 0);
        assert_eq!(p.lives, 3);
        assert_eq!(p.add_score(1), 1);
        assert_eq!(p.lives, 4);
        assert_eq!(p.add_score(200), 2);
        assert_eq!(p.lives, 6);
    }

    #[test]
    fn max_lives_caps_threshold_awards() {
        let mut p = PlayerProgress::new(LivesConfig {
            starting_lives: 4,
            max_lives: 5,
            life_every_points: 50,
            invuln_secs: 1.0,
        });
        assert_eq!(p.add_score(200), 1);
        assert_eq!(p.lives, 5);
    }

    #[test]
    fn shipped_score_cfg_lives_are_classic_scale() {
        let cfg = LivesConfig::parse(include_str!("../../../data/score.cfg")).unwrap();
        assert_eq!(cfg.starting_lives, 3);
        assert_eq!(cfg.max_lives, 9);
        assert_eq!(cfg.life_every_points, 20_000);
        assert!((cfg.invuln_secs - 3.0).abs() < f32::EPSILON);
    }
}
