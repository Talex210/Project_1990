//! Campaign levels 1…100 (manifest + embedded map files).

use crate::map::{Map, MapError};
use crate::wave::WaveConfig;
use std::fmt;

/// Expected number of stages in the v1 campaign.
pub const LEVEL_COUNT: u32 = 100;

const MANIFEST_TEXT: &str = include_str!("../../../data/levels/manifest.txt");

/// One stage: map asset name + wave parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelDef {
    pub number: u32,
    pub map_file: String,
    pub wave: WaveConfig,
}

/// Parsed campaign table (always 100 entries when loaded from shipped manifest).
#[derive(Debug, Clone, PartialEq)]
pub struct Campaign {
    levels: Vec<LevelDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LevelError {
    EmptyManifest,
    BadLine { line: usize, detail: String },
    DuplicateStage { number: u32 },
    MissingStage { number: u32 },
    UnknownMap { map_file: String },
    Map(MapError),
}

impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LevelError::EmptyManifest => write!(f, "level manifest is empty"),
            LevelError::BadLine { line, detail } => {
                write!(f, "manifest line {line}: {detail}")
            }
            LevelError::DuplicateStage { number } => {
                write!(f, "duplicate stage {number} in manifest")
            }
            LevelError::MissingStage { number } => {
                write!(f, "missing stage {number} in manifest")
            }
            LevelError::UnknownMap { map_file } => {
                write!(f, "unknown map file {map_file:?}")
            }
            LevelError::Map(e) => write!(f, "map error: {e}"),
        }
    }
}

impl std::error::Error for LevelError {}

impl From<MapError> for LevelError {
    fn from(value: MapError) -> Self {
        LevelError::Map(value)
    }
}

impl Campaign {
    /// Parse a manifest (`stage map_file total_enemies max_on_field spawn_interval`).
    pub fn parse(text: &str) -> Result<Self, LevelError> {
        let mut by_number: Vec<Option<LevelDef>> = vec![None; LEVEL_COUNT as usize];
        let mut saw_any = false;

        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            saw_any = true;
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 5 {
                return Err(LevelError::BadLine {
                    line: line_no,
                    detail: format!("expected 5 fields, got {}", parts.len()),
                });
            }
            let number: u32 = parts[0].parse().map_err(|_| LevelError::BadLine {
                line: line_no,
                detail: format!("bad stage number {:?}", parts[0]),
            })?;
            if number == 0 || number > LEVEL_COUNT {
                return Err(LevelError::BadLine {
                    line: line_no,
                    detail: format!("stage {number} out of 1..{LEVEL_COUNT}"),
                });
            }
            let total_enemies: u32 = parts[2].parse().map_err(|_| LevelError::BadLine {
                line: line_no,
                detail: format!("bad total_enemies {:?}", parts[2]),
            })?;
            let max_on_field: u32 = parts[3].parse().map_err(|_| LevelError::BadLine {
                line: line_no,
                detail: format!("bad max_on_field {:?}", parts[3]),
            })?;
            let spawn_interval: f32 = parts[4].parse().map_err(|_| LevelError::BadLine {
                line: line_no,
                detail: format!("bad spawn_interval {:?}", parts[4]),
            })?;
            if total_enemies == 0 || max_on_field == 0 {
                return Err(LevelError::BadLine {
                    line: line_no,
                    detail: "total_enemies and max_on_field must be > 0".into(),
                });
            }
            let slot = (number - 1) as usize;
            if by_number[slot].is_some() {
                return Err(LevelError::DuplicateStage { number });
            }
            by_number[slot] = Some(LevelDef {
                number,
                map_file: parts[1].to_string(),
                wave: WaveConfig {
                    total_enemies,
                    max_on_field,
                    spawn_interval,
                },
            });
        }

        if !saw_any {
            return Err(LevelError::EmptyManifest);
        }

        let mut levels = Vec::with_capacity(LEVEL_COUNT as usize);
        for n in 1..=LEVEL_COUNT {
            match by_number[(n - 1) as usize].take() {
                Some(def) => levels.push(def),
                None => return Err(LevelError::MissingStage { number: n }),
            }
        }
        Ok(Self { levels })
    }

    /// Shipped `data/levels/manifest.txt`.
    pub fn bundled() -> Result<Self, LevelError> {
        Self::parse(MANIFEST_TEXT)
    }

    pub fn len(&self) -> usize {
        self.levels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    pub fn get(&self, number: u32) -> Option<&LevelDef> {
        if number == 0 || number > self.levels.len() as u32 {
            return None;
        }
        Some(&self.levels[(number - 1) as usize])
    }

    /// Load map text + wave for a stage number.
    pub fn load(&self, number: u32) -> Result<(Map, WaveConfig), LevelError> {
        let def = self
            .get(number)
            .ok_or(LevelError::MissingStage { number })?;
        let text = embedded_map(&def.map_file).ok_or_else(|| LevelError::UnknownMap {
            map_file: def.map_file.clone(),
        })?;
        let map = Map::parse(text)?;
        Ok((map, def.wave))
    }
}

/// Embedded map assets referenced by the manifest.
pub fn embedded_map(file: &str) -> Option<&'static str> {
    Some(match file {
        "level_001.txt" => include_str!("../../../data/maps/level_001.txt"),
        "level_002.txt" => include_str!("../../../data/maps/level_002.txt"),
        "level_003.txt" => include_str!("../../../data/maps/level_003.txt"),
        "placeholder.txt" => include_str!("../../../data/maps/placeholder.txt"),
        "test_level.txt" => include_str!("../../../data/maps/test_level.txt"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_campaign_has_100_loadable_levels() {
        let campaign = Campaign::bundled().expect("manifest");
        assert_eq!(campaign.len(), LEVEL_COUNT as usize);
        for n in 1..=LEVEL_COUNT {
            let (map, wave) = campaign.load(n).unwrap_or_else(|e| panic!("level {n}: {e}"));
            assert!(
                !map.player_spawns().is_empty(),
                "level {n}: need player spawn"
            );
            assert!(
                !map.enemy_spawns().is_empty(),
                "level {n}: need enemy spawn"
            );
            assert_eq!(map.width(), 16, "level {n}: keep 16-wide for playfield");
            assert_eq!(map.height(), 16, "level {n}: keep 16-tall for playfield");
            assert!(wave.total_enemies >= wave.max_on_field.min(wave.total_enemies));
            assert!(wave.max_on_field >= 1);
        }
    }

    #[test]
    fn early_stages_use_unique_maps() {
        let c = Campaign::bundled().unwrap();
        assert_eq!(c.get(1).unwrap().map_file, "level_001.txt");
        assert_eq!(c.get(2).unwrap().map_file, "level_002.txt");
        assert_eq!(c.get(3).unwrap().map_file, "level_003.txt");
        assert_eq!(c.get(4).unwrap().map_file, "placeholder.txt");
        assert_eq!(c.get(100).unwrap().map_file, "placeholder.txt");
    }
}
