//! Bevy presentation layer for Project 1990.
//!
//! Simulation lives in `project1990-sim`; this binary only renders and feeds input.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use project1990_sim::{
    default_save_path, BonusCatalog, Campaign, Dir, LEVEL_COUNT, LivesConfig, MAX_NAME_CHARS,
    MAX_PROFILES, PlayerProgress, ProfileError, RoundOutcome, SaveData, ScoreConfig, SimConfig,
    SimInput, Team, Tile, World, DEFAULT_TICK_HZ,
};
use std::path::PathBuf;

const TILE_PX: f32 = 32.0;
/// Reserved space above the map so Playing HUD does not cover tiles.
const HUD_BAND_PX: f32 = 120.0;
const SCORE_CFG: &str = include_str!("../../../data/score.cfg");
const BONUS_CATALOG: &str = include_str!("../../../data/bonuses/catalog.cfg");

fn main() {
    let campaign = Campaign::bundled().expect("data/levels/manifest.txt");
    let save_path = default_save_path();
    let save = SaveData::load_path(&save_path).unwrap_or_else(|e| {
        eprintln!("save load failed ({e}); using defaults");
        SaveData::default()
    });
    // Old schema-1 files are rejected — start clean named profiles.
    let cursor = ProfileCursor {
        index: save.active_profile.min(save.profiles.len().saturating_add(1).saturating_sub(1)),
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Project 1990".into(),
                // Tall enough for HUD band + map at default size.
                resolution: (1024, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .init_state::<Screen>()
        .insert_resource(ClearColor(Color::srgb(0.08, 0.08, 0.1)))
        .insert_resource(Paused(false))
        .insert_resource(ShowControlsHelp(true))
        .insert_resource(FrameInput::default())
        .insert_resource(LevelSession { number: 1 })
        .insert_resource(Levels(campaign))
        .insert_resource(SaveStore {
            data: save,
            path: save_path,
        })
        .insert_resource(cursor)
        .insert_resource(MenuCursor { index: 0 })
        .insert_resource(NameDraft::default())
        .insert_resource(HighScoresCtx {
            after_game_over: false,
        })
        .add_systems(Startup, setup)
        .add_systems(OnEnter(Screen::Menu), reset_menu_cursor)
        .add_systems(
            Update,
            (
                profile_select_controls.run_if(in_state(Screen::ProfileSelect)),
                profile_create_controls.run_if(in_state(Screen::ProfileCreate)),
                menu_controls.run_if(in_state(Screen::Menu)),
                high_scores_controls.run_if(in_state(Screen::HighScores)),
                stage_controls.run_if(in_state(Screen::LevelIntro)),
                (playing_controls, step_sim, sync_play)
                    .chain()
                    .run_if(in_state(Screen::Playing)),
                update_overlay_text,
                set_playfield_visibility,
                save_on_app_exit,
            ),
        )
        .run();
}

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
enum Screen {
    #[default]
    ProfileSelect,
    ProfileCreate,
    Menu,
    HighScores,
    /// Classic-style “STAGE N” card before the fight.
    LevelIntro,
    Playing,
}

/// Cursor on profile list; last index = “Create new”.
#[derive(Resource, Clone, Copy)]
struct ProfileCursor {
    index: usize,
}

/// Cursor on main menu rows (Continue / New game / Records / Change player).
#[derive(Resource, Clone, Copy)]
struct MenuCursor {
    index: usize,
}

const MENU_ITEMS: usize = 4;

#[derive(Resource, Default)]
struct NameDraft {
    text: String,
    error: Option<String>,
}

#[derive(Resource, Clone, Copy)]
struct HighScoresCtx {
    after_game_over: bool,
}

/// Which campaign stage is loaded (1…100).
#[derive(Resource, Clone, Copy)]
struct LevelSession {
    number: u32,
}

#[derive(Resource, Deref, DerefMut)]
struct Levels(Campaign);

#[derive(Resource)]
struct SaveStore {
    data: SaveData,
    path: PathBuf,
}

impl SaveStore {
    fn persist(&self) {
        if let Err(e) = self.data.save_path(&self.path) {
            eprintln!("save write failed: {e}");
        }
    }
}

#[derive(Resource)]
struct SimWorld(World);

#[derive(Resource, Default)]
struct FrameInput(SimInput);

#[derive(Resource)]
struct Paused(bool);

#[derive(Resource)]
struct ViewMeta {
    map_w: f32,
    map_h: f32,
}

#[derive(Component)]
struct Playfield;

#[derive(Component)]
struct TileSprite {
    x: u32,
    y: u32,
}

#[derive(Component)]
struct TankSprite {
    sim_id: u64,
}

#[derive(Component)]
struct BulletSprite {
    sim_id: u64,
}

#[derive(Component)]
struct BonusSprite {
    sim_id: u64,
}

/// Letter drawn on top of the bonus square (same `sim_id`).
#[derive(Component)]
struct BonusLetter {
    sim_id: u64,
}

#[derive(Component)]
struct HudLabel;

#[derive(Resource)]
struct ShowControlsHelp(bool);

fn make_world(levels: &Campaign, level: u32, carry: Option<PlayerProgress>) -> World {
    let (map, wave) = levels
        .load(level)
        .unwrap_or_else(|e| panic!("load stage {level}: {e}"));
    let score = ScoreConfig::parse(SCORE_CFG).expect("data/score.cfg");
    let bonuses = BonusCatalog::parse(BONUS_CATALOG).expect("data/bonuses/catalog.cfg");
    let mut world = World::new(
        map,
        SimConfig {
            wave,
            lives: LivesConfig {
                starting_lives: 3,
                max_lives: 5,
                life_every_points: 200,
                invuln_secs: 3.0,
            },
            score,
            bonuses,
            ..Default::default()
        },
    );
    if let Some(mut progress) = carry {
        progress.invuln_remaining = 0.0;
        world.progress = progress;
        // Stars + ship persist across stages until death / bullet hit.
        world.reapply_player_from_progress();
    }
    world
}

fn setup(
    mut commands: Commands,
    levels: Res<Levels>,
    save: Res<SaveStore>,
    mut next: ResMut<NextState<Screen>>,
) {
    if !save.data.has_profiles() {
        next.set(Screen::ProfileCreate);
    }
    let world = make_world(&levels, 1, None);
    let map_w = world.map.width() as f32;
    let map_h = world.map.height() as f32;

    commands.spawn(Camera2d);
    commands.insert_resource(ViewMeta { map_w, map_h });

    for y in 0..world.map.height() {
        for x in 0..world.map.width() {
            let tile = world.map.get(x, y).unwrap();
            let (bx, by) = sim_to_bevy(x as f32 + 0.5, y as f32 + 0.5, map_w, map_h);
            commands.spawn((
                Playfield,
                TileSprite { x, y },
                Sprite::from_color(tile_color(tile), Vec2::splat(TILE_PX - 1.0)),
                Transform::from_xyz(bx, by, 0.0),
                Visibility::Hidden,
            ));
        }
    }

    for tank in &world.tanks {
        spawn_tank_sprite(&mut commands, tank.id, tank.team, tank.x, tank.y, map_w, map_h);
    }

    // HUD sits in the band above the (down-shifted) playfield; TopCenter grows down into the band.
    let map_top = map_h * TILE_PX * 0.5 - HUD_BAND_PX * 0.5;
    commands.spawn((
        HudLabel,
        Text2d::new("PROJECT 1990"),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.95, 0.95, 0.88)),
        TextLayout::justify(Justify::Center),
        Anchor::TOP_CENTER,
        Transform::from_xyz(0.0, map_top + HUD_BAND_PX - 4.0, 2.0),
    ));

    commands.insert_resource(SimWorld(world));
}

fn spawn_tank_sprite(
    commands: &mut Commands,
    sim_id: u64,
    team: Team,
    x: f32,
    y: f32,
    map_w: f32,
    map_h: f32,
) {
    let (bx, by) = sim_to_bevy(x, y, map_w, map_h);
    commands.spawn((
        Playfield,
        TankSprite { sim_id },
        Sprite::from_color(team_color(team), Vec2::splat(TILE_PX * 0.8)),
        Transform::from_xyz(bx, by, 1.0),
        Visibility::Hidden,
    ));
}

fn begin_level(
    sim: &mut SimWorld,
    session: &mut LevelSession,
    paused: &mut Paused,
    levels: &Campaign,
    level: u32,
    carry: Option<PlayerProgress>,
) {
    session.number = level.clamp(1, LEVEL_COUNT);
    *sim = SimWorld(make_world(levels, session.number, carry));
    paused.0 = false;
}

fn profile_list_len(save: &SaveData) -> usize {
    // profiles + mandatory "Create new" row
    save.profiles.len() + 1
}

fn profile_select_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    mut save: ResMut<SaveStore>,
    mut cursor: ResMut<ProfileCursor>,
    mut draft: ResMut<NameDraft>,
) {
    let len = profile_list_len(&save.data);
    if len == 0 {
        return;
    }
    cursor.index %= len;

    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) {
        cursor.index = if cursor.index == 0 { len - 1 } else { cursor.index - 1 };
        return;
    }
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) {
        cursor.index = (cursor.index + 1) % len;
        return;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        if cursor.index >= save.data.profiles.len() {
            *draft = NameDraft::default();
            next.set(Screen::ProfileCreate);
        } else {
            save.data.select_profile(cursor.index);
            save.persist();
            next.set(Screen::Menu);
        }
    }
}

fn profile_create_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    mut save: ResMut<SaveStore>,
    mut draft: ResMut<NameDraft>,
    mut cursor: ResMut<ProfileCursor>,
) {
    if keys.just_pressed(KeyCode::Escape) && save.data.has_profiles() {
        next.set(Screen::ProfileSelect);
        return;
    }

    if keys.just_pressed(KeyCode::Backspace) {
        draft.text.pop();
        draft.error = None;
        return;
    }

    type_into_name(&keys, &mut draft.text);

    if keys.just_pressed(KeyCode::Enter) {
        match save.data.create_profile(&draft.text) {
            Ok(idx) => {
                save.persist();
                cursor.index = idx;
                *draft = NameDraft::default();
                next.set(Screen::Menu);
            }
            Err(e) => {
                draft.error = Some(match e {
                    ProfileError::EmptyName => "Enter a name".into(),
                    ProfileError::NameTaken => "Name already taken".into(),
                    ProfileError::NameTooLong => format!("Max {MAX_NAME_CHARS} characters"),
                    ProfileError::ProfileLimit => format!("Max {MAX_PROFILES} profiles"),
                    ProfileError::NoProfiles => "No profiles".into(),
                });
            }
        }
    }
}

fn type_into_name(keys: &ButtonInput<KeyCode>, text: &mut String) {
    if text.chars().count() >= MAX_NAME_CHARS {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let letters = [
        (KeyCode::KeyA, 'a'),
        (KeyCode::KeyB, 'b'),
        (KeyCode::KeyC, 'c'),
        (KeyCode::KeyD, 'd'),
        (KeyCode::KeyE, 'e'),
        (KeyCode::KeyF, 'f'),
        (KeyCode::KeyG, 'g'),
        (KeyCode::KeyH, 'h'),
        (KeyCode::KeyI, 'i'),
        (KeyCode::KeyJ, 'j'),
        (KeyCode::KeyK, 'k'),
        (KeyCode::KeyL, 'l'),
        (KeyCode::KeyM, 'm'),
        (KeyCode::KeyN, 'n'),
        (KeyCode::KeyO, 'o'),
        (KeyCode::KeyP, 'p'),
        (KeyCode::KeyQ, 'q'),
        (KeyCode::KeyR, 'r'),
        (KeyCode::KeyS, 's'),
        (KeyCode::KeyT, 't'),
        (KeyCode::KeyU, 'u'),
        (KeyCode::KeyV, 'v'),
        (KeyCode::KeyW, 'w'),
        (KeyCode::KeyX, 'x'),
        (KeyCode::KeyY, 'y'),
        (KeyCode::KeyZ, 'z'),
    ];
    for (code, ch) in letters {
        if keys.just_pressed(code) {
            text.push(if shift {
                ch.to_ascii_uppercase()
            } else {
                ch
            });
            return;
        }
    }
    let digits = [
        (KeyCode::Digit0, '0'),
        (KeyCode::Digit1, '1'),
        (KeyCode::Digit2, '2'),
        (KeyCode::Digit3, '3'),
        (KeyCode::Digit4, '4'),
        (KeyCode::Digit5, '5'),
        (KeyCode::Digit6, '6'),
        (KeyCode::Digit7, '7'),
        (KeyCode::Digit8, '8'),
        (KeyCode::Digit9, '9'),
    ];
    for (code, ch) in digits {
        if keys.just_pressed(code) {
            text.push(ch);
            return;
        }
    }
    if keys.just_pressed(KeyCode::Space) && !text.is_empty() {
        text.push(' ');
    }
}

fn reset_menu_cursor(mut cursor: ResMut<MenuCursor>) {
    cursor.index = 0;
}

fn menu_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    mut paused: ResMut<Paused>,
    mut sim: ResMut<SimWorld>,
    mut session: ResMut<LevelSession>,
    levels: Res<Levels>,
    save: Res<SaveStore>,
    mut scores_ctx: ResMut<HighScoresCtx>,
    mut cursor: ResMut<MenuCursor>,
) {
    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) {
        cursor.index = if cursor.index == 0 {
            MENU_ITEMS - 1
        } else {
            cursor.index - 1
        };
        return;
    }
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) {
        cursor.index = (cursor.index + 1) % MENU_ITEMS;
        return;
    }
    if !(keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space)) {
        return;
    }
    match cursor.index {
        0 => {
            // Continue
            let stage = save
                .data
                .active()
                .map(|p| p.continue_stage)
                .unwrap_or(1);
            begin_level(&mut sim, &mut session, &mut paused, &levels, stage, None);
            next.set(Screen::LevelIntro);
        }
        1 => {
            // New game
            begin_level(&mut sim, &mut session, &mut paused, &levels, 1, None);
            next.set(Screen::LevelIntro);
        }
        2 => {
            scores_ctx.after_game_over = false;
            next.set(Screen::HighScores);
        }
        3 => next.set(Screen::ProfileSelect),
        _ => {}
    }
}

fn save_on_app_exit(mut exits: MessageReader<AppExit>, save: Res<SaveStore>) {
    if exits.read().next().is_some() {
        save.persist();
    }
}

fn high_scores_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    mut paused: ResMut<Paused>,
    mut sim: ResMut<SimWorld>,
    mut session: ResMut<LevelSession>,
    levels: Res<Levels>,
    scores_ctx: Res<HighScoresCtx>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next.set(if scores_ctx.after_game_over {
            Screen::Menu
        } else {
            Screen::Menu
        });
        return;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        if scores_ctx.after_game_over {
            begin_level(&mut sim, &mut session, &mut paused, &levels, 1, None);
            next.set(Screen::LevelIntro);
        } else {
            next.set(Screen::Menu);
        }
    }
}

fn stage_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    mut paused: ResMut<Paused>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        paused.0 = false;
        next.set(Screen::Menu);
        return;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        paused.0 = false;
        next.set(Screen::Playing);
    }
}

fn playing_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut frame: ResMut<FrameInput>,
    mut help: ResMut<ShowControlsHelp>,
    mut paused: ResMut<Paused>,
    mut next: ResMut<NextState<Screen>>,
    mut sim: ResMut<SimWorld>,
    mut session: ResMut<LevelSession>,
    levels: Res<Levels>,
    mut save: ResMut<SaveStore>,
    mut scores_ctx: ResMut<HighScoresCtx>,
) {
    let round_over = sim.0.outcome != RoundOutcome::Playing;

    if keys.just_pressed(KeyCode::KeyH) {
        help.0 = !help.0;
    }

    // Esc / P: pause while playing; from pause or round-over → main menu.
    if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyP) {
        if round_over {
            next.set(Screen::Menu);
            paused.0 = false;
            return;
        }
        if paused.0 {
            // Esc from pause → menu; P only resumes.
            if keys.just_pressed(KeyCode::Escape) {
                next.set(Screen::Menu);
                paused.0 = false;
            } else {
                paused.0 = false;
            }
        } else {
            paused.0 = true;
        }
        return;
    }

    if round_over {
        if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
            let n = session.number;
            let score = sim.0.progress.score;
            match sim.0.outcome {
                RoundOutcome::Victory => {
                    save.data.record_run_end(score, n, true);
                    save.persist();
                    let next_stage = (n + 1).min(LEVEL_COUNT);
                    let carry = Some(sim.0.progress.clone());
                    begin_level(
                        &mut sim,
                        &mut session,
                        &mut paused,
                        &levels,
                        next_stage,
                        carry,
                    );
                    next.set(Screen::LevelIntro);
                }
                RoundOutcome::DefeatBaseDestroyed | RoundOutcome::DefeatNoLives => {
                    save.data.record_run_end(score, n, false);
                    save.persist();
                    scores_ctx.after_game_over = true;
                    next.set(Screen::HighScores);
                }
                RoundOutcome::Playing => {}
            }
        }
        frame.0 = SimInput::default();
        return;
    }

    if paused.0 {
        frame.0 = SimInput::default();
        return;
    }

    let move_dir = if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        Some(Dir::Up)
    } else if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        Some(Dir::Down)
    } else if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        Some(Dir::Left)
    } else if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        Some(Dir::Right)
    } else {
        None
    };

    frame.0 = SimInput {
        move_dir,
        fire_held: keys.pressed(KeyCode::Space) || keys.pressed(KeyCode::KeyJ),
        toggle_autofire: keys.just_pressed(KeyCode::KeyK)
            || keys.just_pressed(KeyCode::CapsLock),
    };
}

fn step_sim(mut sim: ResMut<SimWorld>, frame: Res<FrameInput>, paused: Res<Paused>) {
    if paused.0 {
        return;
    }
    let dt = 1.0 / DEFAULT_TICK_HZ as f32;
    sim.0.step(&frame.0, dt);
}

fn update_overlay_text(
    screen: Res<State<Screen>>,
    sim: Res<SimWorld>,
    session: Res<LevelSession>,
    paused: Res<Paused>,
    help: Res<ShowControlsHelp>,
    save: Res<SaveStore>,
    profile_cursor: Res<ProfileCursor>,
    menu_cursor: Res<MenuCursor>,
    draft: Res<NameDraft>,
    scores_ctx: Res<HighScoresCtx>,
    mut label: Query<&mut Text2d, (With<HudLabel>, Without<BonusLetter>)>,
) {
    let Ok(mut text) = label.single_mut() else {
        return;
    };
    text.0 = match screen.get() {
        Screen::ProfileSelect => profile_select_text(&save.data, profile_cursor.index),
        Screen::ProfileCreate => profile_create_text(&draft),
        Screen::Menu => menu_text(&save.data, menu_cursor.index),
        Screen::HighScores => high_scores_text(&save.data, scores_ctx.after_game_over),
        Screen::LevelIntro => stage_text(
            &sim.0,
            session.number,
            save.data.active().map(|p| p.name.as_str()),
        ),
        Screen::Playing => hud_text(&sim.0, help.0, paused.0, session.number),
    };
}

fn set_playfield_visibility(
    screen: Res<State<Screen>>,
    mut q: Query<&mut Visibility, With<Playfield>>,
) {
    let visible = matches!(screen.get(), Screen::Playing);
    let vis = if visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut v in &mut q {
        *v = vis;
    }
}

fn sync_play(
    sim: Res<SimWorld>,
    meta: Res<ViewMeta>,
    mut tiles: Query<
        (&TileSprite, &mut Sprite),
        (
            With<TileSprite>,
            Without<TankSprite>,
            Without<BonusSprite>,
            Without<BonusLetter>,
        ),
    >,
    mut tanks: Query<
        (Entity, &TankSprite, &mut Transform, &mut Sprite),
        (
            With<TankSprite>,
            Without<TileSprite>,
            Without<BulletSprite>,
            Without<BonusSprite>,
            Without<BonusLetter>,
        ),
    >,
    mut bullets: Query<
        (Entity, &BulletSprite, &mut Transform),
        (
            With<BulletSprite>,
            Without<TankSprite>,
            Without<TileSprite>,
            Without<BonusSprite>,
            Without<BonusLetter>,
        ),
    >,
    mut bonuses: Query<
        (Entity, &BonusSprite, &mut Transform, &mut Sprite),
        (
            With<BonusSprite>,
            Without<TankSprite>,
            Without<TileSprite>,
            Without<BulletSprite>,
            Without<BonusLetter>,
        ),
    >,
    mut bonus_letters: Query<
        (Entity, &BonusLetter, &mut Transform, &mut Text2d),
        (
            With<BonusLetter>,
            Without<BonusSprite>,
            Without<TankSprite>,
            Without<BulletSprite>,
            Without<TileSprite>,
            Without<HudLabel>,
        ),
    >,
    mut commands: Commands,
) {
    for (marker, mut sprite) in &mut tiles {
        if let Some(tile) = sim.0.map.get(marker.x, marker.y) {
            sprite.color = tile_color(tile);
        }
    }

    let invuln = sim.0.progress.is_invulnerable()
        || sim.0.player().map(|t| t.mods.invuln).unwrap_or(false);
    let blink_on = !invuln || ((sim.0.tick / 8) % 2 == 0);

    let live_tanks: Vec<(u64, Team, f32, f32, Dir)> = sim
        .0
        .tanks
        .iter()
        .map(|t| (t.id, t.team, t.x, t.y, t.dir))
        .collect();

    let mut seen_tanks = Vec::new();
    for (entity, marker, mut tf, mut sprite) in &mut tanks {
        if let Some((_, team, x, y, dir)) =
            live_tanks.iter().find(|(id, _, _, _, _)| *id == marker.sim_id)
        {
            let (bx, by) = sim_to_bevy(*x, *y, meta.map_w, meta.map_h);
            tf.translation.x = bx;
            tf.translation.y = by;
            tf.rotation = Quat::from_rotation_z(dir_angle(*dir));
            let mut color = team_color(*team);
            if *team == Team::Player && invuln {
                color = if blink_on {
                    Color::srgb(0.85, 1.0, 0.9)
                } else {
                    Color::srgba(0.2, 0.85, 0.35, 0.25)
                };
            }
            sprite.color = color;
            seen_tanks.push(marker.sim_id);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (id, team, x, y, _) in &live_tanks {
        if !seen_tanks.contains(id) {
            spawn_tank_sprite(
                &mut commands,
                *id,
                *team,
                *x,
                *y,
                meta.map_w,
                meta.map_h,
            );
        }
    }

    let live_bullets: Vec<(u64, f32, f32)> = sim
        .0
        .bullets
        .iter()
        .map(|b| {
            let (bx, by) = sim_to_bevy(b.x, b.y, meta.map_w, meta.map_h);
            (b.id, bx, by)
        })
        .collect();

    let mut seen_bullets = Vec::new();
    for (entity, marker, mut tf) in &mut bullets {
        if let Some((_, bx, by)) = live_bullets.iter().find(|(id, _, _)| *id == marker.sim_id) {
            tf.translation.x = *bx;
            tf.translation.y = *by;
            seen_bullets.push(marker.sim_id);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (id, bx, by) in live_bullets {
        if !seen_bullets.contains(&id) {
            commands.spawn((
                Playfield,
                BulletSprite { sim_id: id },
                Sprite::from_color(Color::srgb(1.0, 0.95, 0.4), Vec2::splat(8.0)),
                Transform::from_xyz(bx, by, 1.5),
                Visibility::Visible,
            ));
        }
    }

    let live_bonuses: Vec<(u64, char, f32, f32)> = sim
        .0
        .bonuses
        .pickups
        .iter()
        .map(|p| {
            let (bx, by) = sim_to_bevy(p.x, p.y, meta.map_w, meta.map_h);
            (p.id, p.letter, bx, by)
        })
        .collect();
    let blink_bonus = (sim.0.tick / 12) % 2 == 0;

    let mut seen_bonuses = Vec::new();
    for (entity, marker, mut tf, mut sprite) in &mut bonuses {
        if let Some((_, letter, bx, by)) =
            live_bonuses.iter().find(|(id, _, _, _)| *id == marker.sim_id)
        {
            tf.translation.x = *bx;
            tf.translation.y = *by;
            sprite.color = if blink_bonus {
                bonus_color(*letter)
            } else {
                Color::srgb(1.0, 1.0, 0.55)
            };
            seen_bonuses.push(marker.sim_id);
        } else {
            commands.entity(entity).despawn();
        }
    }
    let mut seen_letters = Vec::new();
    for (entity, marker, mut tf, mut text) in &mut bonus_letters {
        if let Some((_, letter, bx, by)) =
            live_bonuses.iter().find(|(id, _, _, _)| *id == marker.sim_id)
        {
            tf.translation.x = *bx;
            tf.translation.y = *by;
            text.0 = letter.to_string();
            seen_letters.push(marker.sim_id);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (id, letter, bx, by) in live_bonuses {
        if !seen_bonuses.contains(&id) {
            commands.spawn((
                Playfield,
                BonusSprite { sim_id: id },
                Sprite::from_color(bonus_color(letter), Vec2::splat(TILE_PX * 0.78)),
                Transform::from_xyz(bx, by, 1.7),
                Visibility::Visible,
            ));
        }
        if !seen_letters.contains(&id) {
            commands.spawn((
                Playfield,
                BonusLetter { sim_id: id },
                Text2d::new(letter.to_string()),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                },
                TextColor(Color::srgb(0.08, 0.08, 0.1)),
                TextLayout::justify(Justify::Center),
                Transform::from_xyz(bx, by, 1.85),
                Visibility::Visible,
            ));
        }
    }
}

fn profile_select_text(save: &SaveData, cursor: usize) -> String {
    let mut lines = String::from("SELECT PLAYER\n\n");
    if save.profiles.is_empty() {
        lines.push_str("(no profiles yet)\n");
    }
    for (i, p) in save.profiles.iter().enumerate() {
        let mark = if i == cursor { ">" } else { " " };
        lines.push_str(&format!(
            "{mark} {}  best {}  cont {:02}\n",
            p.name, p.best_run_score, p.continue_stage
        ));
    }
    let create_idx = save.profiles.len();
    let mark = if cursor == create_idx { ">" } else { " " };
    let create_label = if save.profiles.len() >= MAX_PROFILES {
        "Create new (full)"
    } else {
        "Create new profile"
    };
    lines.push_str(&format!("\n{mark} {create_label}\n"));
    lines.push_str("\nUp/Down select   Enter confirm");
    lines
}

fn profile_create_text(draft: &NameDraft) -> String {
    let mut lines = format!(
        "NEW PROFILE\n\nName: {}_\n(max {MAX_NAME_CHARS} chars, unique)\n",
        draft.text
    );
    if let Some(err) = &draft.error {
        lines.push_str(&format!("\n! {err}\n"));
    }
    lines.push_str("\nType name   Enter create");
    if draft.text.is_empty() {
        // Esc only if profiles already exist — still show hint softly
        lines.push_str("\nEsc cancel (if profiles exist)");
    } else {
        lines.push_str("\nBackspace delete   Esc cancel");
    }
    lines
}

fn menu_text(save: &SaveData, cursor: usize) -> String {
    let Some(p) = save.active() else {
        return "PROJECT 1990\n\nNo profile — use Change player".into();
    };
    let rows = [
        format!("Continue  (STAGE {:02})", p.continue_stage),
        "New game  (STAGE 01)".into(),
        "Solo records".into(),
        "Change player".into(),
    ];
    let mut lines = format!(
        "PROJECT 1990\n\n\
Player: {}   Personal best {}\n\
Cleared {:02}   Reached {:02}\n\n",
        p.name, p.best_run_score, p.highest_stage_cleared, p.highest_stage_reached
    );
    for (i, row) in rows.iter().enumerate() {
        let mark = if i == cursor { ">" } else { " " };
        lines.push_str(&format!("{mark} {row}\n"));
    }
    lines.push_str("\nUp/Down move   Enter select");
    lines
}

fn high_scores_text(save: &SaveData, after_game_over: bool) -> String {
    let mut lines = if after_game_over {
        String::from("GAME OVER\n\nSOLO RECORDS\n")
    } else {
        String::from("SOLO RECORDS\n")
    };
    lines.push_str("(Coop table — later)\n\n");
    if save.solo_board.is_empty() {
        lines.push_str("  (empty)\n");
    } else {
        for (i, e) in save.solo_board.iter().enumerate() {
            lines.push_str(&format!(
                "  {:2}. {:12}  {:>6}  STAGE {:02}\n",
                i + 1,
                e.name,
                e.score,
                e.stage
            ));
        }
    }
    if after_game_over {
        lines.push_str("\nEnter/Space restart STAGE 01   Esc menu");
    } else {
        lines.push_str("\nEnter/Esc back to menu");
    }
    lines
}

fn stage_text(world: &World, level: u32, player: Option<&str>) -> String {
    let enemies = world.wave.config.total_enemies;
    let lives = world.progress.lives;
    let score = world.progress.score;
    let max_on = world.wave.config.max_on_field;
    let who = player.unwrap_or("Player");
    format!(
        "STAGE {level:02}\n{who}\n\n\
Lives {lives}    Score {score}\n\
Enemies in wave {enemies}    On field max {max_on}\n\
Goal: destroy all enemies - protect the base\n\n\
Enter / Space - fight\n\
Esc - menu"
    )
}

fn hud_text(world: &World, show_help: bool, paused: bool, level: u32) -> String {
    let on = world.enemies_on_field();
    let max = world.wave.config.max_on_field;
    let left = world.wave.remaining_in_wave(on);
    let lives = world.progress.lives;
    let score = world.progress.score;
    let _streak = world.style.streak;
    let autofire = world
        .player()
        .map(|p| if p.fire.autofire { "ON" } else { "OFF" })
        .unwrap_or("OFF");
    let popup = world
        .style
        .last_popup
        .as_ref()
        .map(|p| p.label())
        .unwrap_or_default();
    let upgrade = world.player().map(|p| p.mods.upgrade).unwrap_or(0);
    let mut active = world
        .bonuses
        .hud_active_labels(Team::Player, world.player().map(|p| p.id));
    active.extend(project1990_sim::BonusState::hud_power_labels(
        upgrade,
        world.progress.ship,
    ));
    let field = world.bonuses.hud_field_labels();

    if paused {
        let mut t = format!(
            "PAUSED  |  STAGE {level:02}\nSCORE {score}   lives {lives}   AUTOFIRE {autofire}\nP resume   Esc menu"
        );
        if show_help {
            t.push_str("\nWASD move  Space/J fire  K autofire  H help");
        }
        return t;
    }

    let status = match world.outcome {
        RoundOutcome::Playing => {
            let mut line = format!(
                "STAGE {level:02}  SCORE {score}  lives {lives}  tank★{upgrade}  enemies {on}/{max} left {left}  AUTO {autofire}"
            );
            if !active.is_empty() {
                line.push_str(&format!("\nACTIVE  {}", active.join("  |  ")));
            } else {
                line.push_str("\nACTIVE  (none)");
            }
            if !field.is_empty() {
                line.push_str(&format!("\nON MAP  {}", field.join("  |  ")));
            }
            if !popup.is_empty() {
                line.push_str(&format!("\n{popup}"));
            }
            line
        }
        RoundOutcome::Victory => {
            let mut line =
                format!(
                    "VICTORY - STAGE {level:02}  SCORE {score}\nEnter/Space next stage   Esc menu"
                );
            if !popup.is_empty() {
                line.push_str(&format!("\n{popup}"));
            }
            line
        }
        RoundOutcome::DefeatBaseDestroyed => {
            format!(
                "GAME OVER\nBase destroyed  |  STAGE {level:02}  SCORE {score}\nEnter/Space records   Esc menu"
            )
        }
        RoundOutcome::DefeatNoLives => {
            format!(
                "GAME OVER\nNo lives left  |  STAGE {level:02}  SCORE {score}\nEnter/Space records   Esc menu"
            )
        }
    };

    if world.outcome != RoundOutcome::Playing {
        return status;
    }
    if show_help {
        format!("{status}\nWASD move  Space/J fire  K autofire  Esc/P pause  H hide help")
    } else {
        format!("{status}\nH help")
    }
}

fn sim_to_bevy(sim_x: f32, sim_y: f32, map_w: f32, map_h: f32) -> (f32, f32) {
    let bx = (sim_x - map_w * 0.5) * TILE_PX;
    // Shift playfield down so the HUD band above the map stays clear.
    let by = (map_h * 0.5 - sim_y) * TILE_PX - HUD_BAND_PX * 0.5;
    (bx, by)
}

fn dir_angle(dir: Dir) -> f32 {
    match dir {
        Dir::Up => 0.0,
        Dir::Right => -std::f32::consts::FRAC_PI_2,
        Dir::Down => std::f32::consts::PI,
        Dir::Left => std::f32::consts::FRAC_PI_2,
    }
}

fn team_color(team: Team) -> Color {
    match team {
        Team::Player => Color::srgb(0.2, 0.85, 0.35),
        Team::Enemy => Color::srgb(0.9, 0.25, 0.2),
    }
}

fn tile_color(tile: Tile) -> Color {
    match tile {
        Tile::Empty => Color::srgb(0.12, 0.12, 0.14),
        Tile::Brick => Color::srgb(0.72, 0.35, 0.18),
        Tile::Steel => Color::srgb(0.65, 0.68, 0.72),
        Tile::Water => Color::srgb(0.15, 0.35, 0.75),
        Tile::Forest => Color::srgb(0.12, 0.45, 0.18),
        Tile::Ice => Color::srgb(0.7, 0.85, 0.95),
        Tile::Base => Color::srgb(0.9, 0.75, 0.2),
    }
}

fn bonus_color(letter: char) -> Color {
    match letter {
        'H' => Color::srgb(0.95, 0.85, 0.2),
        'C' => Color::srgb(0.4, 0.85, 1.0),
        'L' => Color::srgb(0.7, 0.55, 0.3),
        'G' => Color::srgb(0.9, 0.25, 0.2),
        '*' => Color::srgb(1.0, 0.9, 0.35),
        'P' => Color::srgb(0.95, 0.55, 0.15),
        'T' => Color::srgb(0.35, 0.9, 0.45),
        'B' => Color::srgb(0.25, 0.55, 0.95),
        'F' => Color::srgb(0.2, 0.75, 0.35),
        _ => Color::srgb(0.95, 0.95, 0.4),
    }
}
