//! Phase 3.4 gate: every classic letter rule + exclusive_group / cancel interactions.

use project1990_sim::{
    BonusCatalog, Bullet, Dir, EffectKind, FireConfig, Map, RoundOutcome, SimConfig, SimInput,
    TankMods, Team, Tile, WaveConfig, World,
};

const CATALOG: &str = include_str!("../../../data/bonuses/catalog.cfg");

fn world_with_catalog(map: &str) -> World {
    let catalog = BonusCatalog::parse(CATALOG).expect("shipped catalog");
    World::new(
        Map::parse(map).unwrap(),
        SimConfig {
            tank_speed: 12.0,
            bullet_speed: 30.0,
            tank_half: 0.35,
            bullet_half: 0.1,
            fire: FireConfig {
                fire_interval: 0.4,
                max_bullets: 4,
            },
            wave: WaveConfig {
                total_enemies: 0,
                max_on_field: 4,
                spawn_interval: 0.0,
            },
            bonuses: catalog,
            clear_grace_secs: 0.0,
            ..Default::default()
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

fn player_xy(world: &World) -> (f32, f32) {
    let p = world.player().unwrap();
    (p.x, p.y)
}

fn collect(world: &mut World, id: &str) {
    let (px, py) = player_xy(world);
    world.spawn_pickup_at(id, px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(
        world.bonuses.pickups.is_empty(),
        "{id} should be collected"
    );
}

// --- Catalog contract -------------------------------------------------------

#[test]
fn catalog_exclusive_groups_and_cancel_flags() {
    let cat = BonusCatalog::parse(CATALOG).unwrap();
    let h = cat.get("helmet").unwrap();
    assert_eq!(h.exclusive_group.as_deref(), Some("protect"));
    assert!(h.cancel.on_death && h.cancel.on_level_end);

    let c = cat.get("clock").unwrap();
    assert_eq!(c.exclusive_group.as_deref(), Some("tempo"));
    assert!(!c.cancel.on_death && c.cancel.on_level_end);

    let l = cat.get("shovel").unwrap();
    assert_eq!(l.exclusive_group.as_deref(), Some("base_walls"));
    assert!(l.cancel.on_level_end);

    let star = cat.get("star").unwrap();
    assert!(star.cancel.on_death);
    assert!(!star.cancel.on_level_end);
    assert_eq!(star.max_stacks, 4);

    let gun = cat.get("gun").unwrap();
    assert!(gun.cancel.on_death);
    assert_eq!(gun.effects, vec![EffectKind::MaxUpgrade]);

    let ship = cat.get("ship").unwrap();
    assert!(ship.cancel.on_death);
    assert_eq!(ship.duration_secs, 0.0);

    let tank = cat.get("tank").unwrap();
    assert_eq!(tank.effects, vec![EffectKind::ExtraLife]);
}

// --- Player letter rules ----------------------------------------------------

#[test]
fn rule_helmet_invuln_cancels_on_death() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    collect(&mut world, "helmet");
    assert!(world.bonuses.active.iter().any(|a| a.bonus_id == "helmet"));

    // Grenade bypasses invuln and still runs cancel_on_death.
    push_enemy(&mut world, 33, 0.5, 0.5);
    world.spawn_pickup_at("grenade", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(
        !world
            .bonuses
            .active
            .iter()
            .any(|a| a.bonus_id == "helmet"),
        "helmet active must cancel on death"
    );
}

#[test]
fn rule_clock_freezes_enemies_not_player_fire() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    push_enemy(&mut world, 40, 0.5, 0.5);
    collect(&mut world, "clock");
    assert!(world.bonuses.freeze_enemy_secs > 0.0);
    assert!(!world.bonuses.opponents_frozen(Team::Player));
    // Enemy move cleared while frozen.
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world.tanks.iter().find(|t| t.id == 40).unwrap();
    assert!(e.move_dir.is_none());
}

#[test]
fn rule_shovel_fortifies_then_restores() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.#E#.
.###.
",
    );
    assert_eq!(world.map.get(1, 2), Some(Tile::Brick));
    collect(&mut world, "shovel");
    assert!(world.bonuses.fortify_secs > 0.0);
    assert_eq!(world.map.get(1, 2), Some(Tile::Steel));
    // Expire fortify.
    let steps = (world.bonuses.fortify_secs * 60.0).ceil() as u32 + 2;
    for _ in 0..steps {
        world.step(&SimInput::default(), 1.0 / 60.0);
    }
    assert_eq!(world.bonuses.fortify_secs, 0.0);
    assert_eq!(world.map.get(1, 2), Some(Tile::Brick));
}

#[test]
fn rule_grenade_kills_enemies_not_player() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    push_enemy(&mut world, 41, 0.5, 0.5);
    push_enemy(&mut world, 42, 4.5, 0.5);
    let lives = world.progress.lives;
    collect(&mut world, "grenade");
    assert_eq!(world.tanks.iter().filter(|t| t.team == Team::Enemy).count(), 0);
    assert_eq!(world.progress.lives, lives);
    assert!(world.player().is_some());
}

#[test]
fn rule_star_stacks_to_four_and_resets_on_death() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    for n in 1..=4 {
        collect(&mut world, "star");
        assert_eq!(world.player().unwrap().mods.upgrade, n);
        assert_eq!(world.progress.tank_upgrade, n);
    }
    collect(&mut world, "star");
    assert_eq!(world.player().unwrap().mods.upgrade, 4, "cap at 4");

    // Death peels stars via cancel + respawn from progress... absorb peels one at a time;
    // full death at ★0. Drain stars by hits.
    world.progress.invuln_remaining = 0.0;
    if let Some(p) = world.player_mut() {
        p.mods.invuln = false;
    }
    let mut bid = 9100u64;
    while world.player().map(|p| p.mods.upgrade).unwrap_or(0) > 0 {
        let (px, py) = player_xy(&world);
        world.bullets.push(Bullet {
            id: bid,
            owner_id: 999,
            team: Team::Enemy,
            x: px,
            y: py,
            dir: Dir::Down,
            speed: 6.0,
        });
        bid += 1;
        world.step(&SimInput::default(), 1.0 / 60.0);
        world.progress.invuln_remaining = 0.0;
        if let Some(p) = world.player_mut() {
            p.mods.invuln = false;
        }
    }
    // Final lethal hit.
    let (px, py) = player_xy(&world);
    world.bullets.push(Bullet {
        id: bid,
        owner_id: 999,
        team: Team::Enemy,
        x: px,
        y: py,
        dir: Dir::Down,
        speed: 6.0,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.progress.tank_upgrade, 0);
    assert_eq!(world.player().unwrap().mods.upgrade, 0);
}

#[test]
fn rule_gun_jumps_to_star3_then_forest_on_second() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    collect(&mut world, "gun");
    assert_eq!(world.player().unwrap().mods.upgrade, 3);
    assert!(!world.player().unwrap().mods.forest_clear);
    collect(&mut world, "gun");
    assert_eq!(world.player().unwrap().mods.upgrade, 4);
    assert!(world.player().unwrap().mods.forest_clear);
}

#[test]
fn rule_tank_adds_life_for_player() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    let lives = world.progress.lives;
    collect(&mut world, "tank");
    assert_eq!(world.progress.lives, lives + 1);
}

#[test]
fn rule_ship_allows_water_until_hit() {
    let mut world = world_with_catalog(
        "\
.....
..P..
..~~.
",
    );
    collect(&mut world, "ship");
    assert!(world.progress.ship);
    assert!(world.player().unwrap().mods.water_walk);
    // Step onto water.
    {
        let p = world.player_mut().unwrap();
        p.x = 2.5;
        p.y = 2.5;
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.player().unwrap().mods.water_walk);

    world.progress.invuln_remaining = 0.0;
    if let Some(p) = world.player_mut() {
        p.mods.invuln = false;
        // Move ashore first so "standing in water keeps walk" does not mask strip.
        p.x = 0.5;
        p.y = 0.5;
        p.mods.upgrade = 1; // absorb hit instead of death
    }
    let (px, py) = player_xy(&world);
    world.bullets.push(Bullet {
        id: 9201,
        owner_id: 999,
        team: Team::Enemy,
        x: px,
        y: py,
        dir: Dir::Down,
        speed: 6.0,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(!world.progress.ship);
    assert!(!world.player().unwrap().mods.water_walk);
}

// --- Exclusive groups -------------------------------------------------------

#[test]
fn exclusive_tempo_second_clock_replaces_active_entry() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    collect(&mut world, "clock");
    let first = world.bonuses.freeze_enemy_secs;
    assert!(first > 0.0);
    // Burn some time, then re-pick — duration should refresh via max().
    for _ in 0..30 {
        world.step(&SimInput::default(), 1.0 / 60.0);
    }
    let mid = world.bonuses.freeze_enemy_secs;
    assert!(mid < first);
    collect(&mut world, "clock");
    assert!(world.bonuses.freeze_enemy_secs >= mid);
    assert!(
        world
            .bonuses
            .active
            .iter()
            .filter(|a| a.bonus_id == "clock")
            .count()
            <= 1
    );
}

#[test]
fn exclusive_protect_helmet_replaces_prior_helmet_active() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    collect(&mut world, "helmet");
    collect(&mut world, "helmet");
    assert_eq!(
        world
            .bonuses
            .active
            .iter()
            .filter(|a| a.bonus_id == "helmet")
            .count(),
        1
    );
}

#[test]
fn exclusive_base_walls_second_shovel_extends_fortify() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.#E#.
.###.
",
    );
    collect(&mut world, "shovel");
    let first = world.bonuses.fortify_secs;
    for _ in 0..60 {
        world.step(&SimInput::default(), 1.0 / 60.0);
    }
    let mid = world.bonuses.fortify_secs;
    assert!(mid < first);
    collect(&mut world, "shovel");
    assert!(world.bonuses.fortify_secs >= mid);
    assert_eq!(world.map.get(1, 2), Some(Tile::Steel));
}

// --- Level-end cancel -------------------------------------------------------

#[test]
fn cancel_level_end_clears_helmet_clock_shovel() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    world.config.wave.total_enemies = 1;
    world.wave = project1990_sim::WaveState::new(world.config.wave);
    world.config.clear_grace_secs = 0.0;
    push_enemy(&mut world, 55, 0.5, 0.5);
    world.wave.on_spawned(); // remaining_to_spawn = 0
    collect(&mut world, "helmet");
    collect(&mut world, "clock");
    collect(&mut world, "shovel");
    assert!(world.bonuses.freeze_enemy_secs > 0.0 || world.bonuses.fortify_secs > 0.0);

    if let Some(pos) = world.tanks.iter().position(|t| t.id == 55) {
        world.tanks.swap_remove(pos);
        world.wave.on_destroyed();
    }
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.outcome, RoundOutcome::Victory);
    assert_eq!(world.bonuses.freeze_enemy_secs, 0.0);
    assert_eq!(world.bonuses.fortify_secs, 0.0);
    assert!(
        !world
            .bonuses
            .active
            .iter()
            .any(|a| matches!(a.bonus_id.as_str(), "helmet" | "clock" | "shovel"))
    );
}

// --- Enemy letter rules (matrix) --------------------------------------------

#[test]
fn enemy_matrix_helmet_clock_tank_star_gun_shovel_grenade() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.#E#.
.###.
",
    );
    push_enemy(&mut world, 80, 0.5, 0.5);
    push_enemy(&mut world, 81, 4.5, 0.5);

    // H on one enemy.
    world.spawn_pickup_at("helmet", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    let h = world.tanks.iter().find(|t| t.id == 80).unwrap();
    assert!(h.mods.armored && h.mods.upgrade >= 2 && !h.mods.invuln);

    // C freezes player (can still conceptually shoot — freeze is move-only).
    world.spawn_pickup_at("clock", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.bonuses.freeze_player_secs > 0.0);

    // T armors ALL enemies (Tank 1990).
    world.spawn_pickup_at("tank", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.tanks.iter().filter(|t| t.team == Team::Enemy).all(|t| t.mods.armored));

    // ★ on picker breaks steel.
    world.spawn_pickup_at("star", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    let s = world.tanks.iter().find(|t| t.id == 80).unwrap();
    assert!(s.mods.upgrade >= TankMods::MAX_FIRE_UPGRADE);

    // P armors all + steel break.
    world.spawn_pickup_at("gun", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.tanks.iter().filter(|t| t.team == Team::Enemy).all(|t| {
        t.mods.armored && t.mods.upgrade >= TankMods::MAX_FIRE_UPGRADE
    }));

    // L weakens base ring.
    assert_eq!(world.map.get(1, 2), Some(Tile::Brick));
    world.spawn_pickup_at("shovel", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.map.get(1, 2), Some(Tile::Empty));

    // G kills player without stars.
    world.progress.invuln_remaining = 0.0;
    world.progress.tank_upgrade = 0;
    if let Some(p) = world.player_mut() {
        p.mods.invuln = false;
        p.mods.upgrade = 0;
    }
    let lives = world.progress.lives;
    let deaths = world.style.deaths_this_level;
    world.spawn_pickup_at("grenade", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(
        world.progress.lives < lives || world.style.deaths_this_level > deaths,
        "enemy grenade must destroy the player"
    );
}

#[test]
fn star_and_ship_survive_level_end_cancel_but_helmet_does_not() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    collect(&mut world, "star");
    collect(&mut world, "ship");
    collect(&mut world, "helmet");
    world.bonuses.cancel_on_level_end();
    assert_eq!(world.player().unwrap().mods.upgrade, 1);
    assert!(world.progress.ship);
    // Helmet timed active + invuln timer cleared by cancel_on_level_end for active list;
    // freeze/fortify cleared; helmet active removed if on_level_end.
    assert!(
        !world
            .bonuses
            .active
            .iter()
            .any(|a| a.bonus_id == "helmet")
    );
}
