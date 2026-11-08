//! Integration: phase-1 data map loads and exposes expected tile kinds.

use project1990_sim::{Map, Tile};

const TEST_LEVEL: &str = include_str!("../../../data/maps/test_level.txt");

#[test]
fn test_level_parses_with_all_tile_kinds() {
    let map = Map::parse(TEST_LEVEL).expect("test_level.txt");
    assert!(map.width() >= 8);
    assert!(map.height() >= 8);
    assert!(!map.player_spawns().is_empty());
    assert!(
        map.enemy_spawns().len() >= 3,
        "need enemy spawn markers (X)"
    );

    let mut kinds = std::collections::HashSet::new();
    for y in 0..map.height() {
        for x in 0..map.width() {
            kinds.insert(map.get(x, y).unwrap());
        }
    }

    for required in [
        Tile::Empty,
        Tile::Brick,
        Tile::Steel,
        Tile::Water,
        Tile::Forest,
        Tile::Ice,
        Tile::Base,
    ] {
        assert!(
            kinds.contains(&required),
            "test map missing {required:?}"
        );
    }

    // Classic HQ: base on the last playable row, flush against the bottom border,
    // steel fort on the other sides (brick fort comes later).
    let h = map.height();
    let w = map.width();
    assert!(h >= 3 && w >= 3);
    let mut bases = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if map.get(x, y) == Some(Tile::Base) {
                bases.push((x, y));
            }
        }
    }
    assert_eq!(bases.len(), 1, "exactly one HQ");
    let (bx, by) = bases[0];
    assert_eq!(by, h - 2, "HQ flush above bottom border row");
    assert_eq!(map.get(bx, h - 1), Some(Tile::Brick), "bottom border under HQ");
    for (dx, dy) in [(-1i32, 0), (1, 0), (0, -1), (-1, -1), (1, -1)] {
        let x = bx as i32 + dx;
        let y = by as i32 + dy;
        assert!(x >= 0 && y >= 0);
        assert_eq!(
            map.get(x as u32, y as u32),
            Some(Tile::Steel),
            "steel fort at ({x},{y})"
        );
    }
}
