//! Fixed-tick world: tanks, bullets, tile collisions.

use crate::bonus::{
    bullet_speed_for_upgrade, fire_for_upgrade, upgrade_breaks_steel, upgrade_clears_forest,
    ActiveBonus, BonusCatalog, BonusPickup, BonusState, EffectKind, TankMods,
};
use crate::fire::{FireConfig, FireController};
use crate::lives::{LivesConfig, PlayerProgress};
use crate::map::Map;
use crate::score::{EnemyKind, ScoreConfig, StyleState};
use crate::tile::Tile;
use crate::wave::{WaveConfig, WaveState};

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
    /// Scoring weight when this tank is an enemy.
    pub enemy_kind: EnemyKind,
    /// Center position in tile coordinates.
    pub x: f32,
    pub y: f32,
    pub dir: Dir,
    pub fire: FireController,
    /// Desired movement direction this tick (`None` = stand still).
    pub move_dir: Option<Dir>,
    /// Star / ship / helmet / forest flags from bonuses.
    pub mods: TankMods,
}

#[derive(Debug, Clone)]
pub struct Bullet {
    pub id: u64,
    pub owner_id: u64,
    pub team: Team,
    pub x: f32,
    pub y: f32,
    pub dir: Dir,
    /// Tiles/sec at spawn (scales with owner star level).
    pub speed: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    pub tank_speed: f32,
    pub bullet_speed: f32,
    pub tank_half: f32,
    pub bullet_half: f32,
    pub fire: FireConfig,
    pub wave: WaveConfig,
    pub lives: LivesConfig,
    pub score: ScoreConfig,
    /// Enemy weapon (autofire while on field).
    pub enemy_fire: FireConfig,
    /// Data-driven bonus catalog (empty = no drops).
    pub bonuses: BonusCatalog,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            tank_speed: 4.0, // tiles per second
            // Base (no stars); stars multiply via `bullet_speed_for_upgrade`.
            bullet_speed: 6.0,
            tank_half: 0.4,
            bullet_half: 0.12,
            fire: FireConfig::default(),
            wave: WaveConfig::default(),
            lives: LivesConfig::default(),
            score: ScoreConfig::default(),
            enemy_fire: FireConfig {
                fire_interval: 1.6,
                max_bullets: 1,
            },
            bonuses: BonusCatalog::default(),
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

/// End-of-round result for the current level attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoundOutcome {
    #[default]
    Playing,
    /// All enemies from the wave destroyed; base still standing.
    Victory,
    /// Base (eagle) was destroyed by a bullet.
    DefeatBaseDestroyed,
    /// Player lost the last life.
    DefeatNoLives,
}

/// Pure simulation state.
#[derive(Debug, Clone)]
pub struct World {
    pub map: Map,
    pub tanks: Vec<Tank>,
    pub bullets: Vec<Bullet>,
    pub config: SimConfig,
    pub wave: WaveState,
    pub progress: PlayerProgress,
    pub style: StyleState,
    pub bonuses: BonusState,
    pub outcome: RoundOutcome,
    pub tick: u64,
    next_id: u64,
    player_tank_id: u64,
    /// Round-robin index into enemy spawn points.
    next_enemy_spawn: usize,
}

impl World {
    pub fn new(map: Map, config: SimConfig) -> Self {
        let wave = WaveState::new(config.wave);
        let progress = PlayerProgress::new(config.lives);
        let bonuses = BonusState::new(config.bonuses.clone());
        let mut world = Self {
            map,
            tanks: Vec::new(),
            bullets: Vec::new(),
            config,
            wave,
            progress,
            style: StyleState::default(),
            bonuses,
            outcome: RoundOutcome::Playing,
            tick: 0,
            next_id: 1,
            player_tank_id: 0,
            next_enemy_spawn: 0,
        };
        world.spawn_player_tank();
        world
    }

    fn player_spawn_pos(&self) -> (f32, f32) {
        let spawn = self
            .map
            .player_spawns()
            .first()
            .copied()
            .unwrap_or(crate::map::SpawnPoint { x: 1, y: 1 });
        (spawn.x as f32 + 0.5, spawn.y as f32 + 0.5)
    }

    fn spawn_player_tank(&mut self) {
        let (x, y) = self.player_spawn_pos();
        let id = self.alloc_id();
        self.player_tank_id = id;
        let upgrade = self.progress.tank_upgrade.min(TankMods::MAX_UPGRADE);
        let (interval, max_b) = fire_for_upgrade(
            self.config.fire.fire_interval,
            self.config.fire.max_bullets,
            upgrade,
        );
        let mut fire = FireController::new(FireConfig {
            fire_interval: interval,
            max_bullets: max_b,
        });
        // Keep autofire off on fresh spawn; carry only weapon power + ship.
        fire.autofire = false;
        self.tanks.push(Tank {
            id,
            team: Team::Player,
            enemy_kind: EnemyKind::Basic,
            x,
            y,
            dir: Dir::Up,
            fire,
            move_dir: None,
            mods: TankMods {
                upgrade,
                forest_clear: upgrade_clears_forest(upgrade),
                water_walk: self.progress.ship,
                invuln: false,
            },
        });
    }

    /// After loading carried `PlayerProgress`, rebuild the player tank so stars apply.
    pub fn reapply_player_from_progress(&mut self) {
        let pid = self.player_tank_id;
        self.tanks.retain(|t| t.id != pid);
        self.bullets.retain(|b| b.owner_id != pid);
        self.spawn_player_tank();
    }

    /// Spawn a pickup by catalog id at tile center (tests / debug).
    pub fn spawn_pickup_at(&mut self, bonus_id: &str, x: f32, y: f32) -> Option<u64> {
        let (bonus_id, letter) = {
            let def = self.bonuses.catalog.get(bonus_id)?;
            (def.id.clone(), def.letter)
        };
        let lifetime = self.bonuses.catalog.pickup_lifetime_secs;
        let id = self.alloc_id();
        self.bonuses.pickups.push(BonusPickup {
            id,
            bonus_id,
            letter,
            x,
            y,
            lifetime_secs: lifetime,
        });
        Some(id)
    }

    pub fn enemies_on_field(&self) -> u32 {
        self.tanks
            .iter()
            .filter(|t| t.team == Team::Enemy)
            .count() as u32
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
        if self.outcome != RoundOutcome::Playing {
            return;
        }

        self.progress.tick_invuln(dt);
        let fortify_was = self.bonuses.fortify_secs > 0.0;
        self.bonuses.tick_timers(dt);
        if fortify_was && self.bonuses.fortify_secs <= 0.0 {
            self.restore_fortify_walls();
        }
        self.sync_tank_mod_flags();

        let player_frozen = self.bonuses.opponents_frozen(Team::Player);
        if let Some(player) = self.player_mut() {
            if player_frozen {
                player.move_dir = None;
                player.fire.set_fire_held(false);
            } else {
                player.move_dir = input.move_dir;
                if let Some(dir) = input.move_dir {
                    player.dir = dir;
                }
                player.fire.set_fire_held(input.fire_held);
                if input.toggle_autofire {
                    player.fire.toggle_autofire();
                }
            }
        }

        self.move_tanks(dt);
        self.fire_weapons(dt);
        let base_hit = self.move_bullets(dt);
        self.resolve_bullet_bullet_hits();
        self.resolve_bullet_tank_hits();
        self.resolve_pickups();
        // Spawn after combat so a dying tank frees a slot the same tick,
        // and leftover bullets don't instantly delete the newborn.
        self.spawn_enemies(dt);

        if base_hit {
            self.outcome = RoundOutcome::DefeatBaseDestroyed;
        } else if self.outcome == RoundOutcome::Playing
            && self.wave.is_cleared(self.enemies_on_field())
        {
            let (bonus, _) = self.style.award_clear(&self.config.score);
            self.progress.add_score(bonus);
            self.bonuses.cancel_on_level_end();
            self.restore_fortify_walls();
            self.outcome = RoundOutcome::Victory;
        }

        self.tick += 1;
    }

    pub fn is_playing(&self) -> bool {
        self.outcome == RoundOutcome::Playing
    }

    fn spawn_enemies(&mut self, dt: f32) {
        self.wave.tick_cooldown(dt);
        let on_field = self.enemies_on_field();
        if !self.wave.can_spawn(on_field) {
            return;
        }
        let spawns = self.map.enemy_spawns();
        if spawns.is_empty() {
            return;
        }

        let half = self.config.tank_half;
        let n = spawns.len();
        for attempt in 0..n {
            let idx = (self.next_enemy_spawn + attempt) % n;
            let sp = spawns[idx];
            let cx = sp.x as f32 + 0.5;
            let cy = sp.y as f32 + 0.5;
            if !Self::tank_fits(&self.map, cx, cy, half, false) {
                continue;
            }
            if self.tank_occupies(cx, cy, half, None) {
                continue;
            }

            let id = self.alloc_id();
            let mut fire = FireController::new(self.config.enemy_fire);
            fire.autofire = true;
            self.tanks.push(Tank {
                id,
                team: Team::Enemy,
                enemy_kind: EnemyKind::Basic,
                x: cx,
                y: cy,
                dir: Dir::Down,
                fire,
                // Stationary for now; AI is phase 4. They still shoot downward.
                move_dir: None,
                mods: TankMods::default(),
            });
            self.wave.on_spawned();
            self.next_enemy_spawn = (idx + 1) % n;
            return;
        }
    }

    /// True if any tank's AABB overlaps the given box (optionally ignoring one id).
    fn tank_occupies(&self, cx: f32, cy: f32, half: f32, ignore_id: Option<u64>) -> bool {
        for t in &self.tanks {
            if ignore_id == Some(t.id) {
                continue;
            }
            let dx = (t.x - cx).abs();
            let dy = (t.y - cy).abs();
            if dx < half * 2.0 && dy < half * 2.0 {
                return true;
            }
        }
        false
    }

    /// Any overlapping bullets cancel each other (including same team).
    fn resolve_bullet_bullet_hits(&mut self) {
        let half = self.config.bullet_half;
        let n = self.bullets.len();
        if n < 2 {
            return;
        }
        let mut dead = vec![false; n];
        for i in 0..n {
            if dead[i] {
                continue;
            }
            for j in (i + 1)..n {
                if dead[j] {
                    continue;
                }
                let a = &self.bullets[i];
                let b = &self.bullets[j];
                let dx = (a.x - b.x).abs();
                let dy = (a.y - b.y).abs();
                if dx < half * 2.0 && dy < half * 2.0 {
                    dead[i] = true;
                    dead[j] = true;
                    break;
                }
            }
        }
        for i in (0..n).rev() {
            if dead[i] {
                self.bullets.swap_remove(i);
            }
        }
    }

    fn resolve_bullet_tank_hits(&mut self) {
        let half_b = self.config.bullet_half;
        let half_t = self.config.tank_half;
        let player_invuln = self.progress.is_invulnerable()
            || self
                .player()
                .map(|t| t.mods.invuln)
                .unwrap_or(false);
        let mut dead_bullets = Vec::new();
        let mut hit_tanks = Vec::new();

        for (bi, bullet) in self.bullets.iter().enumerate() {
            for tank in &self.tanks {
                if tank.id == bullet.owner_id || tank.team == bullet.team {
                    continue;
                }
                let dx = (tank.x - bullet.x).abs();
                let dy = (tank.y - bullet.y).abs();
                if dx < half_t + half_b && dy < half_t + half_b {
                    dead_bullets.push(bi);
                    let tank_invuln = match tank.team {
                        Team::Player => player_invuln,
                        Team::Enemy => tank.mods.invuln,
                    };
                    if !tank_invuln {
                        hit_tanks.push(tank.id);
                    }
                    break;
                }
            }
        }

        dead_bullets.sort_unstable();
        dead_bullets.dedup();
        for bi in dead_bullets.into_iter().rev() {
            self.bullets.swap_remove(bi);
        }

        hit_tanks.sort_unstable();
        hit_tanks.dedup();
        let mut dead_tanks = Vec::new();
        for tid in hit_tanks {
            if !self.absorb_hit_or_strip_ship(tid) {
                dead_tanks.push(tid);
            }
        }

        let mut player_died = false;
        let mut drop_positions: Vec<(f32, f32)> = Vec::new();
        for tid in dead_tanks {
            if let Some(pos) = self.tanks.iter().position(|t| t.id == tid) {
                let team = self.tanks[pos].team;
                let kind = self.tanks[pos].enemy_kind;
                let (dx, dy) = (self.tanks[pos].x, self.tanks[pos].y);
                self.bonuses.cancel_on_death(team, tid);
                self.tanks.swap_remove(pos);
                self.bullets.retain(|b| b.owner_id != tid);
                match team {
                    Team::Enemy => {
                        self.wave.on_destroyed();
                        let (pts, _) = self.style.award_kill(&self.config.score, kind);
                        self.progress.add_score(pts);
                        drop_positions.push((dx, dy));
                    }
                    Team::Player => {
                        player_died = true;
                    }
                }
            }
        }

        for (x, y) in drop_positions {
            self.maybe_drop_pickup(x, y);
        }

        if player_died {
            self.style.on_player_death();
            if self.progress.on_tank_destroyed() {
                self.spawn_player_tank();
                self.progress.start_invuln();
            } else {
                self.outcome = RoundOutcome::DefeatNoLives;
            }
        }
    }

    fn move_tanks(&mut self, dt: f32) {
        let speed = self.config.tank_speed;
        let half = self.config.tank_half;
        let n = self.tanks.len();
        let enemies_frozen = self.bonuses.opponents_frozen(Team::Enemy);

        for i in 0..n {
            if self.tanks[i].team == Team::Enemy && enemies_frozen {
                continue;
            }
            let Some(dir) = self.tanks[i].move_dir else {
                continue;
            };
            let id = self.tanks[i].id;
            let water_walk = self.tanks[i].mods.water_walk;
            let (dx, dy) = dir.delta();
            let x = self.tanks[i].x;
            let y = self.tanks[i].y;
            let nx = x + dx * speed * dt;
            let ny = y + dy * speed * dt;

            if self.can_tank_move(id, nx, ny, half, water_walk) {
                self.tanks[i].x = nx;
                self.tanks[i].y = ny;
            } else {
                // Axis-aligned slide: try X then Y for less sticky corners.
                let try_x = x + dx * speed * dt;
                if dx != 0.0 && self.can_tank_move(id, try_x, y, half, water_walk) {
                    self.tanks[i].x = try_x;
                }
                let y = self.tanks[i].y;
                let try_y = y + dy * speed * dt;
                if dy != 0.0 && self.can_tank_move(id, self.tanks[i].x, try_y, half, water_walk)
                {
                    self.tanks[i].y = try_y;
                }
            }
        }
    }

    fn can_tank_move(&self, id: u64, cx: f32, cy: f32, half: f32, water_walk: bool) -> bool {
        Self::tank_fits(&self.map, cx, cy, half, water_walk)
            && !self.tank_occupies(cx, cy, half, Some(id))
    }

    fn tank_fits(map: &Map, cx: f32, cy: f32, half: f32, water_walk: bool) -> bool {
        // Keep the AABB inside the map.
        if cx - half < 0.0
            || cy - half < 0.0
            || cx + half > map.width() as f32
            || cy + half > map.height() as f32
        {
            return false;
        }
        for (_x, _y, tile) in map.tiles_overlapping(cx, cy, half, half) {
            if water_walk && tile == Tile::Water {
                continue;
            }
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
        let enemies_frozen = self.bonuses.opponents_frozen(Team::Enemy);
        let player_frozen = self.bonuses.opponents_frozen(Team::Player);

        let mut shots: Vec<(u64, Team, f32, f32, Dir, f32)> = Vec::new();
        for (tank, &live) in self.tanks.iter_mut().zip(live_counts.iter()) {
            let frozen = match tank.team {
                Team::Enemy => enemies_frozen,
                Team::Player => player_frozen,
            };
            tank.fire.tick(dt);
            if frozen {
                continue;
            }
            if tank.fire.try_fire(live) {
                let (dx, dy) = tank.dir.delta();
                let speed =
                    bullet_speed_for_upgrade(self.config.bullet_speed, tank.mods.upgrade);
                shots.push((
                    tank.id,
                    tank.team,
                    tank.x + dx * muzzle,
                    tank.y + dy * muzzle,
                    tank.dir,
                    speed,
                ));
            }
        }

        for (owner_id, team, x, y, dir, speed) in shots {
            let id = self.alloc_id();
            self.bullets.push(Bullet {
                id,
                owner_id,
                team,
                x,
                y,
                dir,
                speed,
            });
        }
    }

    /// Move bullets and apply tile hits. Returns `true` if the base was destroyed.
    fn move_bullets(&mut self, dt: f32) -> bool {
        let half = self.config.bullet_half;
        let mut remove = Vec::new();
        let mut tile_hits: Vec<(u32, u32, bool, bool)> = Vec::new();

        for (idx, bullet) in self.bullets.iter_mut().enumerate() {
            let (dx, dy) = bullet.dir.delta();
            let speed = bullet.speed;
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

            let owner_mods = self
                .tanks
                .iter()
                .find(|t| t.id == bullet.owner_id)
                .map(|t| t.mods)
                .unwrap_or_default();
            let break_steel = upgrade_breaks_steel(owner_mods.upgrade);
            // Forest only at ★4 / 2nd gun — not automatically with steel power.
            let clear_forest = owner_mods.forest_clear;

            let overlaps = self
                .map
                .tiles_overlapping(bullet.x, bullet.y, half, half);
            let mut hit = false;
            for (tx, ty, tile) in overlaps {
                if tile == Tile::Forest && clear_forest {
                    tile_hits.push((tx, ty, false, true));
                    continue;
                }
                if tile.blocks_bullet() {
                    hit = true;
                    let can_break = tile.destroyed_by_bullet()
                        || (tile == Tile::Steel && break_steel);
                    if can_break {
                        tile_hits.push((tx, ty, tile == Tile::Base, false));
                    }
                }
            }
            if hit {
                remove.push(idx);
            }
        }

        let mut base_destroyed = false;
        for (tx, ty, is_base, _forest) in tile_hits {
            if is_base || self.map.get(tx, ty) == Some(Tile::Base) {
                base_destroyed = true;
            }
            let _ = self.map.set(tx, ty, Tile::Empty);
        }

        // Remove high indices first.
        remove.sort_unstable();
        remove.dedup();
        for idx in remove.into_iter().rev() {
            self.bullets.swap_remove(idx);
        }

        base_destroyed
    }

    fn maybe_drop_pickup(&mut self, near_x: f32, near_y: f32) {
        if self.bonuses.catalog.defs.is_empty() {
            return;
        }
        self.bonuses.kills_since_drop = self.bonuses.kills_since_drop.saturating_add(1);
        if self.bonuses.kills_since_drop < self.bonuses.catalog.drop_every_kills {
            return;
        }
        self.bonuses.kills_since_drop = 0;
        let Some(def) = self
            .bonuses
            .catalog
            .by_index(self.bonuses.next_drop_index)
            .cloned()
        else {
            return;
        };
        self.bonuses.next_drop_index = self.bonuses.next_drop_index.wrapping_add(1);
        let (x, y) = self.random_pickup_spawn_pos(near_x, near_y);
        let lifetime = self.bonuses.catalog.pickup_lifetime_secs;
        let id = self.alloc_id();
        self.bonuses.pickups.push(BonusPickup {
            id,
            bonus_id: def.id,
            letter: def.letter,
            x,
            y,
            lifetime_secs: lifetime,
        });
    }

    /// Random empty tile (not under a tank, not the kill cell). `near_*` = kill pos to avoid.
    fn random_pickup_spawn_pos(&self, avoid_x: f32, avoid_y: f32) -> (f32, f32) {
        let avoid_tx = avoid_x.floor() as i32;
        let avoid_ty = avoid_y.floor() as i32;
        let mut candidates: Vec<(u32, u32)> = Vec::new();
        for y in 0..self.map.height() {
            for x in 0..self.map.width() {
                if self.map.get(x, y) != Some(Tile::Empty) {
                    continue;
                }
                if x as i32 == avoid_tx && y as i32 == avoid_ty {
                    continue;
                }
                let cx = x as f32 + 0.5;
                let cy = y as f32 + 0.5;
                if self.tank_occupies(cx, cy, self.config.tank_half, None) {
                    continue;
                }
                if self
                    .bonuses
                    .pickups
                    .iter()
                    .any(|p| (p.x - cx).abs() < 0.4 && (p.y - cy).abs() < 0.4)
                {
                    continue;
                }
                candidates.push((x, y));
            }
        }
        if candidates.is_empty() {
            // Map full — fall back to any empty, including avoid cell.
            for y in 0..self.map.height() {
                for x in 0..self.map.width() {
                    if self.map.get(x, y) == Some(Tile::Empty) {
                        return (x as f32 + 0.5, y as f32 + 0.5);
                    }
                }
            }
            return (avoid_x, avoid_y);
        }
        // Deterministic LCG from sim state (no RNG crate).
        let mix = self
            .tick
            .wrapping_mul(1_103_515_245)
            .wrapping_add(self.next_id)
            .wrapping_add((avoid_x * 17.0 + avoid_y * 31.0) as u64);
        let idx = (mix as usize) % candidates.len();
        let (x, y) = candidates[idx];
        (x as f32 + 0.5, y as f32 + 0.5)
    }

    fn resolve_pickups(&mut self) {
        if self.bonuses.pickups.is_empty() {
            return;
        }
        let half = self.config.tank_half;
        let mut collected: Vec<(usize, u64)> = Vec::new();
        for (pi, pickup) in self.bonuses.pickups.iter().enumerate() {
            let Some(def) = self.bonuses.catalog.get(&pickup.bonus_id) else {
                continue;
            };
            for tank in &self.tanks {
                if !def.who.allows(tank.team) {
                    continue;
                }
                let dx = (tank.x - pickup.x).abs();
                let dy = (tank.y - pickup.y).abs();
                if dx < half + 0.35 && dy < half + 0.35 {
                    collected.push((pi, tank.id));
                    break;
                }
            }
        }
        collected.sort_by_key(|(pi, _)| *pi);
        collected.dedup_by_key(|(pi, _)| *pi);
        for (pi, tank_id) in collected.into_iter().rev() {
            let pickup = self.bonuses.pickups.swap_remove(pi);
            self.apply_bonus_pickup(&pickup.bonus_id, tank_id);
        }
    }

    fn apply_bonus_pickup(&mut self, bonus_id: &str, tank_id: u64) {
        let Some(def) = self.bonuses.catalog.get(bonus_id).cloned() else {
            return;
        };
        let Some(team) = self.tanks.iter().find(|t| t.id == tank_id).map(|t| t.team) else {
            return;
        };

        if team == Team::Player && def.score > 0 {
            self.progress.add_score(def.score);
        }

        if let Some(group) = def.exclusive_group.as_ref() {
            self.bonuses.active.retain(|a| {
                a.exclusive_group.as_ref() != Some(group) || a.tank_id != Some(tank_id)
            });
        }

        for effect in &def.effects {
            self.apply_effect(*effect, &def, tank_id, team);
        }

        let timed = def.duration_secs > 0.0;
        let sticky = def.cancel.on_death || def.cancel.on_level_end;
        if timed || sticky {
            self.bonuses.active.push(ActiveBonus {
                bonus_id: def.id.clone(),
                letter: def.letter,
                name: def.name.clone(),
                team,
                tank_id: Some(tank_id),
                remaining_secs: if timed {
                    Some(def.duration_secs)
                } else {
                    None
                },
                cancel: def.cancel,
                exclusive_group: def.exclusive_group.clone(),
                effects: def.effects.clone(),
            });
        }
        self.sync_tank_mod_flags();
    }

    fn apply_effect(
        &mut self,
        effect: EffectKind,
        def: &crate::bonus::BonusDef,
        tank_id: u64,
        team: Team,
    ) {
        match effect {
            EffectKind::Invuln => {
                if team == Team::Player {
                    self.progress.invuln_remaining =
                        self.progress.invuln_remaining.max(def.duration_secs);
                }
                if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                    t.mods.invuln = true;
                }
            }
            EffectKind::FreezeOpponents => {
                match team {
                    Team::Player => {
                        self.bonuses.freeze_enemy_secs =
                            self.bonuses.freeze_enemy_secs.max(def.duration_secs);
                    }
                    Team::Enemy => {
                        self.bonuses.freeze_player_secs =
                            self.bonuses.freeze_player_secs.max(def.duration_secs);
                    }
                }
            }
            EffectKind::FortifyBase => {
                if team == Team::Player {
                    self.apply_fortify_walls(def.duration_secs);
                }
                // Enemy shovel weaken — phase 3.2 polish.
            }
            EffectKind::KillOpponents => {
                let victims: Vec<u64> = self
                    .tanks
                    .iter()
                    .filter(|t| t.team != team)
                    .map(|t| t.id)
                    .collect();
                for vid in victims {
                    if let Some(pos) = self.tanks.iter().position(|t| t.id == vid) {
                        let vteam = self.tanks[pos].team;
                        let kind = self.tanks[pos].enemy_kind;
                        self.bonuses.cancel_on_death(vteam, vid);
                        self.tanks.swap_remove(pos);
                        self.bullets.retain(|b| b.owner_id != vid);
                        if vteam == Team::Enemy {
                            self.wave.on_destroyed();
                            if team == Team::Player {
                                let (pts, _) = self.style.award_kill(&self.config.score, kind);
                                self.progress.add_score(pts);
                            }
                        } else if vteam == Team::Player {
                            self.style.on_player_death();
                            if self.progress.on_tank_destroyed() {
                                self.spawn_player_tank();
                                self.progress.start_invuln();
                            } else {
                                self.outcome = RoundOutcome::DefeatNoLives;
                            }
                        }
                    }
                }
            }
            EffectKind::TankUpgrade => {
                if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                    let cap = if def.max_stacks == 0 {
                        TankMods::MAX_UPGRADE
                    } else {
                        def.max_stacks.min(u32::from(TankMods::MAX_UPGRADE)) as u8
                    };
                    t.mods.upgrade = (t.mods.upgrade + 1).min(cap);
                    t.mods.sync_forest();
                    if team == Team::Player {
                        self.progress.tank_upgrade = t.mods.upgrade;
                    }
                    self.refresh_tank_fire(tank_id);
                }
            }
            EffectKind::MaxUpgrade => {
                // 1st gun → ★3 (fire/steel); 2nd gun (or gun at ★3+) → ★4 forest.
                if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                    if t.mods.upgrade < TankMods::MAX_FIRE_UPGRADE {
                        t.mods.upgrade = TankMods::MAX_FIRE_UPGRADE;
                    } else {
                        t.mods.upgrade = TankMods::MAX_UPGRADE;
                    }
                    t.mods.sync_forest();
                    if team == Team::Player {
                        self.progress.tank_upgrade = t.mods.upgrade;
                    }
                    self.refresh_tank_fire(tank_id);
                }
            }
            EffectKind::ExtraLife => {
                if team == Team::Player {
                    let _ = self.progress.try_add_life();
                }
            }
            EffectKind::WaterWalk => {
                if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                    t.mods.water_walk = true;
                }
                if team == Team::Player {
                    self.progress.ship = true;
                }
            }
            EffectKind::ForestClearShots => {
                // Legacy / unused letter — forest comes from ★4 / 2nd gun.
                if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                    t.mods.upgrade = t.mods.upgrade.max(TankMods::FOREST_AT);
                    t.mods.sync_forest();
                    if team == Team::Player {
                        self.progress.tank_upgrade = t.mods.upgrade;
                    }
                }
            }
        }
    }

    /// Hit while ★>0: lose one star (and ship). Returns `true` if the tank survives.
    fn absorb_hit_or_strip_ship(&mut self, tank_id: u64) -> bool {
        let Some(tank) = self.tanks.iter_mut().find(|t| t.id == tank_id) else {
            return false;
        };
        let team = tank.team;
        // Any solid hit strips ship (permanent until next pickup).
        tank.mods.water_walk = false;
        if team == Team::Player {
            self.progress.ship = false;
        }

        if tank.mods.upgrade == 0 {
            return false;
        }
        tank.mods.upgrade -= 1;
        tank.mods.sync_forest();
        let upgrade = tank.mods.upgrade;
        if team == Team::Player {
            self.progress.tank_upgrade = upgrade;
        }
        self.refresh_tank_fire(tank_id);
        true
    }

    fn refresh_tank_fire(&mut self, tank_id: u64) {
        let Some(tank) = self.tanks.iter_mut().find(|t| t.id == tank_id) else {
            return;
        };
        let base = match tank.team {
            Team::Player => self.config.fire,
            Team::Enemy => self.config.enemy_fire,
        };
        let (interval, max_b) =
            fire_for_upgrade(base.fire_interval, base.max_bullets, tank.mods.upgrade);
        tank.fire.config.fire_interval = interval;
        tank.fire.config.max_bullets = max_b;
    }

    fn sync_tank_mod_flags(&mut self) {
        let active = self.bonuses.active.clone();
        let player_invuln = self.progress.is_invulnerable();
        let player_ship = self.progress.ship;
        let half = self.config.tank_half;
        let on_water: Vec<bool> = self
            .tanks
            .iter()
            .map(|t| Self::tank_overlaps_water(&self.map, t.x, t.y, half))
            .collect();

        for (tank, standing_in_water) in self.tanks.iter_mut().zip(on_water.iter()) {
            let mine: Vec<_> = active
                .iter()
                .filter(|a| a.tank_id == Some(tank.id))
                .collect();
            let invuln = mine.iter().any(|a| a.effects.contains(&EffectKind::Invuln))
                || (tank.team == Team::Player && player_invuln);
            let ship_bonus = (tank.team == Team::Player && player_ship)
                || mine
                    .iter()
                    .any(|a| a.effects.contains(&EffectKind::WaterWalk));
            // If ship was stripped while still on water, keep walk until ashore.
            let water = ship_bonus || *standing_in_water;
            tank.mods.invuln = invuln;
            tank.mods.water_walk = water;
            tank.mods.sync_forest();
        }
    }

    fn tank_overlaps_water(map: &Map, cx: f32, cy: f32, half: f32) -> bool {
        map.tiles_overlapping(cx, cy, half, half)
            .iter()
            .any(|(_, _, tile)| *tile == Tile::Water)
    }

    fn apply_fortify_walls(&mut self, duration: f32) {
        if self.bonuses.fortify_secs <= 0.0 {
            self.bonuses.fortify_backup.clear();
            let bases: Vec<(u32, u32)> = {
                let mut v = Vec::new();
                for y in 0..self.map.height() {
                    for x in 0..self.map.width() {
                        if self.map.get(x, y) == Some(Tile::Base) {
                            v.push((x, y));
                        }
                    }
                }
                v
            };
            for (bx, by) in bases {
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let x = bx as i32 + dx;
                        let y = by as i32 + dy;
                        if x < 0 || y < 0 {
                            continue;
                        }
                        let (x, y) = (x as u32, y as u32);
                        if x >= self.map.width() || y >= self.map.height() {
                            continue;
                        }
                        let Some(tile) = self.map.get(x, y) else {
                            continue;
                        };
                        if matches!(tile, Tile::Base | Tile::Water) {
                            continue;
                        }
                        self.bonuses
                            .fortify_backup
                            .push((x, y, tile.to_char() as u8));
                        let _ = self.map.set(x, y, Tile::Steel);
                    }
                }
            }
        }
        self.bonuses.fortify_secs = self.bonuses.fortify_secs.max(duration);
    }

    fn restore_fortify_walls(&mut self) {
        if self.bonuses.fortify_backup.is_empty() {
            return;
        }
        let backup = std::mem::take(&mut self.bonuses.fortify_backup);
        for (x, y, ch) in backup {
            if let Some(tile) = Tile::from_char(ch as char) {
                // Don't overwrite if something else already cleared the cell mid-fight.
                if self.map.get(x, y) == Some(Tile::Steel) {
                    let _ = self.map.set(x, y, tile);
                }
            }
        }
        self.bonuses.fortify_secs = 0.0;
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
    use crate::score::ScoreConfig;

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
                wave: WaveConfig {
                    total_enemies: 0,
                    max_on_field: 4,
                    spawn_interval: 0.0,
                },
                ..Default::default()
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

    #[test]
    fn wave_never_exceeds_max_on_field() {
        let mut world = World::new(
            Map::parse(
                "\
X.X.X.X
.......
...P...
",
            )
            .unwrap(),
            SimConfig {
                wave: WaveConfig {
                    total_enemies: 10,
                    max_on_field: 4,
                    spawn_interval: 0.0,
                },
                ..Default::default()
            },
        );

        let mut peak = 0u32;
        for _ in 0..30 {
            world.step(&SimInput::default(), 1.0 / 60.0);
            let n = world.enemies_on_field();
            peak = peak.max(n);
            assert!(n <= 4, "on_field={n} exceeded max");
        }
        assert_eq!(peak, 4);
        assert_eq!(world.wave.remaining_to_spawn, 6);
    }

    #[test]
    fn killing_enemy_frees_slot_for_next_spawn() {
        let mut world = World::new(
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
                    total_enemies: 3,
                    max_on_field: 1,
                    spawn_interval: 0.0,
                },
                ..Default::default()
            },
        );

        // Spawn first enemy.
        world.step(&SimInput::default(), 1.0 / 60.0);
        assert_eq!(world.enemies_on_field(), 1);
        assert_eq!(world.wave.remaining_to_spawn, 2);

        let enemy_id = world
            .tanks
            .iter()
            .find(|t| t.team == Team::Enemy)
            .unwrap()
            .id;
        {
            let p = world.player_mut().unwrap();
            p.x = 0.5;
            p.y = 2.5;
            p.dir = Dir::Up;
            p.fire.cooldown = 0.0;
        }

        let mut killed = false;
        for _ in 0..40 {
            world.step(
                &SimInput {
                    fire_held: true,
                    ..Default::default()
                },
                1.0 / 60.0,
            );
            if world.tanks.iter().all(|t| t.id != enemy_id) {
                killed = true;
                break;
            }
        }
        assert!(killed, "first enemy should die");
        assert_eq!(world.wave.destroyed, 1);
        // Same tick as the kill: spawn runs after combat and fills the freed slot.
        assert_eq!(world.enemies_on_field(), 1);
        assert_eq!(world.wave.remaining_to_spawn, 1);

        // Drop leftover player bullets so they don't wipe the replacement.
        world.bullets.clear();
        let before_spawn_queue = world.wave.remaining_to_spawn;
        for _ in 0..10 {
            world.step(&SimInput::default(), 1.0 / 60.0);
            assert!(
                world.enemies_on_field() <= 1,
                "max_on_field must stay 1"
            );
        }
        // Queue only shrinks when a spawn happens after a later kill; with 1 alive it stays.
        assert_eq!(world.wave.remaining_to_spawn, before_spawn_queue);
        assert_eq!(world.enemies_on_field(), 1);
    }

    #[test]
    fn tanks_cannot_overlap() {
        let mut world = tiny_world(
            "\
...
.X.
.P.
",
        );
        world.config.wave = WaveConfig {
            total_enemies: 1,
            max_on_field: 1,
            spawn_interval: 0.0,
        };
        world.wave = crate::wave::WaveState::new(world.config.wave);
        world.step(&SimInput::default(), 1.0 / 60.0);
        assert_eq!(world.enemies_on_field(), 1);

        let enemy_y = world
            .tanks
            .iter()
            .find(|t| t.team == Team::Enemy)
            .unwrap()
            .y;
        {
            let p = world.player_mut().unwrap();
            p.x = 1.5;
            p.y = 2.5;
        }
        for _ in 0..120 {
            world.step(
                &SimInput {
                    move_dir: Some(Dir::Up),
                    ..Default::default()
                },
                1.0 / 60.0,
            );
        }
        let player = world.player().unwrap();
        let gap = (player.y - enemy_y).abs();
        assert!(
            gap >= world.config.tank_half * 2.0 - 0.05,
            "player must not overlap enemy (gap={gap})"
        );
    }

    #[test]
    fn kill_delays_next_spawn() {
        let mut world = World::new(
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
                    total_enemies: 2,
                    max_on_field: 1,
                    spawn_interval: 1.0,
                },
                ..Default::default()
            },
        );
        world.step(&SimInput::default(), 1.0 / 60.0);
        assert_eq!(world.enemies_on_field(), 1);
        let enemy_id = world
            .tanks
            .iter()
            .find(|t| t.team == Team::Enemy)
            .unwrap()
            .id;
        {
            let p = world.player_mut().unwrap();
            p.x = 0.5;
            p.y = 2.5;
            p.dir = Dir::Up;
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
            if world.tanks.iter().all(|t| t.id != enemy_id) {
                break;
            }
        }
        assert_eq!(world.wave.destroyed, 1);
        // Immediately after kill: cooldown blocks refill.
        assert_eq!(world.enemies_on_field(), 0);
        assert!(world.wave.spawn_cooldown > 0.5);

        world.bullets.clear();
        // Still empty before interval elapses.
        for _ in 0..30 {
            world.step(&SimInput::default(), 1.0 / 60.0);
        }
        assert_eq!(world.enemies_on_field(), 0);

        // After ~1s total wait, next enemy appears.
        for _ in 0..60 {
            world.step(&SimInput::default(), 1.0 / 60.0);
        }
        assert_eq!(world.enemies_on_field(), 1);
    }

    #[test]
    fn destroying_base_is_defeat() {
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
        assert_eq!(world.map.get(1, 2), Some(Tile::Base));
        for _ in 0..30 {
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
        assert_eq!(world.outcome, RoundOutcome::DefeatBaseDestroyed);
        assert_eq!(world.map.get(1, 2), Some(Tile::Empty));
        // Frozen after defeat.
        let tick = world.tick;
        world.step(&SimInput::default(), 1.0 / 60.0);
        assert_eq!(world.tick, tick);
    }

    #[test]
    fn clearing_wave_is_victory() {
        let mut world = World::new(
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
                    total_enemies: 1,
                    max_on_field: 1,
                    spawn_interval: 0.0,
                },
                ..Default::default()
            },
        );
        world.step(&SimInput::default(), 1.0 / 60.0);
        assert_eq!(world.enemies_on_field(), 1);
        {
            let p = world.player_mut().unwrap();
            p.x = 0.5;
            p.y = 2.5;
            p.dir = Dir::Up;
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
            if world.outcome != RoundOutcome::Playing {
                break;
            }
        }
        assert_eq!(world.outcome, RoundOutcome::Victory);
        assert_eq!(world.wave.destroyed, 1);
        assert!(world.wave.is_cleared(world.enemies_on_field()));
    }

    #[test]
    fn player_death_respawns_with_invuln_until_lives_out() {
        let mut world = World::new(
            Map::parse(
                "\
.....
..P..
.....
",
            )
            .unwrap(),
            SimConfig {
                lives: LivesConfig {
                    starting_lives: 2,
                    max_lives: 5,
                    life_every_points: 20_000,
                    invuln_secs: 2.0,
                },
                wave: WaveConfig {
                    total_enemies: 0,
                    max_on_field: 4,
                    spawn_interval: 0.0,
                },
                ..Default::default()
            },
        );
        assert_eq!(world.progress.lives, 2);
        let first_id = world.player_tank_id();

        // Simulate enemy bullet hit: spawn hostile bullet on player.
        let bid = world.alloc_id();
        let (px, py) = {
            let p = world.player().unwrap();
            (p.x, p.y)
        };
        world.bullets.push(Bullet {
            id: bid,
            owner_id: 999,
            team: Team::Enemy,
            x: px,
            y: py,
            dir: Dir::Down,
            speed: 6.0,
        });
        world.resolve_bullet_tank_hits();
        assert_eq!(world.progress.lives, 1);
        assert!(world.progress.is_invulnerable());
        assert_ne!(world.player_tank_id(), first_id);
        assert_eq!(world.outcome, RoundOutcome::Playing);

        // Second death while vulnerable → game over.
        world.progress.invuln_remaining = 0.0;
        let bid2 = world.alloc_id();
        let (px, py) = {
            let p = world.player().unwrap();
            (p.x, p.y)
        };
        world.bullets.push(Bullet {
            id: bid2,
            owner_id: 999,
            team: Team::Enemy,
            x: px,
            y: py,
            dir: Dir::Down,
            speed: 6.0,
        });
        world.resolve_bullet_tank_hits();
        assert_eq!(world.progress.lives, 0);
        assert_eq!(world.outcome, RoundOutcome::DefeatNoLives);
        assert!(world.player().is_none());
    }

    #[test]
    fn invulnerable_player_ignores_hits() {
        let mut world = tiny_world(
            "\
...
.P.
...
",
        );
        world.progress.start_invuln();
        let id = world.player_tank_id();
        world.bullets.push(Bullet {
            id: 50,
            owner_id: 999,
            team: Team::Enemy,
            x: world.player().unwrap().x,
            y: world.player().unwrap().y,
            dir: Dir::Up,
            speed: 6.0,
        });
        world.resolve_bullet_tank_hits();
        assert_eq!(world.player_tank_id(), id);
        assert_eq!(world.progress.lives, world.config.lives.starting_lives);
        assert!(world.bullets.is_empty());
    }

    #[test]
    fn killing_enemy_can_grant_life_at_threshold() {
        let mut world = World::new(
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
                    total_enemies: 1,
                    max_on_field: 1,
                    spawn_interval: 0.0,
                },
                lives: LivesConfig {
                    starting_lives: 3,
                    max_lives: 5,
                    life_every_points: 100,
                    invuln_secs: 1.0,
                },
                score: ScoreConfig {
                    enemy_basic: 100,
                    streak_bonus_per_step: 0,
                    clear_bonus: 0,
                    clear_no_death_bonus: 0,
                    ..Default::default()
                },
                enemy_fire: FireConfig {
                    fire_interval: 999.0,
                    max_bullets: 0,
                },
                ..Default::default()
            },
        );
        world.step(&SimInput::default(), 1.0 / 60.0);
        {
            let p = world.player_mut().unwrap();
            p.x = 0.5;
            p.y = 2.5;
            p.dir = Dir::Up;
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
            if world.outcome != RoundOutcome::Playing {
                break;
            }
        }
        assert_eq!(world.progress.score, 100);
        assert_eq!(world.progress.lives, 4);
    }

    #[test]
    fn victory_awards_clear_style_bonuses() {
        let mut world = World::new(
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
                    total_enemies: 1,
                    max_on_field: 1,
                    spawn_interval: 0.0,
                },
                score: ScoreConfig {
                    enemy_basic: 100,
                    streak_bonus_per_step: 0,
                    clear_bonus: 500,
                    clear_no_death_bonus: 1000,
                    ..Default::default()
                },
                enemy_fire: FireConfig {
                    fire_interval: 999.0,
                    max_bullets: 0,
                },
                ..Default::default()
            },
        );
        world.step(&SimInput::default(), 1.0 / 60.0);
        {
            let p = world.player_mut().unwrap();
            p.x = 0.5;
            p.y = 2.5;
            p.dir = Dir::Up;
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
            if world.outcome != RoundOutcome::Playing {
                break;
            }
        }
        assert_eq!(world.outcome, RoundOutcome::Victory);
        // kill 100 + clear 500 + no-death 1000
        assert_eq!(world.progress.score, 1600);
        assert_eq!(world.style.streak, 1);
        assert_eq!(world.style.deaths_this_level, 0);
    }

    #[test]
    fn opposing_bullets_cancel_each_other() {
        let mut world = tiny_world(
            "\
.....
..P..
.....
",
        );
        world.bullets.push(Bullet {
            id: 1,
            owner_id: world.player_tank_id(),
            team: Team::Player,
            x: 2.5,
            y: 2.0,
            dir: Dir::Up,
            speed: 6.0,
        });
        world.bullets.push(Bullet {
            id: 2,
            owner_id: 999,
            team: Team::Enemy,
            x: 2.5,
            y: 2.05,
            dir: Dir::Down,
            speed: 6.0,
        });
        world.resolve_bullet_bullet_hits();
        assert!(
            world.bullets.is_empty(),
            "player and enemy bullets must destroy each other"
        );
    }

    #[test]
    fn same_team_bullets_also_cancel() {
        let mut world = tiny_world(
            "\
.....
..P..
.....
",
        );
        world.bullets.push(Bullet {
            id: 1,
            owner_id: 1,
            team: Team::Enemy,
            x: 2.5,
            y: 2.0,
            dir: Dir::Down,
            speed: 6.0,
        });
        world.bullets.push(Bullet {
            id: 2,
            owner_id: 2,
            team: Team::Enemy,
            x: 2.5,
            y: 2.05,
            dir: Dir::Down,
            speed: 6.0,
        });
        world.resolve_bullet_bullet_hits();
        assert!(
            world.bullets.is_empty(),
            "ally/enemy same-team bullets must cancel too"
        );
    }
}
