//! Phase 4.1 gate: enemy goals base / player / bonus / cover / bypass.

use project1990_sim::{
    BonusCatalog, Dir, EnemyGoal, Map, SimConfig, SimInput, Team, WaveConfig, World,
};

fn catalog() -> BonusCatalog {
    BonusCatalog::parse(include_str!("../../../data/bonuses/catalog.cfg")).unwrap()
}

fn world_from(map: &str) -> World {
    World::new(
        Map::parse(map).unwrap(),
        SimConfig {
            bonuses: catalog(),
            clear_grace_secs: 0.0,
            wave: WaveConfig {
                total_enemies: 0,
                max_on_field: 0,
                spawn_interval: 99.0,
            },
            ..SimConfig::default()
        },
    )
}

fn push_enemy(world: &mut World, id: u64, x: f32, y: f32) {
    world.tanks.push(project1990_sim::Tank {
        id,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x,
        y,
        dir: Dir::Down,
        fire: project1990_sim::FireController::new(world.config.enemy_fire),
        move_dir: None,
        mods: Default::default(),
        carries_bonus: false,
        ai_goal: None,
            ai_stuck_secs: 0.0,
            ai_prev_x: 0.0,
            ai_prev_y: 0.0,
            ai_commit_dir: None,
            ai_commit_left: 0.0,
            ai_commit_cell: None,
    });
}

#[test]
fn enemy_targets_base_when_no_bonus() {
    let mut world = world_from(
        "\
X....
.....
.....
..E..
..P..
",
    );
    // slot 0 → Player, slot 1 → Base
    push_enemy(&mut world, 10, 0.5, 0.5);
    push_enemy(&mut world, 11, 4.5, 0.5);
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 11).unwrap();
    assert_eq!(e.ai_goal, Some(EnemyGoal::Base));
    assert!(e.move_dir.is_some(), "must drive toward HQ");
}

#[test]
fn slot_zero_hunts_player() {
    let mut world = world_from(
        "\
X....
.....
.....
..E..
..P..
",
    );
    push_enemy(&mut world, 10, 0.5, 0.5);
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    // First enemy is slot 0 → Player
    assert_eq!(e.ai_goal, Some(EnemyGoal::Player));
}

#[test]
fn bonus_outranks_base_and_player() {
    let mut world = world_from(
        "\
X....
.....
..P..
..E..
",
    );
    push_enemy(&mut world, 10, 0.5, 0.5);
    world.spawn_pickup_at("helmet", 4.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert_eq!(e.ai_goal, Some(EnemyGoal::Bonus));
}

#[test]
fn hunting_keeps_moving_when_player_aims() {
    let mut world = world_from(
        "\
.....
.#...
.X...
.....
.P...
",
    );
    push_enemy(&mut world, 10, 1.5, 2.5);
    {
        let p = world.player_mut().unwrap();
        p.x = 1.5;
        p.y = 4.5;
        p.dir = Dir::Up;
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    // Cover is no longer a parking goal — keep hunting Base/Player.
    assert!(
        matches!(e.ai_goal, Some(EnemyGoal::Base) | Some(EnemyGoal::Player) | Some(EnemyGoal::Bypass)),
        "got {:?}",
        e.ai_goal
    );
}

#[test]
fn bypass_when_primary_path_sealed() {
    // Pocket above a full steel wall: no grid path to player/base → Bypass flank.
    let mut world = world_from(
        "\
.X...
SSSSS
.....
..E..
..P..
",
    );
    push_enemy(&mut world, 10, 1.5, 0.5);
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert_eq!(
        e.ai_goal,
        Some(EnemyGoal::Bypass),
        "sealed lane must pick Bypass"
    );
}

#[test]
fn enemy_moves_south_toward_base_over_time() {
    let mut world = world_from(
        "\
X....
.....
.....
.....
..E..
..P..
",
    );
    push_enemy(&mut world, 11, 0.5, 0.5); // will be slot 0 if alone — use two
    // Only one enemy → slot 0 hunts player at bottom; still moves down.
    let y0 = 0.5;
    for _ in 0..180 {
        world.step(&SimInput::default(), 1.0 / 60.0);
    }
    let e = world.tanks.iter().find(|t| t.id == 11).unwrap();
    assert!(
        e.y > y0 + 0.5,
        "enemy should advance down the map (y0={y0}, y={})",
        e.y
    );
}

#[test]
fn frozen_enemies_clear_goals() {
    let mut world = world_from(
        "\
X....
.....
..P..
",
    );
    push_enemy(&mut world, 10, 0.5, 0.5);
    world.bonuses.freeze_enemy_secs = 5.0;
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert_eq!(e.ai_goal, None);
    assert!(e.move_dir.is_none());
}
