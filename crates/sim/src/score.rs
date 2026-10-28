//! Score table + style bonuses (see `docs/SCORE.md`, `data/score.cfg`).

use std::fmt;

/// Enemy weight for scoring (spawn variety grows later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EnemyKind {
    #[default]
    Basic,
    Armor,
    Boss,
}

/// Data-driven point values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreConfig {
    pub enemy_basic: u32,
    pub enemy_armor: u32,
    pub enemy_boss: u32,
    /// Extra points per streak step after the first kill in a chain.
    pub streak_bonus_per_step: u32,
    pub clear_bonus: u32,
    pub clear_no_death_bonus: u32,
}

impl Default for ScoreConfig {
    fn default() -> Self {
        Self {
            enemy_basic: 100,
            enemy_armor: 200,
            enemy_boss: 400,
            streak_bonus_per_step: 50,
            clear_bonus: 500,
            clear_no_death_bonus: 1000,
        }
    }
}

impl ScoreConfig {
    pub fn points_for(&self, kind: EnemyKind) -> u32 {
        match kind {
            EnemyKind::Basic => self.enemy_basic,
            EnemyKind::Armor => self.enemy_armor,
            EnemyKind::Boss => self.enemy_boss,
        }
    }

    /// Parse `data/score.cfg`-style text (`key=value`, `#` comments).
    pub fn parse(text: &str) -> Result<Self, ScoreParseError> {
        let mut cfg = Self::default();
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, val) = line
                .split_once('=')
                .ok_or(ScoreParseError::BadLine { line: line_no + 1 })?;
            let key = key.trim();
            let val: u32 = val.trim().parse().map_err(|_| ScoreParseError::BadValue {
                line: line_no + 1,
                key: key.to_string(),
            })?;
            match key {
                "enemy_basic" => cfg.enemy_basic = val,
                "enemy_armor" => cfg.enemy_armor = val,
                "enemy_boss" => cfg.enemy_boss = val,
                "streak_bonus_per_step" => cfg.streak_bonus_per_step = val,
                "clear_bonus" => cfg.clear_bonus = val,
                "clear_no_death_bonus" => cfg.clear_no_death_bonus = val,
                // Ignored legacy key (base-safe bonus removed — classic: base hit = game over).
                "clear_base_safe_bonus" => {}
                other => {
                    return Err(ScoreParseError::UnknownKey {
                        line: line_no + 1,
                        key: other.to_string(),
                    });
                }
            }
        }
        Ok(cfg)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoreParseError {
    BadLine { line: usize },
    BadValue { line: usize, key: String },
    UnknownKey { line: usize, key: String },
}

impl fmt::Display for ScoreParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScoreParseError::BadLine { line } => {
                write!(f, "score.cfg line {line}: expected key=value")
            }
            ScoreParseError::BadValue { line, key } => {
                write!(f, "score.cfg line {line}: bad value for {key}")
            }
            ScoreParseError::UnknownKey { line, key } => {
                write!(f, "score.cfg line {line}: unknown key {key}")
            }
        }
    }
}

impl std::error::Error for ScoreParseError {}

/// Why points were just awarded (for HUD flash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScorePopup {
    Kill {
        kind: EnemyKind,
        base: u32,
        streak_bonus: u32,
        streak: u32,
    },
    Clear { clear: u32, no_death: u32 },
}

impl ScorePopup {
    pub fn total(&self) -> u32 {
        match self {
            ScorePopup::Kill {
                base,
                streak_bonus,
                ..
            } => base + streak_bonus,
            ScorePopup::Clear { clear, no_death } => clear + no_death,
        }
    }

    pub fn label(&self) -> String {
        match self {
            ScorePopup::Kill {
                streak,
                streak_bonus,
                base,
                ..
            } => {
                if *streak_bonus > 0 {
                    format!(
                        "+{} (kill)  streak x{streak} +{streak_bonus}",
                        base + streak_bonus
                    )
                } else {
                    format!("+{base} (kill)")
                }
            }
            ScorePopup::Clear { clear, no_death } => {
                let mut parts = vec![format!("clear +{clear}")];
                if *no_death > 0 {
                    parts.push(format!("no-death +{no_death}"));
                }
                parts.join("  |  ")
            }
        }
    }
}

/// Kill streak + level style flags for the current round.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyleState {
    /// Consecutive kills without dying (resets on player death).
    pub streak: u32,
    /// Deaths this level (for clear-no-death bonus).
    pub deaths_this_level: u32,
    pub last_popup: Option<ScorePopup>,
}

impl StyleState {
    pub fn on_player_death(&mut self) {
        self.streak = 0;
        self.deaths_this_level = self.deaths_this_level.saturating_add(1);
        self.last_popup = None;
    }

    /// Points for one enemy kill including streak step bonus.
    pub fn award_kill(&mut self, cfg: &ScoreConfig, kind: EnemyKind) -> (u32, ScorePopup) {
        self.streak = self.streak.saturating_add(1);
        let base = cfg.points_for(kind);
        let streak_bonus = if self.streak > 1 {
            (self.streak - 1) * cfg.streak_bonus_per_step
        } else {
            0
        };
        let popup = ScorePopup::Kill {
            kind,
            base,
            streak_bonus,
            streak: self.streak,
        };
        self.last_popup = Some(popup.clone());
        (base + streak_bonus, popup)
    }

    /// End-of-level style bonuses (call once on victory).
    pub fn award_clear(&mut self, cfg: &ScoreConfig) -> (u32, ScorePopup) {
        let clear = cfg.clear_bonus;
        let no_death = if self.deaths_this_level == 0 {
            cfg.clear_no_death_bonus
        } else {
            0
        };
        let popup = ScorePopup::Clear { clear, no_death };
        self.last_popup = Some(popup.clone());
        (clear + no_death, popup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_score_cfg_defaults_and_overrides() {
        let cfg = ScoreConfig::parse(
            "\
# comment
enemy_basic=150
streak_bonus_per_step=25
",
        )
        .unwrap();
        assert_eq!(cfg.enemy_basic, 150);
        assert_eq!(cfg.streak_bonus_per_step, 25);
        assert_eq!(cfg.clear_bonus, 500);
    }

    #[test]
    fn streak_adds_step_bonus() {
        let cfg = ScoreConfig {
            streak_bonus_per_step: 50,
            enemy_basic: 100,
            ..Default::default()
        };
        let mut style = StyleState::default();
        let (p1, _) = style.award_kill(&cfg, EnemyKind::Basic);
        assert_eq!(p1, 100);
        let (p2, _) = style.award_kill(&cfg, EnemyKind::Basic);
        assert_eq!(p2, 150);
        style.on_player_death();
        assert_eq!(style.streak, 0);
        assert_eq!(style.deaths_this_level, 1);
    }

    #[test]
    fn clear_bonuses_respect_deaths() {
        let cfg = ScoreConfig::default();
        let mut style = StyleState::default();
        let (pts, pop) = style.award_clear(&cfg);
        assert_eq!(pts, 500 + 1000);
        match pop {
            ScorePopup::Clear { no_death, .. } => assert_eq!(no_death, 1000),
            _ => panic!("expected clear"),
        }

        let mut dirty = StyleState {
            deaths_this_level: 1,
            ..Default::default()
        };
        let (pts2, _) = dirty.award_clear(&cfg);
        assert_eq!(pts2, 500);
    }
}
