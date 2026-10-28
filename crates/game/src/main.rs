//! Bevy presentation layer for Project 1990.
//!
//! Simulation lives in `project1990-sim`; this binary only renders and feeds input.

use bevy::prelude::*;
use project1990_sim::{
    Dir, LivesConfig, Map, RoundOutcome, SimConfig, SimInput, Team, Tile, WaveConfig, World,
    DEFAULT_TICK_HZ,
};

const TILE_PX: f32 = 32.0;
const MAP_TEXT: &str = include_str!("../../../data/maps/test_level.txt");

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Project 1990".into(),
                resolution: (960, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.08, 0.08, 0.1)))
        .add_systems(Startup, setup)
        .add_systems(Update, (read_input, step_sim, sync_view).chain())
        .run();
}

#[derive(Resource)]
struct SimWorld(World);

#[derive(Resource, Default)]
struct FrameInput(SimInput);

#[derive(Resource)]
struct ViewMeta {
    map_w: f32,
    map_h: f32,
}

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

/// When true, show the controls cheat-sheet under the status line.
#[derive(Resource)]
struct ShowControlsHelp(bool);

fn make_world() -> World {
    let map = Map::parse(MAP_TEXT).expect("test_level.txt must parse");
    World::new(
        map,
        SimConfig {
            wave: WaveConfig {
                total_enemies: 4,
                max_on_field: 4,
                spawn_interval: 2.0,
            },
            // Lower threshold in playtest so +1 life is easy to see (prod default 20_000).
            lives: LivesConfig {
                starting_lives: 3,
                max_lives: 5,
                life_every_points: 200,
                invuln_secs: 3.0,
                points_per_enemy: 100,
            },
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
    commands.insert_resource(FrameInput::default());
    commands.insert_resource(ShowControlsHelp(true));

    for y in 0..world.map.height() {
        for x in 0..world.map.width() {
            let tile = world.map.get(x, y).unwrap();
            let (bx, by) = sim_to_bevy(x as f32 + 0.5, y as f32 + 0.5, map_w, map_h);
            commands.spawn((
                TileSprite { x, y },
                Sprite::from_color(tile_color(tile), Vec2::splat(TILE_PX - 1.0)),
                Transform::from_xyz(bx, by, 0.0),
            ));
        }
    }

    for tank in &world.tanks {
        spawn_tank_sprite(&mut commands, tank.id, tank.team, tank.x, tank.y, map_w, map_h);
    }

    commands.spawn((
        HudLabel,
        Text2d::new(hud_text(&world, true)),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.9, 0.85)),
        Transform::from_xyz(0.0, map_h * TILE_PX * 0.5 + 36.0, 2.0),
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
        TankSprite { sim_id },
        Sprite::from_color(team_color(team), Vec2::splat(TILE_PX * 0.8)),
        Transform::from_xyz(bx, by, 1.0),
    ));
}

fn read_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut frame: ResMut<FrameInput>,
    mut help: ResMut<ShowControlsHelp>,
    mut sim: ResMut<SimWorld>,
) {
    if keys.just_pressed(KeyCode::KeyH) {
        help.0 = !help.0;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        *sim = SimWorld(make_world());
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

fn step_sim(mut sim: ResMut<SimWorld>, frame: Res<FrameInput>) {
    let dt = 1.0 / DEFAULT_TICK_HZ as f32;
    sim.0.step(&frame.0, dt);
}

fn sync_view(
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
    mut label: Query<&mut Text2d, With<HudLabel>>,
    help: Res<ShowControlsHelp>,
    mut commands: Commands,
) {
    for (marker, mut sprite) in &mut tiles {
        if let Some(tile) = sim.0.map.get(marker.x, marker.y) {
            sprite.color = tile_color(tile);
        }
    }

    if let Ok(mut text) = label.single_mut() {
        text.0 = hud_text(&sim.0, help.0);
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
                BulletSprite { sim_id: id },
                Sprite::from_color(Color::srgb(1.0, 0.95, 0.4), Vec2::splat(8.0)),
                Transform::from_xyz(bx, by, 1.5),
            ));
        }
    }
}

fn hud_text(world: &World, show_help: bool) -> String {
    let on = world.enemies_on_field();
    let max = world.wave.config.max_on_field;
    let left = world.wave.remaining_in_wave(on);
    let lives = world.progress.lives;
    let score = world.progress.score;
    let auto = world
        .player()
        .map(|p| if p.fire.autofire { "ON" } else { "off" })
        .unwrap_or("off");
    let status = match world.outcome {
        RoundOutcome::Playing => format!(
            "lives {lives}  |  score {score}  |  enemies {on}/{max}  |  left {left}  |  autofire {auto}"
        ),
        RoundOutcome::Victory => {
            format!("VICTORY — score {score}  |  R restart")
        }
        RoundOutcome::DefeatBaseDestroyed => {
            format!("DEFEAT — base destroyed  |  score {score}  |  R restart")
        }
        RoundOutcome::DefeatNoLives => {
            format!("DEFEAT — no lives  |  score {score}  |  R restart")
        }
    };
    if show_help {
        format!(
            "{status}\nWASD move  Space/J fire  K autofire  H hide help  R restart"
        )
    } else {
        format!("{status}   |   H show help")
    }
}

fn sim_to_bevy(sim_x: f32, sim_y: f32, map_w: f32, map_h: f32) -> (f32, f32) {
    let bx = (sim_x - map_w * 0.5) * TILE_PX;
    let by = (map_h * 0.5 - sim_y) * TILE_PX;
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
