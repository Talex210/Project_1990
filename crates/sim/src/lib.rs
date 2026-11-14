//! Headless Battle City–style simulation for Project 1990.
//!
//! No GPU/window dependencies — unit-testable and ready for a future dedicated server.

mod ai;
mod bonus;
mod fire;
mod level;
mod lives;
mod map;
mod save;
mod score;
mod tile;
mod wave;
mod world;

pub use ai::{EnemyGoal, GoalTarget, SeekAction};
pub use bonus::{
    bullet_speed_for_upgrade, fire_for_upgrade, upgrade_breaks_steel, upgrade_clears_forest,
    ActiveBonus, BonusCatalog, BonusDef, BonusParseError, BonusPickup, BonusState, CancelFlags,
    EffectKind, PickupWho, TankMods,
};
pub use fire::{FireConfig, FireController};
pub use level::{embedded_map, Campaign, LevelDef, LevelError, LEVEL_COUNT};
pub use lives::{LivesConfig, LivesParseError, PlayerProgress};
pub use map::{Map, MapError, SpawnPoint};
pub use save::{
    default_save_path, BoardEntry, Profile, ProfileError, SaveData, SaveError, MAX_BOARD_ROWS,
    MAX_NAME_CHARS, MAX_PROFILES, SAVE_SCHEMA,
};
pub use score::{EnemyKind, ScoreConfig, ScorePopup, StyleState};
pub use tile::Tile;
pub use wave::{WaveConfig, WaveState};
pub use world::{
    Bullet, Dir, RoundOutcome, SimConfig, SimInput, SimSfx, Tank, Team, World, DEFAULT_TICK_HZ,
};

/// Library version (workspace package version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
