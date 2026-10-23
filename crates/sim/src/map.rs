//! Tile grid + text map loading.

use crate::tile::Tile;
use std::fmt;

/// Where a player (or later enemy) should spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnPoint {
    pub x: u32,
    pub y: u32,
}

/// Rectangular tile map. Origin is top-left; `y` grows downward (row-major).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map {
    width: u32,
    height: u32,
    tiles: Vec<Tile>,
    player_spawns: Vec<SpawnPoint>,
    enemy_spawns: Vec<SpawnPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    Empty,
    InconsistentRowLengths { line: usize, expected: usize, got: usize },
    UnknownChar { line: usize, column: usize, ch: char },
    OutOfBounds { x: u32, y: u32 },
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::Empty => write!(f, "map is empty"),
            MapError::InconsistentRowLengths {
                line,
                expected,
                got,
            } => write!(
                f,
                "line {line}: expected width {expected}, got {got}"
            ),
            MapError::UnknownChar { line, column, ch } => {
                write!(f, "unknown map char {ch:?} at {line}:{column}")
            }
            MapError::OutOfBounds { x, y } => write!(f, "tile ({x},{y}) out of bounds"),
        }
    }
}

impl std::error::Error for MapError {}

impl Map {
    pub fn new(width: u32, height: u32, fill: Tile) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            tiles: vec![fill; len],
            player_spawns: Vec::new(),
            enemy_spawns: Vec::new(),
        }
    }

    /// Parse a text map.
    ///
    /// Lines are rows top→bottom. Characters:
    /// `.` / ` ` empty, `#` brick, `S` steel, `~`/`W` water, `F`/`T` forest,
    /// `I` ice, `E` base, `P` player spawn, `X` enemy spawn (empty + marker).
    /// Blank lines and lines starting with `//` are ignored.
    pub fn parse(text: &str) -> Result<Self, MapError> {
        let mut rows: Vec<Vec<Tile>> = Vec::new();
        let mut spawns: Vec<SpawnPoint> = Vec::new();
        let mut enemy_spawns: Vec<SpawnPoint> = Vec::new();

        for (line_idx, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            if line.trim().is_empty() || line.trim_start().starts_with("//") {
                continue;
            }
            let mut row = Vec::with_capacity(line.len());
            for (col, ch) in line.chars().enumerate() {
                let tile = Tile::from_char(ch).ok_or(MapError::UnknownChar {
                    line: line_idx + 1,
                    column: col + 1,
                    ch,
                })?;
                if ch == 'P' {
                    spawns.push(SpawnPoint {
                        x: col as u32,
                        y: rows.len() as u32,
                    });
                }
                if ch == 'X' {
                    enemy_spawns.push(SpawnPoint {
                        x: col as u32,
                        y: rows.len() as u32,
                    });
                }
                row.push(tile);
            }
            if let Some(first) = rows.first() {
                if row.len() != first.len() {
                    return Err(MapError::InconsistentRowLengths {
                        line: line_idx + 1,
                        expected: first.len(),
                        got: row.len(),
                    });
                }
            }
            rows.push(row);
        }

        if rows.is_empty() {
            return Err(MapError::Empty);
        }

        let height = rows.len() as u32;
        let width = rows[0].len() as u32;
        let mut tiles = Vec::with_capacity((width * height) as usize);
        for row in rows {
            tiles.extend(row);
        }

        Ok(Self {
            width,
            height,
            tiles,
            player_spawns: spawns,
            enemy_spawns,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn player_spawns(&self) -> &[SpawnPoint] {
        &self.player_spawns
    }

    pub fn enemy_spawns(&self) -> &[SpawnPoint] {
        &self.enemy_spawns
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            Some((y as usize) * (self.width as usize) + (x as usize))
        } else {
            None
        }
    }

    pub fn get(&self, x: u32, y: u32) -> Option<Tile> {
        self.index(x, y).map(|i| self.tiles[i])
    }

    pub fn set(&mut self, x: u32, y: u32, tile: Tile) -> Result<(), MapError> {
        let i = self
            .index(x, y)
            .ok_or(MapError::OutOfBounds { x, y })?;
        self.tiles[i] = tile;
        Ok(())
    }

    /// All tiles overlapping an axis-aligned box in tile space (center + half extents).
    pub fn tiles_overlapping(&self, cx: f32, cy: f32, half_w: f32, half_h: f32) -> Vec<(u32, u32, Tile)> {
        let min_x = (cx - half_w).floor() as i32;
        let max_x = (cx + half_w).ceil() as i32 - 1;
        let min_y = (cy - half_h).floor() as i32;
        let max_y = (cy + half_h).ceil() as i32 - 1;

        let mut out = Vec::new();
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                if self.in_bounds(x, y) {
                    let ux = x as u32;
                    let uy = y as u32;
                    if let Some(t) = self.get(ux, uy) {
                        out.push((ux, uy, t));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_map() {
        let map = Map::parse(
            "\
###
#P#
#E#
",
        )
        .unwrap();
        assert_eq!(map.width(), 3);
        assert_eq!(map.height(), 3);
        assert_eq!(map.get(1, 1), Some(Tile::Empty));
        assert_eq!(map.get(1, 2), Some(Tile::Base));
        assert_eq!(map.player_spawns(), &[SpawnPoint { x: 1, y: 1 }]);
    }
}
