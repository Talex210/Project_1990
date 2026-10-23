//! Tile kinds and v1 collision / destruction rules.

/// One cell on the playfield.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tile {
    Empty,
    Brick,
    Steel,
    Water,
    Forest,
    Ice,
    Base,
}

impl Tile {
    /// Whether a tank may occupy this tile (v1 skeleton).
    ///
    /// Water blocks tanks until a ship/boat bonus (phase 3).
    /// Forest and ice are passable; forest does not block LOS for bullets.
    pub fn blocks_tank(self) -> bool {
        matches!(self, Tile::Brick | Tile::Steel | Tile::Water | Tile::Base)
    }

    /// Whether a bullet is stopped by this tile.
    pub fn blocks_bullet(self) -> bool {
        matches!(self, Tile::Brick | Tile::Steel | Tile::Base)
    }

    /// Whether a normal bullet destroys this tile on hit.
    ///
    /// Steel is immune. Forest is not cleared by normal fire (letter-mod later).
    pub fn destroyed_by_bullet(self) -> bool {
        matches!(self, Tile::Brick | Tile::Base)
    }

    /// Parse a single map character.
    pub fn from_char(c: char) -> Option<Self> {
        Some(match c {
            '.' | ' ' | 'P' | 'X' => Tile::Empty,
            '#' | 'B' => Tile::Brick,
            'S' => Tile::Steel,
            '~' | 'W' => Tile::Water,
            'F' | 'T' => Tile::Forest,
            'I' => Tile::Ice,
            'E' => Tile::Base,
            _ => return None,
        })
    }

    /// Character used when serializing maps (canonical form).
    pub fn to_char(self) -> char {
        match self {
            Tile::Empty => '.',
            Tile::Brick => '#',
            Tile::Steel => 'S',
            Tile::Water => '~',
            Tile::Forest => 'F',
            Tile::Ice => 'I',
            Tile::Base => 'E',
        }
    }
}
