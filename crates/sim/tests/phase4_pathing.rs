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
fn shoots_brick_in_dead_end_corridor() {
    // Surrounded by brick + one steel: no move → must blast a brick.
    let mut world = world_from(
        "\
#####
#XS##
#####
..P..
..E..
",
    );
    push_enemy(&mut world, 10, 1.5, 1.5);
    {
        let e = world.tanks.iter_mut().find(|t| t.id == 10).unwrap();
        e.dir = Dir::Left;
        e.ai_stuck_secs = 1.0;
        e.ai_prev_x = e.x;
        e.ai_prev_y = e.y;
        e.fire.config.fire_interval = 0.05;
        e.fire.autofire = true;
        e.fire.cooldown = 0.0;
    }
    {
        let p = world.player_mut().unwrap();
        p.dir = Dir::Right;
    }
    for _ in 0..300 {
        if let Some(e) = world.tanks.iter_mut().find(|t| t.id == 10) {
            e.fire.config.fire_interval = 0.05;
            e.fire.autofire = true;
        }
        {
            let p = world.player_mut().unwrap();
            p.dir = Dir::Right;
        }
        world.step(&SimInput::default(), 1.0 / 60.0);
        if world.map.get(0, 1) == Some(Tile::Empty)
            || world.map.get(1, 0) == Some(Tile::Empty)
            || world.map.get(1, 2) == Some(Tile::Empty)
        {
            break;
        }
    }
    let cleared = world.map.get(0, 1) == Some(Tile::Empty)
        || world.map.get(1, 0) == Some(Tile::Empty)
        || world.map.get(1, 2) == Some(Tile::Empty);
    assert!(
        cleared,
        "dead-end enemy must blast a brick to escape the pocket"
    );
}

#[test]
fn takes_only_open_exit_in_corridor() {
    // Tip: Left/Up/Right blocked by brick, Down open → must move Down (not face the wall).
    let mut world = world_from(
        "\
#####
#X###
#.###
#...#
#.P.#
#.E.#
",
    );
    push_enemy(&mut world, 10, 1.5, 1.5);
    {
        let e = world.tanks.iter_mut().find(|t| t.id == 10).unwrap();
        e.dir = Dir::Right;
        e.ai_stuck_secs = 0.8;
        e.ai_prev_x = e.x;
        e.ai_prev_y = e.y;
        e.ai_commit_dir = Some(Dir::Right);
        e.ai_commit_left = 1.0;
        e.ai_commit_cell = Some((1, 1));
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert_eq!(e.move_dir, Some(Dir::Down), "must take the only open exit");
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
    {
        let p = world.player_mut().unwrap();
        p.dir = Dir::Right; // not aiming up the pocket
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
        moved > 0.25,
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
fn corridor_blasts_side_brick_not_up_down_twitch() {
    // Vertical lane: only Up/Down free, bricks Left/Right → must face a side brick.
    let mut world = world_from(
        "\
###
#.#
#X#
#.#
#P#
#E#
",
    );
    push_enemy(&mut world, 10, 1.5, 2.5);
    {
        let e = world.tanks.iter_mut().find(|t| t.id == 10).unwrap();
        e.dir = Dir::Up;
        e.move_dir = Some(Dir::Up);
        e.ai_stuck_secs = 0.5;
        e.ai_prev_x = e.x;
        e.ai_prev_y = e.y;
        e.ai_commit_dir = Some(Dir::Up);
        e.ai_commit_left = 1.0;
        e.ai_commit_cell = Some((1, 2));
        e.fire.config.fire_interval = 0.05;
        e.fire.autofire = true;
        e.fire.cooldown = 0.0;
    }
    {
        let p = world.player_mut().unwrap();
        p.dir = Dir::Right; // not aiming at the corridor
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert!(
        matches!(e.dir, Dir::Left | Dir::Right),
        "corridor must face side brick, got {:?}",
        e.dir
    );
    assert!(e.move_dir.is_none(), "must stop for brick shot, not twitch");

    for _ in 0..360 {
        if let Some(e) = world.tanks.iter_mut().find(|t| t.id == 10) {
            e.x = 1.5;
            e.y = 2.5;
            e.fire.config.fire_interval = 0.05;
            e.fire.autofire = true;
        }
        {
            let p = world.player_mut().unwrap();
            p.dir = Dir::Right;
        }
        world.step(&SimInput::default(), 1.0 / 60.0);
        if world.map.get(0, 2) == Some(Tile::Empty) || world.map.get(2, 2) == Some(Tile::Empty)
        {
            break;
        }
    }
    let cleared = world.map.get(0, 2) == Some(Tile::Empty)
        || world.map.get(2, 2) == Some(Tile::Empty);
    assert!(cleared, "must destroy a side brick to leave the corridor");
}

#[test]
fn threat_overrides_move_commit_to_strafe() {
    // Committed downward move, player aims up the lane → must break commit and strafe.
    let mut world = world_from(
        "\
X..............
...............
...............
...............
...............
......P........
",
    );
    push_enemy(&mut world, 10, 6.5, 2.5);
    {
        let e = world.tanks.iter_mut().find(|t| t.id == 10).unwrap();
        e.dir = Dir::Down;
        e.move_dir = Some(Dir::Down);
        e.ai_commit_dir = Some(Dir::Down);
        e.ai_commit_left = 1.0;
        e.ai_commit_cell = Some((6, 2));
        e.ai_prev_x = e.x;
        e.ai_prev_y = e.y;
    }
    {
        let p = world.player_mut().unwrap();
        p.x = 6.5;
        p.y = 5.5;
        p.dir = Dir::Up;
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert!(
        matches!(e.move_dir, Some(Dir::Left) | Some(Dir::Right)),
        "threat must interrupt commit and strafe (move={:?} dir={:?})",
        e.move_dir,
        e.dir
    );
}

#[test]
fn faces_player_when_aimed_and_cannot_strafe() {
    // Vertical pocket: player aims up the lane, no side step → turn and return fire.
    let mut world = world_from(
        "\
###
#X#
#.#
#P#
#E#
",
    );
    push_enemy(&mut world, 10, 1.5, 1.5);
    {
        let e = world.tanks.iter_mut().find(|t| t.id == 10).unwrap();
        e.dir = Dir::Up;
        e.ai_commit_dir = Some(Dir::Up);
        e.ai_commit_left = 1.0;
        e.ai_commit_cell = Some((1, 1));
        e.ai_prev_x = e.x;
        e.ai_prev_y = e.y;
    }
    {
        let p = world.player_mut().unwrap();
        p.x = 1.5;
        p.y = 3.5;
        p.dir = Dir::Up;
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert_eq!(e.dir, Dir::Down, "must face the aiming player");
    assert!(e.move_dir.is_none(), "must stop to return fire");
}

#[test]
fn strafes_when_player_aims_at_enemy() {
    let mut world = world_from(
        "\
X..............
...............
...............
......E........
......P........
",
    );
    push_enemy(&mut world, 10, 6.5, 0.5);
    {
        let e = world.tanks.iter_mut().find(|t| t.id == 10).unwrap();
        e.x = 6.5;
        e.y = 2.5;
        e.ai_prev_x = 6.5;
        e.ai_prev_y = 2.5;
        e.ai_commit_dir = None;
        e.ai_commit_left = 0.0;
        e.ai_commit_cell = None;
    }
    {
        let p = world.player_mut().unwrap();
        p.x = 6.5;
        p.y = 4.5;
        p.dir = Dir::Up; // aiming straight at the enemy
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 10).unwrap();
    assert!(
        matches!(e.move_dir, Some(Dir::Left) | Some(Dir::Right)),
        "must strafe off the aim line, got {:?}",
        e.move_dir
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
