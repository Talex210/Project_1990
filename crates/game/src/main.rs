//! Bevy presentation layer for Project 1990.
//!
//! Simulation lives in `project1990-sim`; this binary only renders and feeds input.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use project1990_sim::{
    Dir, LivesConfig, Map, RoundOutcome, ScoreConfig, SimConfig, SimInput, Team, Tile, WaveConfig,
    World, DEFAULT_TICK_HZ,
};

const TILE_PX: f32 = 32.0;
/// Reserved space above the map so Playing HUD does not cover tiles.
const HUD_BAND_PX: f32 = 100.0;
const MAP_TEXT: &str = include_str!("../../../data/maps/test_level.txt");
const SCORE_CFG: &str = include_str!("../../../data/score.cfg");

fn main() {
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
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                menu_controls.run_if(in_state(Screen::Menu)),
                stage_controls.run_if(in_state(Screen::LevelIntro)),
                (playing_controls, step_sim, sync_play)
                    .chain()
                    .run_if(in_state(Screen::Playing)),
                update_overlay_text,
                set_playfield_visibility,
            ),
        )
        .run();
}

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
enum Screen {
    #[default]
    Menu,
    /// Classic-style “STAGE N” card before the fight.
    LevelIntro,
    Playing,
}

/// Which campaign stage is loaded (1…100 later).
#[derive(Resource, Clone, Copy)]
struct LevelSession {
    number: u32,
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
struct HudLabel;

#[derive(Resource)]
struct ShowControlsHelp(bool);

fn make_world() -> World {
    let map = Map::parse(MAP_TEXT).expect("test_level.txt must parse");
    let score = ScoreConfig::parse(SCORE_CFG).expect("data/score.cfg");
    World::new(
        map,
        SimConfig {
            wave: WaveConfig {
                total_enemies: 4,
                max_on_field: 4,
                spawn_interval: 2.0,
            },
            lives: LivesConfig {
                starting_lives: 3,
                max_lives: 5,
                life_every_points: 200,
                invuln_secs: 3.0,
            },
            score,
            ..Default::default()
        },
    )
}

fn setup(mut commands: Commands) {
    let world = make_world();
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
        Text2d::new(menu_text()),
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
    level: u32,
) {
    session.number = level;
    *sim = SimWorld(make_world());
    paused.0 = false;
}

fn menu_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    mut paused: ResMut<Paused>,
    mut sim: ResMut<SimWorld>,
    mut session: ResMut<LevelSession>,
) {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        begin_level(&mut sim, &mut session, &mut paused, 1);
        next.set(Screen::LevelIntro);
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
            begin_level(&mut sim, &mut session, &mut paused, n);
            next.set(Screen::LevelIntro);
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
    mut label: Query<&mut Text2d, With<HudLabel>>,
) {
    let Ok(mut text) = label.single_mut() else {
        return;
    };
    text.0 = match screen.get() {
        Screen::Menu => menu_text(),
        Screen::LevelIntro => stage_text(&sim.0, session.number),
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
    mut tiles: Query<(&TileSprite, &mut Sprite), (With<TileSprite>, Without<TankSprite>)>,
    mut tanks: Query<
        (Entity, &TankSprite, &mut Transform, &mut Sprite),
        (With<TankSprite>, Without<TileSprite>, Without<BulletSprite>),
    >,
    mut bullets: Query<
        (Entity, &BulletSprite, &mut Transform),
        (With<BulletSprite>, Without<TankSprite>, Without<TileSprite>),
    >,
    mut commands: Commands,
) {
    for (marker, mut sprite) in &mut tiles {
        if let Some(tile) = sim.0.map.get(marker.x, marker.y) {
            sprite.color = tile_color(tile);
        }
    }

    let invuln = sim.0.progress.is_invulnerable();
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
}

fn menu_text() -> String {
    "PROJECT 1990\n\nEnter / Space - play\n\nEsc/P pause | K autofire | H help\n(Classic title screen - later polish)"
        .to_string()
}

fn stage_text(world: &World, level: u32) -> String {
    let enemies = world.wave.config.total_enemies;
    let lives = world.progress.lives;
    let score = world.progress.score;
    let max_on = world.wave.config.max_on_field;
    format!(
        "STAGE {level:02}\n\n\
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
    let streak = world.style.streak;
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
                "STAGE {level:02}  SCORE {score}  lives {lives}  streak x{streak}  enemies {on}/{max} left {left}  AUTO {autofire}"
            );
            if !popup.is_empty() {
                line.push_str(&format!("\n{popup}"));
            }
            line
        }
        RoundOutcome::Victory => {
            let mut line =
                format!("VICTORY - STAGE {level:02}  SCORE {score}\nEnter/Space next try   Esc menu");
            if !popup.is_empty() {
                line.push_str(&format!("\n{popup}"));
            }
            line
        }
        RoundOutcome::DefeatBaseDestroyed => {
            format!(
                "DEFEAT - base destroyed  |  STAGE {level:02}  SCORE {score}\nEnter/Space retry   Esc menu"
            )
        }
        RoundOutcome::DefeatNoLives => {
            format!(
                "DEFEAT - no lives  |  STAGE {level:02}  SCORE {score}\nEnter/Space retry   Esc menu"
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
