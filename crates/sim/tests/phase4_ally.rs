//! Phase 4.4 gate: AI ally P2 — spawn, roles, score, respawn without GAME OVER.

use project1990_sim::{
    ally_face_toward, AllyRole, BonusCatalog, Dir, Map, SimConfig, SimInput, Team, WaveConfig,
    World,
};

fn catalog() -> BonusCatalog {
    BonusCatalog::parse(include_str!("../../../data/bonuses/catalog.cfg")).unwrap()
}

fn ally_world(map: &str) -> World {
    World::new(
        Map::parse(map).unwrap(),
        SimConfig {
            bonuses: catalog(),
            clear_grace_secs: 0.0,
            ally_enabled: true,
            wave: WaveConfig {
                total_enemies: 0,
                max_on_field: 0,
                spawn_interval: 99.0,
            },
            ..SimConfig::default()
        },
    )
}

#[test]
fn ally_spawns_with_player() {
    let world = ally_world(
        "\
.....
.P.P.
..E..
",
    );
    assert!(world.ally.enabled);
    assert_eq!(world.ally.lives, world.config.lives.starting_lives);
    assert!(world.player().is_some());
    assert!(world.ally_tank().is_some());
    assert!(world.ally_tank().unwrap().is_ally);
    assert!(!world.player().unwrap().is_ally);
}

#[test]
fn ally_offset_spawn_when_single_p() {
    let world = ally_world(
        "\
.....
..P..
..E..
",
    );
    let p = world.player().unwrap();
    let a = world.ally_tank().unwrap();
    assert!((a.x - p.x).abs() > 0.5, "ally should offset from single P");
}

#[test]
fn ally_kill_awards_shared_score() {
    let mut world = ally_world(
        "\
X....
.....
.P.P.
",
    );
    // Place a stationary enemy in front of the ally and force a kill via bullet.
    let ally_id = world.ally_tank().unwrap().id;
    let ex = world.ally_tank().unwrap().x;
    let ey = world.ally_tank().unwrap().y - 1.0;
    world.tanks.push(project1990_sim::Tank {
        id: 80,
        team: Team::Enemy,
        is_ally: false,
        enemy_kind: Default::default(),
        move_speed: 0.0,
        x: ex,
        y: ey,
        dir: Dir::Down,
        fire: project1990_sim::FireController::new(world.config.enemy_fire),
        move_dir: None,
        mods: Default::default(),
        carries_bonus: false,
        ai_goal: None,
        ai_stuck_secs: 0.0,
        ai_prev_x: ex,
        ai_prev_y: ey,
        ai_commit_dir: None,
        ai_commit_left: 0.0,
        ai_commit_cell: None,
    });
    world.wave = project1990_sim::WaveState::new(WaveConfig {
        total_enemies: 1,
        max_on_field: 1,
        spawn_interval: 99.0,
    });
    world.wave.remaining_to_spawn = 0;
    let score0 = world.progress.score;
    world.bullets.push(project1990_sim::Bullet {
        id: 900,
        owner_id: ally_id,
        team: Team::Player,
        x: ex,
        y: ey,
        dir: Dir::Up,
        speed: 6.0,
    });
    // Direct hit resolution.
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(
        world.progress.score > score0,
        "ally kill must add shared score"
    );
    assert!(world.tanks.iter().all(|t| t.id != 80));
}

#[test]
fn ally_death_respawns_without_defeat() {
    let mut world = ally_world(
        "\
.....
.P.P.
..E..
",
    );
    let lives0 = world.progress.lives;
    let ally_lives0 = world.ally.lives;
    let ally_id = world.ally_tank().unwrap().id;
    let (ax, ay) = {
        let a = world.ally_tank().unwrap();
        (a.x, a.y)
    };
    world.ally.invuln_remaining = 0.0;
    if let Some(a) = world.tanks.iter_mut().find(|t| t.id == ally_id) {
        a.mods.invuln = false;
    }
    world.bullets.push(project1990_sim::Bullet {
        id: 901,
        owner_id: 999,
        team: Team::Enemy,
        x: ax,
        y: ay,
        dir: Dir::Down,
        speed: 6.0,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.outcome, project1990_sim::RoundOutcome::Playing);
    assert_eq!(world.progress.lives, lives0, "player lives untouched");
    assert_eq!(world.ally.lives, ally_lives0 - 1);
    assert!(world.ally_tank().is_some(), "ally respawned");
}

#[test]
fn ally_guards_when_enemy_near_base() {
    let mut world = ally_world(
        "\
.....
.P.P.
.....
..E..
",
    );
    // Enemy sitting on the eagle cell neighbourhood.
    let (bx, by) = (2.5, 3.5);
    world.tanks.push(project1990_sim::Tank {
        id: 81,
        team: Team::Enemy,
        is_ally: false,
        enemy_kind: Default::default(),
        move_speed: 0.0,
        x: bx,
        y: by,
        dir: Dir::Up,
        fire: project1990_sim::FireController::new(world.config.enemy_fire),
        move_dir: None,
        mods: Default::default(),
        carries_bonus: false,
        ai_goal: None,
        ai_stuck_secs: 0.0,
        ai_prev_x: bx,
        ai_prev_y: by,
        ai_commit_dir: None,
        ai_commit_left: 0.0,
        ai_commit_cell: None,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.ally.role, AllyRole::Guard);
}

#[test]
fn ally_hunts_when_field_clear_of_base_threat() {
    let mut world = ally_world(
        "\
.....
.....
.....
.....
.....
.....
.....
.P.P.
..E..
",
    );
    // Keep human next to ally so Support leash does not fire.
    let (ax, ay) = {
        let a = world.ally_tank().unwrap();
        (a.x, a.y)
    };
    {
        let p = world.player_mut().unwrap();
        p.x = ax - 1.0;
        p.y = ay;
    }
    // Far from HQ (bottom) and from player — Hunt, not Guard/Support.
    world.tanks.push(project1990_sim::Tank {
        id: 82,
        team: Team::Enemy,
        is_ally: false,
        enemy_kind: Default::default(),
        move_speed: 0.0,
        x: 0.5,
        y: 0.5,
        dir: Dir::Left,
        fire: project1990_sim::FireController::new(world.config.enemy_fire),
        move_dir: None,
        mods: Default::default(),
        carries_bonus: false,
        ai_goal: None,
        ai_stuck_secs: 0.0,
        ai_prev_x: 0.5,
        ai_prev_y: 0.5,
        ai_commit_dir: None,
        ai_commit_left: 0.0,
        ai_commit_cell: None,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.ally.role, AllyRole::Hunt);
}

#[test]
fn ally_face_helper_works() {
    assert_eq!(ally_face_toward(0.0, 0.0, 2.0, 0.0), Some(Dir::Right));
    assert_eq!(ally_face_toward(0.0, 0.0, 0.0, 2.0), Some(Dir::Down));
}
