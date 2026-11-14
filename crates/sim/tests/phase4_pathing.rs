//! Phase 4.2 gate: soft path through brick, shoot blockers, anti-stuck.

use project1990_sim::{
    BonusCatalog, Dir, EnemyGoal, Map, SimConfig, SimInput, Team, Tile, WaveConfig, World,
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
        ai_prev_x: x,
        ai_prev_y: y,
        ai_commit_dir: None,
        ai_commit_left: 0.0,
            ai_commit_cell: None,
    });
}

#[test]
fn shoots_brick_blocking_path_to_base() {
    // Only path to HQ is through brick. Player offset so Cover does not steal the goal.
    let mut world = world_from(
        "\
#X#..
###..
#.#..
#E#..
...P.
",
    );
    push_enemy(&mut world, 10, 4.5, 4.5); // dummy slot 0 (away)
    push_enemy(&mut world, 11, 1.5, 0.5); // slot 1 → Base
    {
        let e = world.tanks.iter_mut().find(|t| t.id == 11).unwrap();
        e.fire.config.fire_interval = 0.05;
        e.fire.config.max_bullets = 2;
        e.fire.cooldown = 0.0;
        e.fire.autofire = true;
    }
    {
        let p = world.player_mut().unwrap();
        p.dir = Dir::Right; // not aiming up the corridor
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 11).unwrap();
    assert_eq!(e.ai_goal, Some(EnemyGoal::Base));
    assert_eq!(e.dir, Dir::Down, "must face the brick toward HQ");
    assert!(e.move_dir.is_none(), "must not drive into brick");

    for _ in 0..300 {
        {
            let p = world.player_mut().unwrap();
            p.dir = Dir::Right;
        }
        // Keep rapid fire — AI may refresh FireController from mods.
        if let Some(e) = world.tanks.iter_mut().find(|t| t.id == 11) {
            e.fire.config.fire_interval = 0.05;
            e.fire.autofire = true;
        }
        world.step(&SimInput::default(), 1.0 / 60.0);
        if world.map.get(1, 1) == Some(Tile::Empty) {
            break;
        }
    }
    assert_eq!(
        world.map.get(1, 1),
        Some(Tile::Empty),
        "enemy soft-path must clear the brick"
    );
}

#[test]
fn stuck_near_steel_abandons_base_for_player() {
    // Approach cell next to steel HQ ring — cannot progress → hunt player.
    let mut world = world_from(
        "\
.....
.....
.P...
.SSS.
.SES.
",
    );
    push_enemy(&mut world, 10, 1.5, 0.5); // far slot 0
    push_enemy(&mut world, 11, 2.5, 2.5); // above steel ring, slot 1 Base
    // Accumulate stuck over real ticks (watchdog must see no movement).
    for _ in 0..90 {
        {
            let e = world.tanks.iter_mut().find(|t| t.id == 11).unwrap();
            e.x = 2.5;
            e.y = 2.5;
            e.ai_prev_x = 2.5;
            e.ai_prev_y = 2.5;
        }
        world.step(&SimInput::default(), 1.0 / 60.0);
        let e = world.tanks.iter().find(|t| t.id == 11).unwrap();
        if e.ai_goal == Some(EnemyGoal::Player) {
            return;
        }
    }
    let e = world.tanks.iter().find(|t| t.id == 11).unwrap();
    assert_eq!(
        e.ai_goal,
        Some(EnemyGoal::Player),
        "stuck on steel ring must switch to player (stuck={})",
        e.ai_stuck_secs
    );
}

#[test]
fn never_idles_when_open_step_exists() {
    // Against the top border with open cells below — must move, not stand shooting forever.
    let mut world = world_from(
        "\
###
#X#
#.#
...
.P.
.E.
",
    );
    push_enemy(&mut world, 10, 1.5, 1.5);
    {
        let e = world.tanks.iter_mut().find(|t| t.id == 10).unwrap();
        e.dir = Dir::Up;
        e.ai_stuck_secs = 0.7;
        e.ai_prev_x = e.x;
        e.ai_prev_y = e.y;
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert!(
        e.move_dir.is_some(),
        "must pick an open step instead of idling at the edge"
    );
}

#[test]
fn leaves_map_edge_corridor_over_time() {
    let mut world = world_from(
        "\
###..
#X#..
#.#..
.....
..P..
..E..
",
    );
    push_enemy(&mut world, 10, 1.5, 1.5);
    let y0 = 1.5;
    for _ in 0..240 {
        world.step(&SimInput::default(), 1.0 / 60.0);
    }
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert!(
        e.y > y0 + 0.3,
        "must leave the top pocket (y={})",
        e.y
    );
}

#[test]
fn unstick_leaves_dead_end_corner() {
    // Dead-end pocket: after stuck timeout the tank must shift position.
    let mut world = world_from(
        "\
###..
#X#..
#.#..
.....
..P..
..E..
",
    );
    push_enemy(&mut world, 10, 1.5, 1.5);
    let (x0, y0) = (1.5, 1.5);
    for _ in 0..300 {
        world.step(&SimInput::default(), 1.0 / 60.0);
    }
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    let moved = (e.x - x0).hypot(e.y - y0);
    assert!(
        moved > 0.4,
        "anti-stuck must leave the corner (moved={moved}, at {},{})",
        e.x,
        e.y
    );
}

#[test]
fn smoke_not_spinning_in_place_forever() {
    // Open map: committed heading must keep facing flips low.
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
    push_enemy(&mut world, 10, 0.5, 0.5);
    let mut flips = 0u32;
    let mut last = world.tanks.iter().find(|t| t.id == 10).unwrap().dir;
    for _ in 0..180 {
        world.step(&SimInput::default(), 1.0 / 60.0);
        let d = world.tanks.iter().find(|t| t.id == 10).unwrap().dir;
        if d != last {
            flips += 1;
            last = d;
        }
    }
    assert!(
        flips < 25,
        "enemy flipped facing {flips} times in 3s — still spinning"
    );
}

#[test]
fn commit_holds_direction_across_frames() {
    let mut world = world_from(
        "\
X..............
...............
...............
...............
......E........
......P........
",
    );
    push_enemy(&mut world, 10, 0.5, 0.5);
    world.step(&SimInput::default(), 1.0 / 60.0);
    let first = world.tanks.iter().find(|t| t.id == 10).unwrap().dir;
    let mut same = 0u32;
    for _ in 0..20 {
        world.step(&SimInput::default(), 1.0 / 60.0);
        let d = world.tanks.iter().find(|t| t.id == 10).unwrap().dir;
        if d == first {
            same += 1;
        }
    }
    assert!(
        same >= 15,
        "commit should hold heading (~0.45s); same={same}/20 first={first:?}"
    );
}
