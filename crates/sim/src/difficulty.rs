//! Data-driven enemy difficulty curve + kind profiles (phase 4.3).
//!
//! Level 1…100 params come from L1/L100 endpoints (linear lerp).
//! Kind mix and per-kind speed/fire/armor/goal bias live in `data/ai/difficulty.cfg`.

use crate::score::EnemyKind;
use std::fmt;

/// How a kind chooses Base vs Player when no bonus/bypass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GoalBias {
    /// Use `slot % hunt_period == 0` → Player.
    #[default]
    Slot,
    PreferPlayer,
    PreferBase,
}

/// Tunables for one `EnemyKind`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemyKindProfile {
    pub speed_mul: f32,
    pub fire_mul: f32,
    pub upgrade: u8,
    pub armored: bool,
    pub goal_bias: GoalBias,
}

impl Default for EnemyKindProfile {
    fn default() -> Self {
        Self {
            speed_mul: 1.0,
            fire_mul: 1.0,
            upgrade: 0,
            armored: false,
            goal_bias: GoalBias::Slot,
        }
    }
}

/// Resolved AI/combat params for one campaign stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelAiParams {
    pub speed_mul: f32,
    /// Absolute enemy fire interval (seconds) before kind `fire_mul`.
    pub fire_interval: f32,
    /// `slot % hunt_period == 0` prefers the player (Basic / Slot bias).
    pub hunt_period: u32,
    pub basic_weight: u32,
    pub fast_weight: u32,
    pub armor_weight: u32,
    pub boss_weight: u32,
}

/// Full table: curve endpoints + four kind profiles.
#[derive(Debug, Clone, PartialEq)]
pub struct DifficultyTable {
    pub speed_mul_l1: f32,
    pub speed_mul_l100: f32,
    pub fire_interval_l1: f32,
    pub fire_interval_l100: f32,
    pub hunt_period_l1: u32,
    pub hunt_period_l100: u32,
    pub basic_weight_l1: u32,
    pub basic_weight_l100: u32,
    pub fast_weight_l1: u32,
    pub fast_weight_l100: u32,
    pub armor_weight_l1: u32,
    pub armor_weight_l100: u32,
    pub boss_weight_l1: u32,
    pub boss_weight_l100: u32,
    pub basic: EnemyKindProfile,
    pub fast: EnemyKindProfile,
    pub armor: EnemyKindProfile,
    pub boss: EnemyKindProfile,
}

impl Default for DifficultyTable {
    fn default() -> Self {
        Self {
            speed_mul_l1: 0.95,
            speed_mul_l100: 1.3,
            fire_interval_l1: 2.15,
            fire_interval_l100: 0.85,
            hunt_period_l1: 3,
            hunt_period_l100: 2,
            basic_weight_l1: 100,
            basic_weight_l100: 25,
            fast_weight_l1: 6,
            fast_weight_l100: 28,
            armor_weight_l1: 0,
            armor_weight_l100: 30,
            boss_weight_l1: 0,
            boss_weight_l100: 15,
            basic: EnemyKindProfile {
                speed_mul: 1.0,
                fire_mul: 1.0,
                upgrade: 0,
                armored: false,
                goal_bias: GoalBias::Slot,
            },
            fast: EnemyKindProfile {
                speed_mul: 1.45,
                fire_mul: 0.95,
                upgrade: 0,
                armored: false,
                goal_bias: GoalBias::PreferPlayer,
            },
            armor: EnemyKindProfile {
                speed_mul: 0.72,
                fire_mul: 1.2,
                upgrade: 2,
                armored: true,
                goal_bias: GoalBias::PreferBase,
            },
            boss: EnemyKindProfile {
                speed_mul: 0.58,
                fire_mul: 0.9,
                upgrade: 3,
                armored: true,
                goal_bias: GoalBias::PreferPlayer,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DifficultyParseError {
    BadLine { line: usize },
    BadValue { line: usize, key: String },
    UnknownKey { line: usize, key: String },
    UnknownSection { line: usize, name: String },
    BadBias { line: usize, value: String },
}

impl fmt::Display for DifficultyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DifficultyParseError::BadLine { line } => {
                write!(f, "difficulty.cfg line {line}: expected key=value")
            }
            DifficultyParseError::BadValue { line, key } => {
                write!(f, "difficulty.cfg line {line}: bad value for {key}")
            }
            DifficultyParseError::UnknownKey { line, key } => {
                write!(f, "difficulty.cfg line {line}: unknown key {key}")
            }
            DifficultyParseError::UnknownSection { line, name } => {
                write!(f, "difficulty.cfg line {line}: unknown section [{name}]")
            }
            DifficultyParseError::BadBias { line, value } => {
                write!(
                    f,
                    "difficulty.cfg line {line}: bad goal_bias {value:?} (slot|player|base)"
                )
            }
        }
    }
}

impl std::error::Error for DifficultyParseError {}

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_u32(a: u32, b: u32, t: f32) -> u32 {
    let v = lerp_f32(a as f32, b as f32, t).round();
    v.max(0.0) as u32
}

fn stage_t(stage: u32) -> f32 {
    let s = stage.clamp(1, 100);
    (s - 1) as f32 / 99.0
}

impl DifficultyTable {
    pub fn parse(text: &str) -> Result<Self, DifficultyParseError> {
        let mut cfg = Self::default();
        let mut section: Option<&'static str> = None;

        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let name = rest
                    .strip_suffix(']')
                    .ok_or(DifficultyParseError::BadLine { line: line_no })?
                    .trim();
                section = Some(match name {
                    "basic" => "basic",
                    "fast" => "fast",
                    "armor" => "armor",
                    "boss" => "boss",
                    other => {
                        return Err(DifficultyParseError::UnknownSection {
                            line: line_no,
                            name: other.to_string(),
                        });
                    }
                });
                continue;
            }

            let (key, val) = line
                .split_once('=')
                .ok_or(DifficultyParseError::BadLine { line: line_no })?;
            let key = key.trim();
            let val = val.trim();

            if let Some(sec) = section {
                let profile = match sec {
                    "basic" => &mut cfg.basic,
                    "fast" => &mut cfg.fast,
                    "armor" => &mut cfg.armor,
                    "boss" => &mut cfg.boss,
                    _ => unreachable!(),
                };
                match key {
                    "speed_mul" => {
                        profile.speed_mul = parse_f32(val, line_no, key)?;
                    }
                    "fire_mul" => {
                        profile.fire_mul = parse_f32(val, line_no, key)?;
                    }
                    "upgrade" => {
                        profile.upgrade = parse_u32(val, line_no, key)? as u8;
                    }
                    "armored" => {
                        profile.armored = parse_u32(val, line_no, key)? != 0;
                    }
                    "goal_bias" => {
                        profile.goal_bias = parse_bias(val, line_no)?;
                    }
                    other => {
                        return Err(DifficultyParseError::UnknownKey {
                            line: line_no,
                            key: other.to_string(),
                        });
                    }
                }
                continue;
            }

            match key {
                "speed_mul_l1" => cfg.speed_mul_l1 = parse_f32(val, line_no, key)?,
                "speed_mul_l100" => cfg.speed_mul_l100 = parse_f32(val, line_no, key)?,
                "fire_interval_l1" => cfg.fire_interval_l1 = parse_f32(val, line_no, key)?,
                "fire_interval_l100" => cfg.fire_interval_l100 = parse_f32(val, line_no, key)?,
                "hunt_period_l1" => cfg.hunt_period_l1 = parse_u32(val, line_no, key)?.max(1),
                "hunt_period_l100" => cfg.hunt_period_l100 = parse_u32(val, line_no, key)?.max(1),
                "basic_weight_l1" => cfg.basic_weight_l1 = parse_u32(val, line_no, key)?,
                "basic_weight_l100" => cfg.basic_weight_l100 = parse_u32(val, line_no, key)?,
                "fast_weight_l1" => cfg.fast_weight_l1 = parse_u32(val, line_no, key)?,
                "fast_weight_l100" => cfg.fast_weight_l100 = parse_u32(val, line_no, key)?,
                "armor_weight_l1" => cfg.armor_weight_l1 = parse_u32(val, line_no, key)?,
                "armor_weight_l100" => cfg.armor_weight_l100 = parse_u32(val, line_no, key)?,
                "boss_weight_l1" => cfg.boss_weight_l1 = parse_u32(val, line_no, key)?,
                "boss_weight_l100" => cfg.boss_weight_l100 = parse_u32(val, line_no, key)?,
                other => {
                    return Err(DifficultyParseError::UnknownKey {
                        line: line_no,
                        key: other.to_string(),
                    });
                }
            }
        }
        Ok(cfg)
    }

    pub fn for_level(&self, stage: u32) -> LevelAiParams {
        let t = stage_t(stage);
        LevelAiParams {
            speed_mul: lerp_f32(self.speed_mul_l1, self.speed_mul_l100, t),
            fire_interval: lerp_f32(self.fire_interval_l1, self.fire_interval_l100, t),
            hunt_period: lerp_u32(self.hunt_period_l1, self.hunt_period_l100, t).max(1),
            basic_weight: lerp_u32(self.basic_weight_l1, self.basic_weight_l100, t),
            fast_weight: lerp_u32(self.fast_weight_l1, self.fast_weight_l100, t),
            armor_weight: lerp_u32(self.armor_weight_l1, self.armor_weight_l100, t),
            boss_weight: lerp_u32(self.boss_weight_l1, self.boss_weight_l100, t),
        }
    }

    pub fn profile(&self, kind: EnemyKind) -> EnemyKindProfile {
        match kind {
            EnemyKind::Basic => self.basic,
            EnemyKind::Fast => self.fast,
            EnemyKind::Armor => self.armor,
            EnemyKind::Boss => self.boss,
        }
    }

    /// Deterministic kind from wave spawn ordinal (0-based).
    pub fn pick_kind(&self, params: &LevelAiParams, spawn_index: u32) -> EnemyKind {
        let total = params
            .basic_weight
            .saturating_add(params.fast_weight)
            .saturating_add(params.armor_weight)
            .saturating_add(params.boss_weight)
            .max(1);
        let roll = spawn_index.wrapping_mul(2654435761) % total;
        let mut acc = params.basic_weight;
        if roll < acc {
            return EnemyKind::Basic;
        }
        acc = acc.saturating_add(params.fast_weight);
        if roll < acc {
            return EnemyKind::Fast;
        }
        acc = acc.saturating_add(params.armor_weight);
        if roll < acc {
            return EnemyKind::Armor;
        }
        EnemyKind::Boss
    }

    pub fn enemy_move_speed(
        &self,
        base_tank_speed: f32,
        stage: u32,
        kind: EnemyKind,
    ) -> f32 {
        let level = self.for_level(stage);
        let profile = self.profile(kind);
        base_tank_speed * level.speed_mul * profile.speed_mul
    }

    pub fn enemy_fire_interval(&self, stage: u32, kind: EnemyKind) -> f32 {
        let level = self.for_level(stage);
        let profile = self.profile(kind);
        (level.fire_interval * profile.fire_mul).max(0.05)
    }
}

fn parse_f32(val: &str, line: usize, key: &str) -> Result<f32, DifficultyParseError> {
    val.parse().map_err(|_| DifficultyParseError::BadValue {
        line,
        key: key.to_string(),
    })
}

fn parse_u32(val: &str, line: usize, key: &str) -> Result<u32, DifficultyParseError> {
    val.parse().map_err(|_| DifficultyParseError::BadValue {
        line,
        key: key.to_string(),
    })
}

fn parse_bias(val: &str, line: usize) -> Result<GoalBias, DifficultyParseError> {
    match val {
        "slot" => Ok(GoalBias::Slot),
        "player" => Ok(GoalBias::PreferPlayer),
        "base" => Ok(GoalBias::PreferBase),
        other => Err(DifficultyParseError::BadBias {
            line,
            value: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_100_harder_than_1() {
        let t = DifficultyTable::default();
        let a = t.for_level(1);
        let b = t.for_level(100);
        assert!(b.speed_mul > a.speed_mul);
        assert!(b.fire_interval < a.fire_interval);
        assert!(b.armor_weight > a.armor_weight);
        assert!(b.boss_weight >= a.boss_weight);
        assert!(b.hunt_period <= a.hunt_period);
    }

    #[test]
    fn fast_faster_than_basic() {
        let t = DifficultyTable::default();
        let basic = t.enemy_move_speed(2.0, 1, EnemyKind::Basic);
        let fast = t.enemy_move_speed(2.0, 1, EnemyKind::Fast);
        let armor = t.enemy_move_speed(2.0, 1, EnemyKind::Armor);
        assert!(fast > basic);
        assert!(armor < basic);
    }

    #[test]
    fn parse_shipped_defaults_shape() {
        let text = include_str!("../../../data/ai/difficulty.cfg");
        let t = DifficultyTable::parse(text).unwrap();
        assert_eq!(t.fast.goal_bias, GoalBias::PreferPlayer);
        assert_eq!(t.armor.goal_bias, GoalBias::PreferBase);
        assert!(t.armor.armored);
        assert!(t.boss.upgrade >= 3);
    }
}
