//! Fixed-tick world: tanks, bullets, tile collisions.

use crate::fire::{FireConfig, FireController};
use crate::map::Map;
use crate::tile::Tile;

/// Default simulation rate (Hz). Presentation interpolates; network later locks to this.
pub const DEFAULT_TICK_HZ: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub fn delta(self) -> (f32, f32) {
        match self {
            Dir::Up => (0.0, -1.0),
            Dir::Down => (0.0, 1.0),
            Dir::Left => (-1.0, 0.0),
            Dir::Right => (1.0, 0.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Team {
    Player,
    Enemy,
}

#[derive(Debug, Clone)]
pub struct Tank {
    pub id: u64,
    pub team: Team,
    /// Center position in tile coordinates.
    pub x: f32,
    pub y: f32,
    pub dir: Dir,
    pub fire: FireController,
    /// Desired movement direction this tick (`None` = stand still).
    pub move_dir: Option<Dir>,
}

#[derive(Debug, Clone)]
pub struct Bullet {
    pub id: u64,
    pub owner_id: u64,
    pub x: f32,
    pub y: f32,
    pub dir: Dir,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    pub tank_speed: f32,
    pub bullet_speed: f32,
    pub tank_half: f32,
    pub bullet_half: f32,
    pub fire: FireConfig,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            tank_speed: 4.0,   // tiles per second
            bullet_speed: 12.0,
            tank_half: 0.4,
            bullet_half: 0.12,
            fire: FireConfig::default(),
        }
    }
}

/// Per-tick input applied to the player tank before stepping.
#[derive(Debug, Clone, Default)]
pub struct SimInput {
    pub move_dir: Option<Dir>,
    pub fire_held: bool,
    pub toggle_autofire: bool,
}

/// Pure simulation state.
#[derive(Debug, Clone)]
pub struct World {
    pub map: Map,
    pub tanks: Vec<Tank>,
    pub bullets: Vec<Bullet>,
    pub config: SimConfig,
    pub tick: u64,
    next_id: u64,
    player_tank_id: u64,
}

impl World {
    pub fn new(map: Map, config: SimConfig) -> Self {
        let mut world = Self {
            map,
            tanks: Vec::new(),
            bullets: Vec::new(),
            config,
            tick: 0,
            next_id: 1,
            player_tank_id: 0,
        };

        let spawn = world
            .map
            .player_spawns()
            .first()
            .copied()
            .unwrap_or(crate::map::SpawnPoint { x: 1, y: 1 });

        let id = world.alloc_id();
        world.player_tank_id = id;
        world.tanks.push(Tank {
            id,
            team: Team::Player,
            x: spawn.x as f32 + 0.5,
            y: spawn.y as f32 + 0.5,
            dir: Dir::Up,
            fire: FireController::new(world.config.fire),
            move_dir: None,
        });
        world
    }

    pub fn player_tank_id(&self) -> u64 {
        self.player_tank_id
    }

    pub fn player(&self) -> Option<&Tank> {
        self.tanks.iter().find(|t| t.id == self.player_tank_id)
    }

    pub fn player_mut(&mut self) -> Option<&mut Tank> {
        let id = self.player_tank_id;
        self.tanks.iter_mut().find(|t| t.id == id)
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Apply input then advance simulation by `dt` seconds (usually `1.0 / tick_hz`).
    pub fn step(&mut self, input: &SimInput, dt: f32) {
        if let Some(player) = self.player_mut() {
            player.move_dir = input.move_dir;
            if let Some(dir) = input.move_dir {
                player.dir = dir;
            }
            player.fire.set_fire_held(input.fire_held);
            if input.toggle_autofire {
                player.fire.toggle_autofire();
            }
        }

        self.move_tanks(dt);
        self.fire_weapons(dt);
        self.move_bullets(dt);
        self.tick += 1;
    }

    fn move_tanks(&mut self, dt: f32) {
        let speed = self.config.tank_speed;
        let half = self.config.tank_half;
        let map = &self.map;

        for tank in &mut self.tanks {
            let Some(dir) = tank.move_dir else {
                continue;
            };
            let (dx, dy) = dir.delta();
            let nx = tank.x + dx * speed * dt;
            let ny = tank.y + dy * speed * dt;

            if Self::tank_fits(map, nx, ny, half) {
                tank.x = nx;
                tank.y = ny;
            } else {
                // Axis-aligned slide: try X then Y for less sticky corners.
                let try_x = tank.x + dx * speed * dt;
                if dx != 0.0 && Self::tank_fits(map, try_x, tank.y, half) {
                    tank.x = try_x;
                }
                let try_y = tank.y + dy * speed * dt;
                if dy != 0.0 && Self::tank_fits(map, tank.x, try_y, half) {
                    tank.y = try_y;
                }
            }
        }
    }

    fn tank_fits(map: &Map, cx: f32, cy: f32, half: f32) -> bool {
        // Keep the AABB inside the map.
        if cx - half < 0.0
            || cy - half < 0.0
            || cx + half > map.width() as f32
            || cy + half > map.height() as f32
        {
            return false;
        }
        for (_x, _y, tile) in map.tiles_overlapping(cx, cy, half, half) {
            if tile.blocks_tank() {
                return false;
            }
        }
        true
    }

    fn fire_weapons(&mut self, dt: f32) {
        let muzzle = self.config.tank_half + self.config.bullet_half;
        let live_counts: Vec<usize> = self
            .tanks
            .iter()
            .map(|t| self.live_bullets(t.id))
            .collect();

        let mut shots: Vec<(u64, f32, f32, Dir)> = Vec::new();
        for (tank, &live) in self.tanks.iter_mut().zip(live_counts.iter()) {
            tank.fire.tick(dt);
            if tank.fire.try_fire(live) {
                let (dx, dy) = tank.dir.delta();
                shots.push((
                    tank.id,
                    tank.x + dx * muzzle,
                    tank.y + dy * muzzle,
                    tank.dir,
                ));
            }
        }

        for (owner_id, x, y, dir) in shots {
            let id = self.alloc_id();
            self.bullets.push(Bullet {
                id,
                owner_id,
                x,
                y,
                dir,
            });
        }
    }

    fn move_bullets(&mut self, dt: f32) {
        let speed = self.config.bullet_speed;
        let half = self.config.bullet_half;
        let mut remove = Vec::new();
        let mut tile_hits: Vec<(u32, u32)> = Vec::new();

        for (idx, bullet) in self.bullets.iter_mut().enumerate() {
            let (dx, dy) = bullet.dir.delta();
            bullet.x += dx * speed * dt;
            bullet.y += dy * speed * dt;

            // Out of map → despawn.
            if bullet.x < 0.0
                || bullet.y < 0.0
                || bullet.x >= self.map.width() as f32
                || bullet.y >= self.map.height() as f32
            {
                remove.push(idx);
                continue;
            }

            let overlaps = self
                .map
                .tiles_overlapping(bullet.x, bullet.y, half, half);
            let mut hit = false;
            for (tx, ty, tile) in overlaps {
                if tile.blocks_bullet() {
                    hit = true;
                    if tile.destroyed_by_bullet() {
                        tile_hits.push((tx, ty));
                    }
                }
            }
            if hit {
                remove.push(idx);
            }
        }

        for (tx, ty) in tile_hits {
            let _ = self.map.set(tx, ty, Tile::Empty);
        }

        // Remove high indices first.
        remove.sort_unstable();
        remove.dedup();
        for idx in remove.into_iter().rev() {
            self.bullets.swap_remove(idx);
        }
    }

    /// Count live bullets owned by `tank_id`.
    pub fn live_bullets(&self, tank_id: u64) -> usize {
        self.bullets.iter().filter(|b| b.owner_id == tank_id).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Map;

    fn tiny_world(map: &str) -> World {
        let map = Map::parse(map).unwrap();
        World::new(
            map,
            SimConfig {
                tank_speed: 10.0,
                bullet_speed: 20.0,
                tank_half: 0.35,
                bullet_half: 0.1,
                fire: FireConfig {
                    fire_interval: 0.1,
                    max_bullets: 2,
                },
            },
        )
    }

    #[test]
    fn brick_destroyed_by_bullet_steel_not() {
        // Player at (1,2) facing up toward brick at (1,1) then steel at (1,0) — wait,
        // layout:
        // S . .
        // # . .
        // . P .
        let mut world = tiny_world(
            "\
S..
#..
.P.
",
        );
        {
            let p = world.player_mut().unwrap();
            p.dir = Dir::Up;
            p.x = 0.5;
            p.y = 2.5;
            p.fire.autofire = true;
            p.fire.config.max_bullets = 4;
            p.fire.config.fire_interval = 0.0;
            p.fire.cooldown = 0.0;
        }

        // Fire until brick is gone.
        for _ in 0..30 {
            world.step(
                &SimInput {
                    fire_held: true,
                    ..Default::default()
                },
                1.0 / 60.0,
            );
        }
        assert_eq!(world.map.get(0, 1), Some(Tile::Empty), "brick should break");
        assert_eq!(world.map.get(0, 0), Some(Tile::Steel), "steel must remain");
    }

    #[test]
    fn water_blocks_tank_bullets_pass() {
        let mut world = tiny_world(
            "\
...
~~~
.P.
",
        );
        {
            let p = world.player_mut().unwrap();
            p.x = 1.5;
            p.y = 2.5;
            p.dir = Dir::Up;
        }
        let start_y = world.player().unwrap().y;
        for _ in 0..60 {
            world.step(
                &SimInput {
                    move_dir: Some(Dir::Up),
                    ..Default::default()
                },
                1.0 / 60.0,
            );
        }
        let after = world.player().unwrap().y;
        assert!(
            after > start_y - 0.6,
            "tank must not cross water (y={after} from {start_y})"
        );
        assert!(after >= 1.5, "should stay south of water row");

        // One shot across water (no autofire — avoid respawn after exit).
        {
            let p = world.player_mut().unwrap();
            p.x = 1.5;
            p.y = 2.5;
            p.dir = Dir::Up;
            p.fire.autofire = false;
            p.fire.config.fire_interval = 0.0;
            p.fire.config.max_bullets = 1;
            p.fire.cooldown = 0.0;
        }
        world.step(
            &SimInput {
                fire_held: true,
                ..Default::default()
            },
            1.0 / 60.0,
        );
        assert_eq!(world.bullets.len(), 1);
        let mut crossed_water = false;
        for _ in 0..40 {
            world.step(&SimInput::default(), 1.0 / 60.0);
            if let Some(b) = world.bullets.first() {
                if b.y < 2.0 && b.y > 0.0 {
                    // Still in playfield over/near water row — not destroyed by water.
                    crossed_water = true;
                }
            }
        }
        assert!(
            crossed_water || world.bullets.is_empty(),
            "bullet must travel over water (not stop on it)"
        );
        assert!(
            world.bullets.is_empty(),
            "bullet should leave the map after passing water"
        );
        assert_eq!(world.map.get(1, 1), Some(Tile::Water));
    }

    #[test]
    fn forest_passable_and_not_cleared_by_normal_fire() {
        let mut world = tiny_world(
            "\
...
FFF
.P.
",
        );
        {
            let p = world.player_mut().unwrap();
            p.x = 1.5;
            p.y = 2.5;
            p.dir = Dir::Up;
        }
        for _ in 0..90 {
            world.step(
                &SimInput {
                    move_dir: Some(Dir::Up),
                    ..Default::default()
                },
                1.0 / 60.0,
            );
        }
        let y = world.player().unwrap().y;
        assert!(y < 1.5, "tank should drive through forest, y={y}");

        // Shoot into forest — tiles remain Forest.
        {
            let p = world.player_mut().unwrap();
            p.x = 1.5;
            p.y = 2.5;
            p.dir = Dir::Up;
            p.fire.autofire = true;
            p.fire.config.fire_interval = 0.0;
            p.fire.config.max_bullets = 3;
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
        assert_eq!(world.map.get(1, 1), Some(Tile::Forest));
        assert_eq!(world.map.get(0, 1), Some(Tile::Forest));
    }

    #[test]
    fn autofire_respects_fire_rate_and_bullet_cap() {
        // Tall empty corridor + slow bullets so the first shot stays alive.
        let mut world = World::new(
            Map::parse(
                "\
.
.
.
.
.
.
.
.
.
P
.
.
",
            )
            .unwrap(),
            SimConfig {
                tank_speed: 10.0,
                bullet_speed: 2.0,
                tank_half: 0.35,
                bullet_half: 0.1,
                fire: FireConfig {
                    fire_interval: 0.5,
                    max_bullets: 1,
                },
            },
        );
        {
            let p = world.player_mut().unwrap();
            p.dir = Dir::Up;
            p.fire.cooldown = 0.0;
            p.fire.autofire = true;
        }

        world.step(&SimInput::default(), 1.0 / 60.0);
        assert_eq!(world.bullets.len(), 1, "first autofire shot");

        let mut max_live = 1usize;
        for _ in 0..45 {
            world.step(&SimInput::default(), 1.0 / 60.0);
            max_live = max_live.max(world.live_bullets(world.player_tank_id()));
        }
        assert_eq!(max_live, 1, "cap is 1 while bullet exists");

        // After enough time the bullet exits; never exceed cap.
        for _ in 0..300 {
            world.step(&SimInput::default(), 1.0 / 60.0);
            assert!(
                world.live_bullets(world.player_tank_id()) <= 1,
                "never above max_bullets"
            );
        }
    }
}
