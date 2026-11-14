//! Phase 3.1–3.2 gate: data-driven catalog, letters, enemies take bonuses.

use project1990_sim::{
    BonusCatalog, Dir, EffectKind, FireConfig, Map, PickupWho, SimConfig, SimInput, Team, Tile,
    WaveConfig, World,
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
            ..Default::default()
        },
    )
}

#[test]
fn shipped_catalog_parses_all_classic_letters() {
    let cat = BonusCatalog::parse(CATALOG).unwrap();
    assert_eq!(cat.drop_every_kills, 4);
    for id in [
        "helmet", "clock", "shovel", "grenade", "star", "gun", "tank", "ship",
    ] {
        assert!(cat.get(id).is_some(), "missing {id}");
    }
    assert!(cat.get("forest").is_none(), "no separate forest letter");
    let gun = cat.get("gun").unwrap();
    assert_eq!(gun.letter, 'P');
    assert_eq!(gun.effects, vec![EffectKind::MaxUpgrade]);
    assert_eq!(cat.get("star").unwrap().max_stacks, 4);
    assert_eq!(cat.get("ship").unwrap().duration_secs, 0.0);
    assert_eq!(cat.get("tank").unwrap().who, PickupWho::Both);
    assert_eq!(
        cat.get("clock").unwrap().duration_secs,
        8.0,
        "clock freeze duration (playtest; classic ~10s)"
    );
}

#[test]
fn helmet_pickup_grants_invuln() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("helmet", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.bonuses.pickups.is_empty(), "pickup collected");
    assert!(world.progress.is_invulnerable() || world.player().unwrap().mods.invuln);
    let labels = world
        .bonuses
        .hud_active_labels(Team::Player, Some(world.player_tank_id()));
    assert!(
        labels.iter().any(|s| s.contains("Helmet")),
        "expected Helmet in {labels:?}"
    );
}

#[test]
fn star_pickup_upgrades_fire() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    let base_interval = world.config.fire.fire_interval;
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("star", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    let p = world.player().unwrap();
    assert_eq!(p.mods.upgrade, 1);
    assert!(p.fire.config.fire_interval < base_interval);
}

#[test]
fn gun_sets_max_upgrade_and_breaks_steel() {
    let mut world = world_with_catalog(
        "\
S....
.....
..P..
",
    );
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("gun", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.player().unwrap().mods.upgrade, 3);
    assert!(!world.player().unwrap().mods.forest_clear);

    {
        let p = world.player_mut().unwrap();
        p.dir = Dir::Up;
        p.x = 0.5;
        p.y = 2.5;
        p.fire.config.fire_interval = 0.0;
        p.fire.autofire = true;
        p.fire.cooldown = 0.0;
    }
    for _ in 0..40 {
        world.step(
            &SimInput {
                fire_held: true,
                ..Default::default()
            },
            1.0 / 60.0,
        );
    }
    assert_eq!(world.map.get(0, 0), Some(Tile::Empty), "★3/P breaks steel");
}

#[test]
fn fourth_star_or_second_gun_unlocks_forest() {
    let mut world = world_with_catalog(
        "\
F....
.....
..P..
",
    );
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    for _ in 0..4 {
        world.spawn_pickup_at("star", px, py).unwrap();
        world.step(&SimInput::default(), 1.0 / 60.0);
    }
    assert_eq!(world.player().unwrap().mods.upgrade, 4);
    assert!(world.player().unwrap().mods.forest_clear);

    {
        let p = world.player_mut().unwrap();
        p.dir = Dir::Up;
        p.x = 0.5;
        p.y = 2.5;
        p.fire.config.fire_interval = 0.0;
        p.fire.autofire = true;
        p.fire.cooldown = 0.0;
    }
    for _ in 0..40 {
        world.step(
            &SimInput {
                fire_held: true,
                ..Default::default()
            },
            1.0 / 60.0,
        );
    }
    assert_eq!(world.map.get(0, 0), Some(Tile::Empty), "★4 clears forest");

    // Two guns from scratch.
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("gun", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.player().unwrap().mods.upgrade, 3);
    world.spawn_pickup_at("gun", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.player().unwrap().mods.upgrade, 4);
    assert!(world.player().unwrap().mods.forest_clear);
}

#[test]
fn tank_letter_adds_life() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    let lives = world.progress.lives;
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("tank", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.progress.lives, lives + 1);
}

#[test]
fn enemy_tank_letter_armors_all_enemies() {
    // Tank 1990: enemy picks T → all on-field enemies gain armor.
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 70,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 0.5,
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
    world.tanks.push(project1990_sim::Tank {
        id: 71,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 4.5,
        y: 0.5,
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
    world.spawn_pickup_at("tank", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.bonuses.pickups.is_empty());
    let a = world.tanks.iter().find(|t| t.id == 70).unwrap();
    let b = world.tanks.iter().find(|t| t.id == 71).unwrap();
    assert!(a.mods.armored && b.mods.armored);
    assert!(a.mods.upgrade >= 2 && b.mods.upgrade >= 2);
}

#[test]
fn shovel_fortify_ejects_tank_standing_in_ring() {
    // Base with open ring; tank stands where steel will appear.
    let mut world = world_with_catalog(
        "\
.....
..P..
..E..
.....
",
    );
    // Place enemy on cell left of base (1,2) — fortify ring cell.
    assert_eq!(world.map.get(2, 2), Some(Tile::Base));
    world.tanks.push(project1990_sim::Tank {
        id: 72,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 1.5,
        y: 2.5,
        dir: Dir::Right,
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
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("shovel", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.bonuses.fortify_secs > 0.0);
    assert_eq!(world.map.get(1, 2), Some(Tile::Steel));
    let e = world.tanks.iter().find(|t| t.id == 72).unwrap();
    let half = world.config.tank_half;
    let blocked = world
        .map
        .tiles_overlapping(e.x, e.y, half, half)
        .iter()
        .any(|(_, _, t)| t.blocks_tank());
    assert!(
        !blocked,
        "tank must be ejected off fortify steel (at {}, {})",
        e.x,
        e.y
    );
}

#[test]
fn clock_freezes_enemies() {
    let mut world = world_with_catalog(
        "\
X....
.....
..P..
",
    );
    // Manual enemy.
    let eid = {
        // spawn via kill drop path is heavy — place enemy tank by reusing spawn_enemies wave
        world.config.wave.total_enemies = 1;
        world.wave = project1990_sim::WaveState::new(world.config.wave);
        world.step(&SimInput::default(), 0.0);
        world
            .tanks
            .iter()
            .find(|t| t.team == Team::Enemy)
            .map(|t| t.id)
    };
    // If spawn failed (interval), force one enemy:
    let eid = eid.unwrap_or_else(|| {
        let id = 42;
        world.tanks.push(project1990_sim::Tank {
            id,
            team: Team::Enemy,
            enemy_kind: Default::default(),
            x: 0.5,
            y: 0.5,
            dir: Dir::Down,
            fire: project1990_sim::FireController::new(world.config.enemy_fire),
            move_dir: Some(Dir::Down),
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
        id
    });
    let _ = eid;

    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("clock", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.bonuses.freeze_enemy_secs > 0.0);
    assert!(world.bonuses.opponents_frozen(Team::Enemy));
}

#[test]
fn ship_allows_water_tiles() {
    let mut world = world_with_catalog(
        "\
.....
~~~~~
..P..
",
    );
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("ship", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.player().unwrap().mods.water_walk);

    for _ in 0..30 {
        world.step(
            &SimInput {
                move_dir: Some(Dir::Up),
                ..Default::default()
            },
            1.0 / 60.0,
        );
    }
    let p = world.player().unwrap();
    assert!(p.y < 2.0, "ship should let player enter water row, y={}", p.y);
}

#[test]
fn ship_is_permanent_until_hit_and_carries_in_progress() {
    let mut world = world_with_catalog(
        "\
.....
~~~~~
..P..
",
    );
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("ship", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.progress.ship);
    assert!(world.player().unwrap().mods.water_walk);

    // Carry to a fresh world (next STAGE).
    let progress = world.progress.clone();
    let catalog = world.bonuses.catalog.clone();
    let map = world.map.clone();
    let mut next = World::new(
        map,
        SimConfig {
            bonuses: catalog,
            ..world.config.clone()
        },
    );
    next.progress = progress;
    next.reapply_player_from_progress();
    assert!(next.progress.ship);
    assert!(next.player().unwrap().mods.water_walk);

    // ★0 + ship: enemy bullet kills and strips ship from progress.
    {
        let p = next.player_mut().unwrap();
        p.mods.upgrade = 0;
    }
    next.progress.tank_upgrade = 0;
    let (px, py) = {
        let p = next.player().unwrap();
        (p.x, p.y)
    };
    next.bullets.push(project1990_sim::Bullet {
        id: 99,
        owner_id: 999,
        team: Team::Enemy,
        x: px,
        y: py,
        dir: Dir::Down,
        speed: 6.0,
    });
    next.step(&SimInput::default(), 1.0 / 60.0);
    assert!(!next.progress.ship);
}

#[test]
fn star_absorbs_hit_instead_of_death() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("star", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    world.spawn_pickup_at("star", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.player().unwrap().mods.upgrade, 2);
    world.progress.ship = true;
    world.player_mut().unwrap().mods.water_walk = true;

    let lives = world.progress.lives;
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.bullets.push(project1990_sim::Bullet {
        id: 50,
        owner_id: 999,
        team: Team::Enemy,
        x: px,
        y: py,
        dir: Dir::Up,
        speed: 6.0,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.progress.lives, lives, "star hit must not cost a life");
    assert_eq!(world.player().unwrap().mods.upgrade, 1);
    assert!(!world.progress.ship, "hit strips ship");
}

#[test]
fn field_pickup_expires_after_lifetime() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    world.bonuses.catalog.pickup_lifetime_secs = 0.5;
    world.spawn_pickup_at("helmet", 0.5, 0.5).unwrap();
    assert_eq!(world.bonuses.pickups.len(), 1);
    for _ in 0..40 {
        world.step(&SimInput::default(), 1.0 / 60.0);
    }
    assert!(
        world.bonuses.pickups.is_empty(),
        "pickup must despawn from the map"
    );
}

#[test]
fn star_power_carries_via_progress_to_respawn_tank() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.....
",
    );
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.spawn_pickup_at("gun", px, py).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.progress.tank_upgrade, 3);
    assert_eq!(world.player().unwrap().mods.upgrade, 3);

    // Simulate stage carry: rebuild tank from progress without death reset.
    let progress = world.progress.clone();
    let catalog = world.bonuses.catalog.clone();
    let map = world.map.clone();
    let mut next = World::new(
        map,
        SimConfig {
            bonuses: catalog,
            ..world.config.clone()
        },
    );
    next.progress = progress;
    next.reapply_player_from_progress();
    assert_eq!(next.player().unwrap().mods.upgrade, 3);
    assert!(next.player().unwrap().fire.config.fire_interval < next.config.fire.fire_interval);
}

#[test]
fn enemy_kill_drops_letter_from_catalog() {
    let mut world = world_with_catalog(
        "\
X....
.....
..P..
",
    );
    world.config.wave.total_enemies = 1;
    world.wave = project1990_sim::WaveState::new(world.config.wave);
    world.tanks.push(project1990_sim::Tank {
        id: 77,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 0.5,
        dir: Dir::Down,
        fire: project1990_sim::FireController::new(world.config.enemy_fire),
        move_dir: None,
        mods: Default::default(),
        carries_bonus: true,
        ai_goal: None,
            ai_stuck_secs: 0.0,
            ai_prev_x: 0.0,
            ai_prev_y: 0.0,
            ai_commit_dir: None,
            ai_commit_left: 0.0,
            ai_commit_cell: None,
    });
    world.wave.on_spawned();

    {
        let p = world.player_mut().unwrap();
        p.dir = Dir::Up;
        p.x = 0.5;
        p.y = 2.5;
        p.fire.config.fire_interval = 0.0;
        p.fire.autofire = true;
        p.fire.cooldown = 0.0;
    }
    for _ in 0..80 {
        // Pin the target — this test checks drop rules, not dodge.
        if let Some(e) = world.tanks.iter_mut().find(|t| t.id == 77) {
            e.x = 0.5;
            e.y = 0.5;
            e.move_dir = None;
        }
        world.step(
            &SimInput {
                fire_held: true,
                ..Default::default()
            },
            1.0 / 60.0,
        );
        if !world.bonuses.pickups.is_empty() {
            break;
        }
    }
    assert!(
        !world.bonuses.pickups.is_empty(),
        "killing enemy must drop a catalog letter"
    );
    let p = &world.bonuses.pickups[0];
    assert!(
        (p.x - 0.5).abs() > 0.1 || (p.y - 0.5).abs() > 0.1,
        "pickup should not stay on the death tile by default (got {}, {})",
        p.x,
        p.y
    );
}

#[test]
fn enemy_seeks_and_picks_up_helmet() {
    let mut world = world_with_catalog(
        "\
.....
X....
..P..
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 88,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 1.5,
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
    world.spawn_pickup_at("helmet", 3.5, 1.5).unwrap();
    for _ in 0..120 {
        {
            let p = world.player_mut().unwrap();
            p.dir = Dir::Down; // don't steal the bonus path with aim-strafe
        }
        world.step(&SimInput::default(), 1.0 / 60.0);
        if world.bonuses.pickups.is_empty() {
            break;
        }
    }
    assert!(
        world.bonuses.pickups.is_empty(),
        "enemy should collect the helmet"
    );
    let enemy = world.tanks.iter().find(|t| t.id == 88).unwrap();
    assert!(enemy.mods.armored, "enemy helmet must grant permanent armor");
    assert!(enemy.mods.upgrade >= 2, "armor comes with multi-hit ★");
    assert!(!enemy.mods.invuln, "enemy H must not be timed god-mode");
    assert!(!world.enemy_power_labels().is_empty());
}

#[test]
fn enemy_seeks_around_wall_to_bonus() {
    // Wall blocks the straight path; BFS must go around.
    // X # . .
    // . . . .
    // . . . H(pickup)
    // . P . .
    let mut world = world_with_catalog(
        "\
X#..
....
....
.P..
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 89,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 0.5,
        dir: Dir::Right,
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
    world.spawn_pickup_at("helmet", 3.5, 0.5).unwrap();
    {
        let p = world.player_mut().unwrap();
        p.dir = Dir::Down;
    }
    for _ in 0..400 {
        {
            let p = world.player_mut().unwrap();
            p.dir = Dir::Down;
        }
        world.step(&SimInput::default(), 1.0 / 60.0);
        if world.bonuses.pickups.is_empty() {
            break;
        }
    }
    assert!(
        world.bonuses.pickups.is_empty(),
        "enemy must path around the wall to the letter"
    );
}

#[test]
fn enemy_shovel_weakens_base_walls() {
    let mut world = world_with_catalog(
        "\
.....
..P..
.#E#.
.###.
",
    );
    assert_eq!(world.map.get(1, 2), Some(Tile::Brick));
    assert_eq!(world.map.get(2, 2), Some(Tile::Base));
    world.tanks.push(project1990_sim::Tank {
        id: 90,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 0.5,
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
    world.spawn_pickup_at("shovel", 0.5, 0.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(
        world.map.get(1, 2),
        Some(Tile::Empty),
        "enemy shovel should clear brick beside base"
    );
}

#[test]
fn enemy_clock_freezes_player() {
    let mut world = world_with_catalog(
        "\
.....
X....
..P..
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 91,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 1.5,
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
    world.spawn_pickup_at("clock", 0.5, 1.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.bonuses.freeze_player_secs > 0.0);
    assert!(world.bonuses.opponents_frozen(Team::Player));
    // Frozen player can still authorize fire.
    {
        let p = world.player_mut().unwrap();
        p.fire.set_fire_held(true);
        p.fire.cooldown = 0.0;
        assert!(p.fire.try_fire(0), "frozen player must still shoot");
    }
}

#[test]
fn enemy_clock_survives_carrier_death() {
    let mut world = world_with_catalog(
        "\
.....
X....
..P..
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 91,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 1.5,
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
    world.spawn_pickup_at("clock", 0.5, 1.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    let frozen = world.bonuses.freeze_player_secs;
    assert!(frozen > 0.0);

    // Kill the carrier — freeze must keep ticking.
    world.tanks.retain(|t| t.id != 91);
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(
        world.bonuses.freeze_player_secs > 0.0,
        "killing the clock carrier must not clear the freeze"
    );
    assert!(world.bonuses.opponents_frozen(Team::Player));
}

#[test]
fn enemy_clock_clears_when_player_dies() {
    let mut world = world_with_catalog(
        "\
.....
X....
..P..
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 92,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 1.5,
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
    world.spawn_pickup_at("clock", 0.5, 1.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(world.bonuses.freeze_player_secs > 0.0);
    assert_eq!(world.bonuses.freeze_player_by_enemy, Some(92));

    // Kill the player with an enemy bullet while frozen.
    world.progress.invuln_remaining = 0.0;
    if let Some(p) = world.player_mut() {
        p.mods.invuln = false;
        p.mods.armored = false;
        p.mods.upgrade = 0;
    }
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.bullets.push(project1990_sim::Bullet {
        id: 9001,
        owner_id: 92,
        team: Team::Enemy,
        x: px,
        y: py,
        dir: Dir::Down,
        speed: 6.0,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(
        world.bonuses.freeze_player_secs, 0.0,
        "player death must clear enemy clock freeze"
    );
    assert!(world.bonuses.freeze_player_by_enemy.is_none());
    assert!(!world.bonuses.opponents_frozen(Team::Player));
}

#[test]
fn enemy_star_breaks_steel() {
    let mut world = world_with_catalog(
        "\
S....
X....
..P..
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 94,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 1.5,
        dir: Dir::Up,
        fire: project1990_sim::FireController::new(FireConfig {
            fire_interval: 0.0,
            max_bullets: 4,
        }),
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
    world.spawn_pickup_at("star", 0.5, 1.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    let enemy = world.tanks.iter().find(|t| t.id == 94).unwrap();
    assert!(enemy.mods.upgrade >= 3, "enemy ★ unlocks steel break");
    // AI now always roams when idle — fire a ★ bullet at steel directly.
    world.tanks.retain(|t| t.team != Team::Player);
    let bid = {
        // alloc via step-safe push
        let e = world.tanks.iter().find(|t| t.id == 94).unwrap();
        assert!(e.mods.upgrade >= 3);
        9001u64
    };
    world.bullets.push(project1990_sim::Bullet {
        id: bid,
        owner_id: 94,
        team: Team::Enemy,
        x: 0.5,
        y: 1.2,
        dir: Dir::Up,
        speed: world.config.bullet_speed,
    });
    for _ in 0..30 {
        world.step(&SimInput::default(), 1.0 / 60.0);
        if world.map.get(0, 0) == Some(Tile::Empty) {
            break;
        }
    }
    assert_eq!(world.map.get(0, 0), Some(Tile::Empty), "enemy ★ breaks steel");
}

#[test]
fn enemy_gun_armors_all_enemies() {
    let mut world = world_with_catalog(
        "\
.....
X..X.
..P..
",
    );
    for (id, x) in [(95u64, 0.5), (96, 3.5)] {
        world.tanks.push(project1990_sim::Tank {
            id,
            team: Team::Enemy,
            enemy_kind: Default::default(),
            x,
            y: 1.5,
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
    world.spawn_pickup_at("gun", 0.5, 1.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    for id in [95u64, 96] {
        let e = world.tanks.iter().find(|t| t.id == id).unwrap();
        assert!(e.mods.armored, "gun must armor every enemy ({id})");
        assert!(e.mods.upgrade >= 3, "gun must grant steel break ({id})");
    }
}

#[test]
fn enemy_grenade_destroys_player_without_stars() {
    let mut world = world_with_catalog(
        "\
.....
X....
..P..
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 92,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 1.5,
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
    let lives_before = world.progress.lives;
    world.spawn_pickup_at("grenade", 0.5, 1.5).unwrap();
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert!(
        world.progress.lives < lives_before || world.player().is_none(),
        "enemy grenade should hit the player stock"
    );
}

#[test]
fn enemy_star_absorbs_player_bullet() {
    let mut world = world_with_catalog(
        "\
X....
.....
..P..
",
    );
    world.tanks.push(project1990_sim::Tank {
        id: 93,
        team: Team::Enemy,
        enemy_kind: Default::default(),
        x: 0.5,
        y: 0.5,
        dir: Dir::Down,
        fire: project1990_sim::FireController::new(world.config.enemy_fire),
        move_dir: None,
        mods: project1990_sim::TankMods {
            upgrade: 2,
            ..Default::default()
        },
    carries_bonus: false,
        ai_goal: None,
            ai_stuck_secs: 0.0,
            ai_prev_x: 0.0,
            ai_prev_y: 0.0,
            ai_commit_dir: None,
            ai_commit_left: 0.0,
            ai_commit_cell: None,
    });
    world.bullets.push(project1990_sim::Bullet {
        id: 501,
        owner_id: world.player_tank_id(),
        team: Team::Player,
        x: 0.5,
        y: 0.5,
        dir: Dir::Up,
        speed: 6.0,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    let enemy = world.tanks.iter().find(|t| t.id == 93).unwrap();
    assert_eq!(enemy.mods.upgrade, 1, "enemy star should absorb one hit");
}
