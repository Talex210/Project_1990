//! Phase 4.3 gate: difficulty curve 1…100 + Basic/Fast/Armor/Boss profiles.

use project1990_sim::{
    BonusCatalog, DifficultyTable, EnemyGoal, EnemyKind, GoalBias, Map, SimConfig, SimInput, Team,
    WaveConfig, World,
};

fn catalog() -> BonusCatalog {
    BonusCatalog::parse(include_str!("../../../data/bonuses/catalog.cfg")).unwrap()
}

fn difficulty() -> DifficultyTable {
    DifficultyTable::parse(include_str!("../../../data/ai/difficulty.cfg")).unwrap()
}

fn world_at(stage: u32, map: &str) -> World {
    World::new(
        Map::parse(map).unwrap(),
        SimConfig {
            bonuses: catalog(),
            clear_grace_secs: 0.0,
            stage,
            difficulty: difficulty(),
            wave: WaveConfig {
                total_enemies: 20,
                max_on_field: 4,
                spawn_interval: 0.0,
            },
            ..SimConfig::default()
        },
    )
}

#[test]
fn shipped_difficulty_cfg_loads() {
    let t = difficulty();
    assert_eq!(t.fast.goal_bias, GoalBias::PreferPlayer);
    assert_eq!(t.armor.goal_bias, GoalBias::PreferBase);
    assert!(t.armor.armored);
    assert!(t.boss.upgrade >= 3);
}

#[test]
fn level_100_harder_than_level_1() {
    let t = difficulty();
    let a = t.for_level(1);
    let b = t.for_level(100);
    assert!(b.speed_mul > a.speed_mul, "speed should rise");
    assert!(
        b.fire_interval < a.fire_interval,
        "fire interval should shrink"
    );
    assert!(b.armor_weight > a.armor_weight);
    assert!(b.boss_weight > a.boss_weight);
    assert!(b.fast_weight > a.fast_weight);
    assert!(b.hunt_period <= a.hunt_period);
}

#[test]
fn fast_faster_armor_slower_than_basic() {
    let t = difficulty();
    let basic = t.enemy_move_speed(2.0, 10, EnemyKind::Basic);
    let fast = t.enemy_move_speed(2.0, 10, EnemyKind::Fast);
    let armor = t.enemy_move_speed(2.0, 10, EnemyKind::Armor);
    let boss = t.enemy_move_speed(2.0, 10, EnemyKind::Boss);
    assert!(fast > basic, "fast={fast} basic={basic}");
    assert!(armor < basic, "armor={armor} basic={basic}");
    assert!(boss < armor, "boss={boss} armor={armor}");
}

#[test]
fn stage1_mix_is_mostly_basic() {
    let t = difficulty();
    let p = t.for_level(1);
    let mut counts = [0u32; 4];
    for i in 0..200 {
        match t.pick_kind(&p, i) {
            EnemyKind::Basic => counts[0] += 1,
            EnemyKind::Fast => counts[1] += 1,
            EnemyKind::Armor => counts[2] += 1,
            EnemyKind::Boss => counts[3] += 1,
        }
    }
    assert!(
        counts[0] > 150,
        "stage 1 should be mostly Basic, got {counts:?}"
    );
    assert_eq!(counts[2], 0, "no Armor on stage 1");
    assert_eq!(counts[3], 0, "no Boss on stage 1");
}

#[test]
fn stage100_mix_includes_fast_armor_boss() {
    let t = difficulty();
    let p = t.for_level(100);
    let mut saw = [false; 4];
    for i in 0..200 {
        match t.pick_kind(&p, i) {
            EnemyKind::Basic => saw[0] = true,
            EnemyKind::Fast => saw[1] = true,
            EnemyKind::Armor => saw[2] = true,
            EnemyKind::Boss => saw[3] = true,
        }
    }
    assert!(saw[1], "Fast must appear on late stages");
    assert!(saw[2], "Armor must appear on late stages");
    assert!(saw[3], "Boss must appear on late stages");
}

#[test]
fn spawn_applies_armor_profile() {
    // Force Armor by using a table that only weights Armor.
    let mut diff = DifficultyTable::default();
    diff.basic_weight_l1 = 0;
    diff.basic_weight_l100 = 0;
    diff.fast_weight_l1 = 0;
    diff.fast_weight_l100 = 0;
    diff.armor_weight_l1 = 100;
    diff.armor_weight_l100 = 100;
    diff.boss_weight_l1 = 0;
    diff.boss_weight_l100 = 0;

    let mut world = World::new(
        Map::parse(
            "\
E....
.....
..P..
..X..
",
        )
        .unwrap(),
        SimConfig {
            bonuses: catalog(),
            clear_grace_secs: 0.0,
            stage: 1,
            difficulty: diff,
            wave: WaveConfig {
                total_enemies: 1,
                max_on_field: 1,
                spawn_interval: 0.0,
            },
            ..SimConfig::default()
        },
    );
    world.step(&SimInput::default(), 1.0 / 60.0);
    let e = world
        .tanks
        .iter()
        .find(|t| t.team == Team::Enemy)
        .expect("enemy spawned");
    assert_eq!(e.enemy_kind, EnemyKind::Armor);
    assert!(e.mods.armored);
    assert!(e.mods.upgrade >= 2);
    assert!(e.move_speed < world.config.tank_speed);
}

#[test]
fn armor_goal_biases_to_base_fast_to_player() {
    let map = "\
X....
.....
.....
..E..
..P..
";
    // Armor
    let mut armor_world = world_at(50, map);
    armor_world.tanks.retain(|t| t.team != Team::Enemy);
    armor_world.tanks.push(project1990_sim::Tank {
        id: 50,
        team: Team::Enemy,
        is_ally: false,
        enemy_kind: EnemyKind::Armor,
        move_speed: 1.0,
        x: 0.5,
        y: 0.5,
        dir: project1990_sim::Dir::Down,
        fire: project1990_sim::FireController::new(armor_world.config.enemy_fire),
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
    armor_world.step(&SimInput::default(), 1.0 / 60.0);
    let a = armor_world.tanks.iter().find(|t| t.id == 50).unwrap();
    assert_eq!(a.ai_goal, Some(EnemyGoal::Base));

    // Fast — slot would prefer base (slot 0 alone is hunter for Basic; Fast forces player)
    let mut fast_world = world_at(50, map);
    fast_world.tanks.retain(|t| t.team != Team::Enemy);
    // Push a dummy so Fast is slot 1 (Basic slot-logic would pick Base).
    fast_world.tanks.push(project1990_sim::Tank {
        id: 49,
        team: Team::Enemy,
        is_ally: false,
        enemy_kind: EnemyKind::Basic,
        move_speed: 1.0,
        x: 4.5,
        y: 0.5,
        dir: project1990_sim::Dir::Down,
        fire: project1990_sim::FireController::new(fast_world.config.enemy_fire),
        move_dir: None,
        mods: Default::default(),
        carries_bonus: false,
        ai_goal: None,
        ai_stuck_secs: 0.0,
        ai_prev_x: 4.5,
        ai_prev_y: 0.5,
        ai_commit_dir: None,
        ai_commit_left: 0.0,
        ai_commit_cell: None,
    });
    fast_world.tanks.push(project1990_sim::Tank {
        id: 51,
        team: Team::Enemy,
        is_ally: false,
        enemy_kind: EnemyKind::Fast,
        move_speed: 3.0,
        x: 0.5,
        y: 0.5,
        dir: project1990_sim::Dir::Down,
        fire: project1990_sim::FireController::new(fast_world.config.enemy_fire),
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
    fast_world.step(&SimInput::default(), 1.0 / 60.0);
    let f = fast_world.tanks.iter().find(|t| t.id == 51).unwrap();
    assert_eq!(
        f.ai_goal,
        Some(EnemyGoal::Player),
        "Fast at slot 1 must still hunt player"
    );
}

#[test]
fn points_for_fast_between_basic_and_armor() {
    let cfg = project1990_sim::ScoreConfig::parse(include_str!("../../../data/score.cfg")).unwrap();
    assert_eq!(cfg.enemy_fast, 150);
    assert!(cfg.enemy_basic < cfg.enemy_fast);
    assert!(cfg.enemy_fast < cfg.enemy_armor);
    assert_eq!(cfg.points_for(EnemyKind::Fast), 150);
}
