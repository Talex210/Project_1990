//! Phase 2.8 gate: victory/defeat, scoring, life threshold, save/records.
//!
//! Keeps the roadmap checklist as runnable integration coverage (in addition to
//! unit tests inside `world` / `lives` / `score` / `save`).

use project1990_sim::{
    BoardEntry, Campaign, Dir, FireConfig, LEVEL_COUNT, LivesConfig, Map, RoundOutcome, SaveData,
    ScoreConfig, SimConfig, SimInput, Team, WaveConfig, World, Bullet,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn open_lane_world(wave_enemies: u32, lives: LivesConfig, score: ScoreConfig) -> World {
    World::new(
        Map::parse(
            "\
X......
.......
...P...
",
        )
        .unwrap(),
        SimConfig {
            bullet_speed: 40.0,
            fire: FireConfig {
                fire_interval: 0.0,
                max_bullets: 4,
            },
            wave: WaveConfig {
                total_enemies: wave_enemies,
                max_on_field: wave_enemies.max(1),
                spawn_interval: 0.0,
            },
            lives,
            score,
            enemy_fire: FireConfig {
                fire_interval: 999.0,
                max_bullets: 0,
            },
            clear_grace_secs: 0.0,
            ..Default::default()
        },
    )
}

fn aim_player_at_spawn(world: &mut World) {
    world.step(&SimInput::default(), 1.0 / 60.0);
    let p = world.player_mut().unwrap();
    p.x = 0.5;
    p.y = 2.5;
    p.dir = Dir::Up;
    p.fire.cooldown = 0.0;
}

fn shoot_until_round_over(world: &mut World) {
    for _ in 0..60 {
        world.step(
            &SimInput {
                fire_held: true,
                ..Default::default()
            },
            1.0 / 60.0,
        );
        if world.outcome != RoundOutcome::Playing {
            break;
        }
    }
}

#[test]
fn gate_victory_on_wave_clear() {
    let mut world = open_lane_world(
        1,
        LivesConfig::default(),
        ScoreConfig {
            clear_bonus: 0,
            clear_no_death_bonus: 0,
            streak_bonus_per_step: 0,
            ..Default::default()
        },
    );
    aim_player_at_spawn(&mut world);
    shoot_until_round_over(&mut world);
    assert_eq!(world.outcome, RoundOutcome::Victory);
    assert!(world.wave.is_cleared(world.enemies_on_field()));
}

#[test]
fn gate_defeat_when_base_destroyed() {
    let mut world = World::new(
        Map::parse(
            "\
...
.P.
.E.
",
        )
        .unwrap(),
        SimConfig {
            bullet_speed: 40.0,
            fire: FireConfig {
                fire_interval: 0.0,
                max_bullets: 2,
            },
            wave: WaveConfig {
                total_enemies: 0,
                max_on_field: 4,
                spawn_interval: 0.0,
            },
            ..Default::default()
        },
    );
    {
        let p = world.player_mut().unwrap();
        p.x = 1.5;
        p.y = 1.5;
        p.dir = Dir::Down;
        p.fire.cooldown = 0.0;
    }
    shoot_until_round_over(&mut world);
    assert_eq!(world.outcome, RoundOutcome::DefeatBaseDestroyed);
}

#[test]
fn gate_defeat_when_lives_exhausted() {
    let mut world = World::new(
        Map::parse(
            "\
...
.P.
...
",
        )
        .unwrap(),
        SimConfig {
            // Slow bullets so a point-blank shot still overlaps after move_bullets.
            bullet_speed: 1.0,
            lives: LivesConfig {
                starting_lives: 1,
                max_lives: 5,
                life_every_points: 20_000,
                invuln_secs: 1.0,
            },
            wave: WaveConfig {
                total_enemies: 0,
                max_on_field: 4,
                spawn_interval: 0.0,
            },
            ..Default::default()
        },
    );
    assert_eq!(world.progress.lives, 1);
    world.progress.invuln_remaining = 0.0;
    let (px, py) = {
        let p = world.player().unwrap();
        (p.x, p.y)
    };
    world.bullets.push(Bullet {
        id: 9_001,
        owner_id: 999,
        team: Team::Enemy,
        x: px,
        y: py,
        dir: Dir::Down,
        speed: 6.0,
    });
    world.step(&SimInput::default(), 1.0 / 60.0);
    assert_eq!(world.outcome, RoundOutcome::DefeatNoLives);
    assert_eq!(world.progress.lives, 0);
}

#[test]
fn gate_kill_awards_score_points() {
    let mut world = open_lane_world(
        1,
        LivesConfig {
            starting_lives: 3,
            max_lives: 5,
            life_every_points: 20_000,
            invuln_secs: 1.0,
        },
        ScoreConfig {
            enemy_basic: 100,
            streak_bonus_per_step: 0,
            clear_bonus: 0,
            clear_no_death_bonus: 0,
            ..Default::default()
        },
    );
    aim_player_at_spawn(&mut world);
    shoot_until_round_over(&mut world);
    assert!(world.progress.score >= 100);
    assert_eq!(world.outcome, RoundOutcome::Victory);
}

#[test]
fn gate_score_threshold_grants_life() {
    let mut world = open_lane_world(
        1,
        LivesConfig {
            starting_lives: 3,
            max_lives: 5,
            life_every_points: 100,
            invuln_secs: 1.0,
        },
        ScoreConfig {
            enemy_basic: 100,
            streak_bonus_per_step: 0,
            clear_bonus: 0,
            clear_no_death_bonus: 0,
            ..Default::default()
        },
    );
    aim_player_at_spawn(&mut world);
    shoot_until_round_over(&mut world);
    assert_eq!(world.progress.score, 100);
    assert_eq!(world.progress.lives, 4);
}

#[test]
fn gate_save_file_roundtrip_profiles_and_solo_board() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("project1990_phase2_gate_{nanos}.txt"));
    let _ = fs::remove_file(&path);

    let mut save = SaveData::default();
    save.create_profile("GateAda").unwrap();
    save.create_profile("GateBob").unwrap();
    save.select_profile(0);
    save.record_run_end(1200, 3, false);
    save.insert_solo_score(BoardEntry {
        name: "GateBob".into(),
        score: 2000,
        stage: 5,
    });
    save.save_path(&path).unwrap();

    let loaded = SaveData::load_path(&path).unwrap();
    assert_eq!(loaded.profiles.len(), 2);
    assert_eq!(loaded.profiles[0].name, "GateAda");
    assert_eq!(loaded.profiles[0].best_run_score, 1200);
    assert_eq!(loaded.profiles[0].continue_stage, 1);
    assert_eq!(loaded.solo_board.len(), 2);
    assert_eq!(loaded.solo_board[0].score, 2000);
    assert_eq!(loaded.solo_board[0].name, "GateBob");
    assert!(loaded.is_name_taken("gateada"));

    let _ = fs::remove_file(&path);
}

#[test]
fn gate_campaign_and_save_progress_align() {
    let campaign = Campaign::bundled().expect("manifest");
    assert_eq!(campaign.len(), LEVEL_COUNT as usize);

    let mut save = SaveData::default();
    save.create_profile("Runner").unwrap();
    // Mid-campaign victory unlocks continue, does not pollute board.
    save.record_run_end(400, 2, true);
    assert_eq!(save.active().unwrap().continue_stage, 3);
    assert!(save.solo_board.is_empty());

    let (map, wave) = campaign.load(save.active().unwrap().continue_stage).unwrap();
    assert!(!map.player_spawns().is_empty());
    assert!(wave.total_enemies >= 1);

    // GAME OVER records board and resets continue.
    save.record_run_end(900, 3, false);
    assert_eq!(save.active().unwrap().continue_stage, 1);
    assert_eq!(save.solo_board[0].score, 900);
}

#[test]
fn gate_data_score_cfg_and_temp_path_helpers() {
    let cfg = ScoreConfig::parse(include_str!("../../../data/score.cfg")).unwrap();
    assert_eq!(cfg.clear_bonus, 500);
    let path: PathBuf = project1990_sim::default_save_path();
    assert!(path.ends_with("save_v1.txt"));
}
