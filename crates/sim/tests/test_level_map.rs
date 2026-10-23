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
}
