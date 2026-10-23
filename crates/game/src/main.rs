//! Bevy presentation layer for Project 1990 (phase 1).
//!
//! Simulation lives in `project1990-sim`; this binary only renders and feeds input.

use bevy::prelude::*;
use project1990_sim::{Dir, Map, SimInput, Tile, World, DEFAULT_TICK_HZ};

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
struct TankSprite;

#[derive(Component)]
struct BulletSprite {
    sim_id: u64,
}

#[derive(Component)]
struct AutofireLabel;

fn setup(mut commands: Commands) {
    let map = Map::parse(MAP_TEXT).expect("test_level.txt must parse");
    let map_w = map.width() as f32;
    let map_h = map.height() as f32;
    let world = World::new(map, Default::default());

    commands.spawn(Camera2d);
    commands.insert_resource(ViewMeta { map_w, map_h });
    commands.insert_resource(FrameInput::default());

    // Tiles
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

    // Player tank
    if let Some(tank) = world.player() {
        let (bx, by) = sim_to_bevy(tank.x, tank.y, map_w, map_h);
        commands.spawn((
            TankSprite,
            Sprite::from_color(Color::srgb(0.2, 0.85, 0.35), Vec2::splat(TILE_PX * 0.8)),
            Transform::from_xyz(bx, by, 1.0),
        ));
    }

    commands.spawn((
        AutofireLabel,
        Text2d::new("autofire: off  |  WASD move  Space fire  K toggle autofire"),
        TextFont {
            font_size: FontSize::Px(18.0),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.9, 0.85)),
        Transform::from_xyz(0.0, map_h * TILE_PX * 0.5 + 24.0, 2.0),
    ));

    commands.insert_resource(SimWorld(world));
}

fn read_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut frame: ResMut<FrameInput>,
) {
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
    mut tiles: Query<(&TileSprite, &mut Sprite)>,
    mut tank_q: Query<&mut Transform, (With<TankSprite>, Without<BulletSprite>)>,
    mut bullets: Query<(Entity, &BulletSprite, &mut Transform)>,
    mut label: Query<&mut Text2d, With<AutofireLabel>>,
    mut commands: Commands,
) {
    for (marker, mut sprite) in &mut tiles {
        if let Some(tile) = sim.0.map.get(marker.x, marker.y) {
            sprite.color = tile_color(tile);
        }
    }

    if let Some(tank) = sim.0.player() {
        if let Ok(mut tf) = tank_q.single_mut() {
            let (bx, by) = sim_to_bevy(tank.x, tank.y, meta.map_w, meta.map_h);
            tf.translation.x = bx;
            tf.translation.y = by;
            tf.rotation = Quat::from_rotation_z(dir_angle(tank.dir));
        }
        if let Ok(mut text) = label.single_mut() {
            let state = if tank.fire.autofire { "ON" } else { "off" };
            text.0 = format!(
                "autofire: {state}  |  WASD move  Space/J fire  K toggle autofire"
            );
        }
    }

    let live: Vec<(u64, f32, f32)> = sim
        .0
        .bullets
        .iter()
        .map(|b| {
            let (bx, by) = sim_to_bevy(b.x, b.y, meta.map_w, meta.map_h);
            (b.id, bx, by)
        })
        .collect();

    let mut seen = Vec::new();
    for (entity, marker, mut tf) in &mut bullets {
        if let Some((_, bx, by)) = live.iter().find(|(id, _, _)| *id == marker.sim_id) {
            tf.translation.x = *bx;
            tf.translation.y = *by;
            seen.push(marker.sim_id);
        } else {
            commands.entity(entity).despawn();
        }
    }

    for (id, bx, by) in live {
        if !seen.contains(&id) {
            commands.spawn((
                BulletSprite { sim_id: id },
                Sprite::from_color(Color::srgb(1.0, 0.95, 0.4), Vec2::splat(8.0)),
                Transform::from_xyz(bx, by, 1.5),
            ));
        }
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
