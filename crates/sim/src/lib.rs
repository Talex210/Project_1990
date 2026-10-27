//! Headless Battle City–style simulation for Project 1990.
//!
//! No GPU/window dependencies — unit-testable and ready for a future dedicated server.

mod fire;
mod map;
mod tile;
mod wave;
mod world;

pub use fire::{FireConfig, FireController};
pub use map::{Map, MapError, SpawnPoint};
pub use tile::Tile;
pub use wave::{WaveConfig, WaveState};
pub use world::{
    Bullet, Dir, RoundOutcome, SimConfig, SimInput, Tank, Team, World, DEFAULT_TICK_HZ,
};

/// Library version (workspace package version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
