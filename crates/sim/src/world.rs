//! Fixed-tick world: tanks, bullets, tile collisions.

use std::collections::VecDeque;

use crate::ai::{self, EnemyGoal, EnemyGoalInput, GoalTarget};
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

/// One-shot cues for the presentation layer (cleared at the start of each `step`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimSfx {
    Shot,
    Explosion,
}

fn tile_center(v: f32) -> f32 {
    (v - 0.5).round() + 0.5
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
    /// Classic flashing carrier: killing this tank drops a letter.
    pub carries_bonus: bool,
    /// Phase 4.1: current enemy goal (`None` for player / idle).
    pub ai_goal: Option<EnemyGoal>,
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
    /// Classic pause after the last enemy dies before Victory (pickups / base still live).
    /// `0` = resolve Victory on the clear tick (handy for unit tests).
    pub clear_grace_secs: f32,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            // ~8s to cross a 16-tile map — closer to classic BC feel than 4.0.
            tank_speed: 2.0,
            // Base (no stars); stars multiply via `bullet_speed_for_upgrade`.
            // Kept ~3× tank so shots stay readable after the speed cut.
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
            // Classic NES feels ~2–4s of free play after the last kill.
            clear_grace_secs: 3.0,
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
    /// Seconds left in post-clear grace (`0` = not in grace / already resolved).
    pub clear_grace_remaining: f32,
    /// Presentation SFX queued this tick (shot / explosion).
    pub sfx: Vec<SimSfx>,
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
            clear_grace_remaining: 0.0,
            sfx: Vec::new(),
            next_id: 1,
            player_tank_id: 0,
            next_enemy_spawn: 0,
        };
        world.spawn_player_tank();
        world
    }

    /// Wave cleared and still in the classic free-play window before Victory.
    pub fn in_clear_grace(&self) -> bool {
        self.outcome == RoundOutcome::Playing && self.clear_grace_remaining > 0.0
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
                armored: false,
            },
            carries_bonus: false,
            ai_goal: None,
        });
    }

    /// After loading carried `PlayerProgress`, rebuild the player tank so stars apply.
    pub fn reapply_player_from_progress(&mut self) {
        let pid = self.player_tank_id;
        self.tanks.retain(|t| t.id != pid);
        self.bullets.retain(|b| b.owner_id != pid);
        self.spawn_player_tank();
    }

    /// Readable enemy power for HUD (phase 3.2).
    pub fn enemy_power_labels(&self) -> Vec<String> {
        let mut out = Vec::new();
        for t in self.tanks.iter().filter(|t| t.team == Team::Enemy) {
            let mut parts = Vec::new();
            if t.carries_bonus {
                parts.push("DROP".into());
            }
            if t.mods.upgrade > 0 {
                parts.push(format!("★{}", t.mods.upgrade));
            }
            if t.mods.armored {
                parts.push("H".into());
            }
            if t.mods.invuln {
                parts.push("H!".into());
            }
            if t.mods.water_walk {
                parts.push("B".into());
            }
            let timed = self.bonuses.hud_active_labels(Team::Enemy, Some(t.id));
            for label in timed {
                if let Some(c) = label.chars().next() {
                    let s = c.to_string();
                    if !parts.iter().any(|p| p.starts_with(&s)) {
                        parts.push(s);
                    }
                }
            }
            if !parts.is_empty() {
                out.push(format!("E{}:{}", t.id, parts.join("")));
            }
        }
        if self.bonuses.freeze_player_secs > 0.0 {
            out.push(format!(
                "frozen {:.0}s (can shoot)",
                self.bonuses.freeze_player_secs.ceil()
            ));
        }
        out
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

        self.sfx.clear();
        self.progress.tick_invuln(dt);
        let fortify_was = self.bonuses.fortify_secs > 0.0;
        self.bonuses.tick_timers(dt);
        if fortify_was && self.bonuses.fortify_secs <= 0.0 {
            self.restore_fortify_walls();
            self.eject_blocked_tanks();
        }
        self.sync_tank_mod_flags();

        let player_frozen = self.bonuses.opponents_frozen(Team::Player);
        if let Some(player) = self.player_mut() {
            // Frozen: can shoot & turn in place? Classic: shoot yes, move no.
            // Keep facing from last move; still accept fire / autofire.
            if player_frozen {
                player.move_dir = None;
            } else {
                player.move_dir = input.move_dir;
                if let Some(dir) = input.move_dir {
                    player.dir = dir;
                }
            }
            player.fire.set_fire_held(input.fire_held);
            if input.toggle_autofire {
                player.fire.toggle_autofire();
            }
        }

        // Phase 4.1: goal priorities (base / player / bonus / cover / bypass).
        self.update_enemy_ai();

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
            self.clear_grace_remaining = 0.0;
            self.outcome = RoundOutcome::DefeatBaseDestroyed;
        } else if self.outcome == RoundOutcome::Playing
            && self.wave.is_cleared(self.enemies_on_field())
        {
            self.tick_clear_grace(dt);
        }

        self.tick += 1;
    }

    /// After the last enemy dies: short free-play window (pick up letters / risk the base),
    /// then award clear score and set Victory.
    fn tick_clear_grace(&mut self, dt: f32) {
        if self.clear_grace_remaining <= 0.0 {
            let grace = self.config.clear_grace_secs.max(0.0);
            if grace <= 0.0 {
                self.finalize_victory();
                return;
            }
            self.clear_grace_remaining = grace;
            return;
        }
        self.clear_grace_remaining = (self.clear_grace_remaining - dt).max(0.0);
        if self.clear_grace_remaining <= 0.0 {
            self.finalize_victory();
        }
    }

    fn finalize_victory(&mut self) {
        let (bonus, _) = self.style.award_clear(&self.config.score);
        self.progress.add_score(bonus);
        self.bonuses.cancel_on_level_end();
        self.restore_fortify_walls();
        self.clear_grace_remaining = 0.0;
        self.outcome = RoundOutcome::Victory;
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
            self.bonuses.spawns_since_carrier =
                self.bonuses.spawns_since_carrier.saturating_add(1);
            let carries = self.bonuses.spawns_since_carrier
                >= self.bonuses.catalog.drop_every_kills.max(1);
            if carries {
                self.bonuses.spawns_since_carrier = 0;
            }
            self.tanks.push(Tank {
                id,
                team: Team::Enemy,
                enemy_kind: EnemyKind::Basic,
                x: cx,
                y: cy,
                dir: Dir::Down,
                fire,
                move_dir: None,
                mods: TankMods::default(),
                carries_bonus: carries,
                ai_goal: None,
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
        // Classic: only a player bullet can cancel an enemy bullet.
        // Enemy↔enemy (and player↔player) pass through each other.
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
                let opposing = matches!(
                    (a.team, b.team),
                    (Team::Player, Team::Enemy) | (Team::Enemy, Team::Player)
                );
                if !opposing {
                    continue;
                }
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
                let carries = self.tanks[pos].carries_bonus;
                let (dx, dy) = (self.tanks[pos].x, self.tanks[pos].y);
                self.bonuses.cancel_on_death(team, tid);
                self.bonuses.clear_enemy_clock_if(tid);
                self.tanks.swap_remove(pos);
                self.bullets.retain(|b| b.owner_id != tid);
                self.sfx.push(SimSfx::Explosion);
                match team {
                    Team::Enemy => {
                        self.wave.on_destroyed();
                        let (pts, _) = self.style.award_kill(&self.config.score, kind);
                        self.progress.add_score(pts);
                        if carries {
                            drop_positions.push((dx, dy));
                        }
                    }
                    Team::Player => {
                        player_died = true;
                    }
                }
            }
        }

        for (x, y) in drop_positions {
            self.spawn_catalog_pickup(x, y);
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
            // Classic corridor assist: align the cross-axis to a tile center before driving.
            self.align_tank_to_grid(i, dir, dt);
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

    /// Snap (or softly nudge) the axis perpendicular to `dir` toward the nearest tile center.
    fn align_tank_to_grid(&mut self, i: usize, dir: Dir, dt: f32) {
        let id = self.tanks[i].id;
        let half = self.config.tank_half;
        let water_walk = self.tanks[i].mods.water_walk;
        let x = self.tanks[i].x;
        let y = self.tanks[i].y;
        let (tx, ty) = match dir {
            Dir::Left | Dir::Right => (x, tile_center(y)),
            Dir::Up | Dir::Down => (tile_center(x), y),
        };
        if (tx - x).abs() < 1e-4 && (ty - y).abs() < 1e-4 {
            return;
        }
        if self.can_tank_move(id, tx, ty, half, water_walk) {
            self.tanks[i].x = tx;
            self.tanks[i].y = ty;
            return;
        }
        // Blocked hard-snap (tank overlap / corner) — light settle toward the center.
        let settle = self.config.tank_speed * dt * 1.25;
        let nx = x + (tx - x).clamp(-settle, settle);
        let ny = y + (ty - y).clamp(-settle, settle);
        if self.can_tank_move(id, nx, ny, half, water_walk) {
            self.tanks[i].x = nx;
            self.tanks[i].y = ny;
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

        let mut shots: Vec<(u64, Team, f32, f32, Dir, f32)> = Vec::new();
        for (tank, &live) in self.tanks.iter_mut().zip(live_counts.iter()) {
            // Player may shoot while frozen; only enemies lose fire when clocked.
            let fire_locked = tank.team == Team::Enemy && enemies_frozen;
            tank.fire.tick(dt);
            if fire_locked {
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
            self.sfx.push(SimSfx::Shot);
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

    /// Drop a weighted-random catalog letter (carrier kill). Position is randomized on the map.
    fn spawn_catalog_pickup(&mut self, near_x: f32, near_y: f32) {
        if self.bonuses.catalog.defs.is_empty() {
            return;
        }
        let roll = self
            .tick
            .wrapping_mul(1_664_525)
            .wrapping_add(self.next_id)
            .wrapping_add(self.bonuses.drop_salt)
            .wrapping_add((near_x * 13.0 + near_y * 29.0) as u64);
        let Some(def) = self.bonuses.catalog.pick_weighted(roll).cloned() else {
            return;
        };
        self.bonuses.drop_salt = self.bonuses.drop_salt.wrapping_add(1);
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
        // Enemy H/C are handled as permanent rules — no timed ActiveBonus (no god-mode).
        let skip_active = team == Team::Enemy
            && def.effects.iter().all(|e| {
                matches!(
                    e,
                    EffectKind::Invuln | EffectKind::FreezeOpponents
                )
            });
        if (timed || sticky) && !skip_active {
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
                    if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                        t.mods.invuln = true;
                    }
                } else {
                    // Enemy H: permanent armor (multi-hit via ★), no god-mode timer.
                    if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                        t.mods.armored = true;
                        t.mods.upgrade =
                            (t.mods.upgrade + 2).min(TankMods::MAX_UPGRADE).max(2);
                        t.mods.sync_forest();
                    }
                    self.refresh_tank_fire(tank_id);
                }
            }
            EffectKind::FreezeOpponents => {
                match team {
                    Team::Player => {
                        self.bonuses.freeze_enemy_secs =
                            self.bonuses.freeze_enemy_secs.max(def.duration_secs);
                    }
                    Team::Enemy => {
                        // Timed freeze: player can shoot, cannot move.
                        self.bonuses.freeze_player_by_enemy = None;
                        self.bonuses.freeze_player_secs =
                            self.bonuses.freeze_player_secs.max(def.duration_secs);
                    }
                }
            }
            EffectKind::FortifyBase => {
                if team == Team::Player {
                    self.apply_fortify_walls(def.duration_secs);
                    self.eject_blocked_tanks();
                } else {
                    self.apply_weaken_base();
                    self.eject_blocked_tanks();
                }
            }
            EffectKind::KillOpponents => {
                let victims: Vec<u64> = self
                    .tanks
                    .iter()
                    .filter(|t| t.team != team)
                    .map(|t| t.id)
                    .collect();
                let mut drops: Vec<(f32, f32)> = Vec::new();
                for vid in victims {
                    if let Some(pos) = self.tanks.iter().position(|t| t.id == vid) {
                        let vteam = self.tanks[pos].team;
                        let kind = self.tanks[pos].enemy_kind;
                        let carries = self.tanks[pos].carries_bonus;
                        let (dx, dy) = (self.tanks[pos].x, self.tanks[pos].y);
                        self.bonuses.cancel_on_death(vteam, vid);
                        self.tanks.swap_remove(pos);
                        self.bullets.retain(|b| b.owner_id != vid);
                        self.sfx.push(SimSfx::Explosion);
                        if vteam == Team::Enemy {
                            self.bonuses.clear_enemy_clock_if(vid);
                            self.wave.on_destroyed();
                            if team == Team::Player {
                                let (pts, _) = self.style.award_kill(&self.config.score, kind);
                                self.progress.add_score(pts);
                            }
                            if carries {
                                drops.push((dx, dy));
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
                for (x, y) in drops {
                    self.spawn_catalog_pickup(x, y);
                }
            }
            EffectKind::TankUpgrade => {
                if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                    let cap = if def.max_stacks == 0 {
                        TankMods::MAX_UPGRADE
                    } else {
                        def.max_stacks.min(u32::from(TankMods::MAX_UPGRADE)) as u8
                    };
                    let next = (t.mods.upgrade + 1).min(cap);
                    // Enemy star: from the first pickup they can break steel (броня).
                    t.mods.upgrade = if team == Team::Enemy {
                        next.max(TankMods::MAX_FIRE_UPGRADE)
                    } else {
                        next
                    };
                    t.mods.sync_forest();
                    if team == Team::Player {
                        self.progress.tank_upgrade = t.mods.upgrade;
                    }
                    self.refresh_tank_fire(tank_id);
                }
            }
            EffectKind::MaxUpgrade => {
                if team == Team::Enemy {
                    // Enemy gun: ALL enemies become armored + break steel.
                    let ids: Vec<u64> = self
                        .tanks
                        .iter()
                        .filter(|t| t.team == Team::Enemy)
                        .map(|t| t.id)
                        .collect();
                    for eid in ids {
                        if let Some(t) = self.tanks.iter_mut().find(|t| t.id == eid) {
                            t.mods.armored = true;
                            t.mods.upgrade =
                                t.mods.upgrade.max(TankMods::MAX_FIRE_UPGRADE);
                            t.mods.sync_forest();
                        }
                        self.refresh_tank_fire(eid);
                    }
                } else if let Some(t) = self.tanks.iter_mut().find(|t| t.id == tank_id) {
                    // Player: 1st gun → ★3; 2nd → ★4 forest.
                    if t.mods.upgrade < TankMods::MAX_FIRE_UPGRADE {
                        t.mods.upgrade = TankMods::MAX_FIRE_UPGRADE;
                    } else {
                        t.mods.upgrade = TankMods::MAX_UPGRADE;
                    }
                    t.mods.sync_forest();
                    self.progress.tank_upgrade = t.mods.upgrade;
                    self.refresh_tank_fire(tank_id);
                }
            }
            EffectKind::ExtraLife => {
                if team == Team::Player {
                    let _ = self.progress.try_add_life();
                } else {
                    // Tank 1990: enemy "life" → all on-field enemies gain armor (green durability).
                    self.armour_all_enemies();
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
            // Enemies never get timed god-mode; H → armored + ★ instead.
            let invuln = match tank.team {
                Team::Player => {
                    mine.iter().any(|a| a.effects.contains(&EffectKind::Invuln))
                        || player_invuln
                }
                Team::Enemy => false,
            };
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

    /// Phase 4.1: assign goals and drive movement (BFS seek still used for pathing).
    fn update_enemy_ai(&mut self) {
        for t in &mut self.tanks {
            if t.team == Team::Enemy {
                t.move_dir = None;
                t.ai_goal = None;
            }
        }
        if self.bonuses.opponents_frozen(Team::Enemy) {
            return;
        }

        let bonus_targets: Vec<(f32, f32)> = self
            .bonuses
            .pickups
            .iter()
            .filter_map(|p| {
                let def = self.bonuses.catalog.get(&p.bonus_id)?;
                if def.who.allows(Team::Enemy) {
                    Some((p.x, p.y))
                } else {
                    None
                }
            })
            .collect();

        let bonus_seeker: Option<(u64, f32, f32)> = {
            let mut best: Option<(u64, f32, f32, f32)> = None;
            for t in self.tanks.iter().filter(|t| t.team == Team::Enemy) {
                for &(tx, ty) in &bonus_targets {
                    let dist = (tx - t.x).hypot(ty - t.y);
                    let better = match best {
                        None => true,
                        Some((_, _, _, d)) => dist < d,
                    };
                    if better {
                        best = Some((t.id, tx, ty, dist));
                    }
                }
            }
            best.map(|(id, x, y, _)| (id, x, y))
        };

        let base = self.base_center();
        let player = self.player().map(|p| (p.x, p.y, p.dir));
        let enemy_ids: Vec<u64> = self
            .tanks
            .iter()
            .filter(|t| t.team == Team::Enemy)
            .map(|t| t.id)
            .collect();

        for (slot, eid) in enemy_ids.into_iter().enumerate() {
            let Some((ex, ey, water_walk)) = self
                .tanks
                .iter()
                .find(|t| t.id == eid)
                .map(|t| (t.x, t.y, t.mods.water_walk))
            else {
                continue;
            };

            let assigned_bonus = bonus_seeker
                .filter(|(id, _, _)| *id == eid)
                .map(|(_, x, y)| (x, y));

            let prefer_player = (slot as u32) % 3 == 0;
            let primary_xy = if prefer_player {
                player
                    .map(|(x, y, _)| (x, y))
                    .or(base)
            } else {
                base.or_else(|| player.map(|(x, y, _)| (x, y)))
            };

            let primary_blocked = match primary_xy {
                Some((tx, ty)) => !self.has_grid_path(ex, ey, tx, ty, water_walk),
                None => false,
            };

            let under_threat = self.enemy_under_threat(eid, ex, ey, player);
            let cover = if under_threat {
                self.nearest_cover(ex, ey, water_walk)
            } else {
                None
            };

            let target = ai::pick_enemy_goal(EnemyGoalInput {
                slot: slot as u32,
                assigned_bonus,
                base,
                player: player.map(|(x, y, _)| (x, y)),
                cover,
                primary_blocked,
                under_threat,
            });

            let Some(GoalTarget {
                goal,
                x: mut tx,
                y: mut ty,
            }) = target
            else {
                continue;
            };

            // Base tile blocks tanks — path to a neighbouring approach cell.
            if goal == EnemyGoal::Base {
                if let Some(a) = self.approach_cell(tx, ty, ex, ey, water_walk) {
                    tx = a.0;
                    ty = a.1;
                }
            }
            if goal == EnemyGoal::Bypass {
                let (cx, cy) = self.clamp_aim(tx, ty);
                tx = cx;
                ty = cy;
            }

            let dir = self.seek_dir(eid, ex, ey, tx, ty, water_walk);
            if let Some(tank) = self.tanks.iter_mut().find(|t| t.id == eid) {
                tank.ai_goal = Some(goal);
                tank.move_dir = dir;
                if let Some(d) = dir {
                    tank.dir = d;
                }
            }
        }
    }

    fn clamp_aim(&self, x: f32, y: f32) -> (f32, f32) {
        let max_x = (self.map.width() as f32 - 0.5).max(0.5);
        let max_y = (self.map.height() as f32 - 0.5).max(0.5);
        (x.clamp(0.5, max_x), y.clamp(0.5, max_y))
    }

    /// Walkable cell next to a blocked goal (HQ), preferring the side closer to the enemy.
    fn approach_cell(
        &self,
        goal_x: f32,
        goal_y: f32,
        from_x: f32,
        from_y: f32,
        water_walk: bool,
    ) -> Option<(f32, f32)> {
        let gx = goal_x.floor() as i32;
        let gy = goal_y.floor() as i32;
        let half = self.config.tank_half;
        let mut best: Option<(f32, f32, f32)> = None;
        for (dx, dy) in [(0i32, -1), (0, 1), (-1, 0), (1, 0)] {
            let nx = gx + dx;
            let ny = gy + dy;
            if !self.map.in_bounds(nx, ny) {
                continue;
            }
            let cx = nx as f32 + 0.5;
            let cy = ny as f32 + 0.5;
            if !Self::tank_fits(&self.map, cx, cy, half, water_walk) {
                continue;
            }
            let dist = (cx - from_x).hypot(cy - from_y);
            let better = match best {
                None => true,
                Some((_, _, d)) => dist < d,
            };
            if better {
                best = Some((cx, cy, dist));
            }
        }
        best.map(|(x, y, _)| (x, y))
    }

    fn base_center(&self) -> Option<(f32, f32)> {
        for y in 0..self.map.height() {
            for x in 0..self.map.width() {
                if self.map.get(x, y) == Some(Tile::Base) {
                    return Some((x as f32 + 0.5, y as f32 + 0.5));
                }
            }
        }
        None
    }

    /// Nearest brick cell the tank can stand beside (simple cover for 4.1).
    fn nearest_cover(&self, from_x: f32, from_y: f32, water_walk: bool) -> Option<(f32, f32)> {
        let mut best: Option<(f32, f32, f32)> = None;
        let half = self.config.tank_half;
        for y in 0..self.map.height() {
            for x in 0..self.map.width() {
                if self.map.get(x, y) != Some(Tile::Brick) {
                    continue;
                }
                // Stand on an adjacent empty cell, not inside the brick.
                for (dx, dy) in [(0i32, -1), (0, 1), (-1, 0), (1, 0)] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if !self.map.in_bounds(nx, ny) {
                        continue;
                    }
                    let cx = nx as f32 + 0.5;
                    let cy = ny as f32 + 0.5;
                    if !Self::tank_fits(&self.map, cx, cy, half, water_walk) {
                        continue;
                    }
                    if !self.has_grid_path(from_x, from_y, cx, cy, water_walk) {
                        continue;
                    }
                    let dist = (cx - from_x).hypot(cy - from_y);
                    let better = match best {
                        None => true,
                        Some((_, _, d)) => dist < d,
                    };
                    if better {
                        best = Some((cx, cy, dist));
                    }
                }
            }
        }
        best.map(|(x, y, _)| (x, y))
    }

    fn enemy_under_threat(
        &self,
        enemy_id: u64,
        ex: f32,
        ey: f32,
        player: Option<(f32, f32, Dir)>,
    ) -> bool {
        for b in &self.bullets {
            if b.team != Team::Player {
                continue;
            }
            if (b.x - ex).hypot(b.y - ey) < 4.0 {
                return true;
            }
        }
        let Some((px, py, pdir)) = player else {
            return false;
        };
        let dist = (px - ex).hypot(py - ey);
        if dist > 8.0 {
            return false;
        }
        let _ = enemy_id;
        ai::aims_at(pdir, px, py, ex, ey)
    }

    /// Grid BFS reachability (same walk rules as `path_waypoint`).
    fn has_grid_path(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        water_walk: bool,
    ) -> bool {
        let sx = from_x.floor() as i32;
        let sy = from_y.floor() as i32;
        let gx = to_x.floor() as i32;
        let gy = to_y.floor() as i32;
        if sx == gx && sy == gy {
            return true;
        }
        let w = self.map.width() as i32;
        let h = self.map.height() as i32;
        if !(0..w).contains(&sx)
            || !(0..h).contains(&sy)
            || !(0..w).contains(&gx)
            || !(0..h).contains(&gy)
        {
            return false;
        }
        let walkable = |x: i32, y: i32| -> bool {
            if x < 0 || y < 0 || x >= w || y >= h {
                return false;
            }
            match self.map.get(x as u32, y as u32) {
                Some(Tile::Water) => water_walk,
                Some(t) => !t.blocks_tank(),
                None => false,
            }
        };
        // Goal cell may be the base (blocked) or a tank — path to a neighbour is enough.
        let goal_ok = |x: i32, y: i32| -> bool {
            if x == gx && y == gy {
                return true;
            }
            false
        };
        let mut goal_cells: Vec<(i32, i32)> = vec![(gx, gy)];
        if !walkable(gx, gy) {
            goal_cells.clear();
            for (dx, dy) in [(0i32, -1), (0, 1), (-1, 0), (1, 0)] {
                let nx = gx + dx;
                let ny = gy + dy;
                if walkable(nx, ny) {
                    goal_cells.push((nx, ny));
                }
            }
            if goal_cells.is_empty() {
                return false;
            }
        }

        let idx = |x: i32, y: i32| -> usize { (y * w + x) as usize };
        let mut seen = vec![false; (w * h) as usize];
        let mut q = VecDeque::new();
        q.push_back((sx, sy));
        seen[idx(sx, sy)] = true;
        const STEPS: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
        while let Some((x, y)) = q.pop_front() {
            if goal_cells.iter().any(|&(cx, cy)| cx == x && cy == y) || goal_ok(x, y) {
                return true;
            }
            for &(dx, dy) in &STEPS {
                let nx = x + dx;
                let ny = y + dy;
                if !walkable(nx, ny) {
                    continue;
                }
                let i = idx(nx, ny);
                if seen[i] {
                    continue;
                }
                seen[i] = true;
                q.push_back((nx, ny));
            }
        }
        false
    }

    fn dir_toward(from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> Option<Dir> {
        ai::facing_toward(from_x, from_y, to_x, to_y)
    }

    /// BFS waypoint + collision probe so seekers clear corners instead of wedging into walls.
    fn seek_dir(
        &self,
        tank_id: u64,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        water_walk: bool,
    ) -> Option<Dir> {
        let (wx, wy) = self
            .path_waypoint(from_x, from_y, to_x, to_y, water_walk)
            .unwrap_or((to_x, to_y));
        let half = self.config.tank_half;
        let probe = half.max(0.2);
        let mut candidates: Vec<Dir> = Vec::with_capacity(4);
        if let Some(d) = Self::dir_toward(from_x, from_y, wx, wy) {
            candidates.push(d);
        }
        // Secondary axis toward the waypoint (clears corners when primary is blocked).
        let dx = wx - from_x;
        let dy = wy - from_y;
        if dx.abs() >= 0.05 && dy.abs() >= 0.05 {
            let secondary = if dx.abs() >= dy.abs() {
                if dy > 0.0 {
                    Dir::Down
                } else {
                    Dir::Up
                }
            } else if dx > 0.0 {
                Dir::Right
            } else {
                Dir::Left
            };
            if !candidates.contains(&secondary) {
                candidates.push(secondary);
            }
        }
        for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
            if !candidates.contains(&dir) {
                candidates.push(dir);
            }
        }
        for dir in candidates {
            let (ddx, ddy) = dir.delta();
            let nx = from_x + ddx * probe;
            let ny = from_y + ddy * probe;
            if self.can_tank_move(tank_id, nx, ny, half, water_walk) {
                return Some(dir);
            }
        }
        None
    }

    /// Center of the next BFS cell toward the pickup (or the pickup itself if same cell).
    fn path_waypoint(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        water_walk: bool,
    ) -> Option<(f32, f32)> {
        let sx = from_x.floor() as i32;
        let sy = from_y.floor() as i32;
        let gx = to_x.floor() as i32;
        let gy = to_y.floor() as i32;
        if sx == gx && sy == gy {
            return Some((to_x, to_y));
        }

        let w = self.map.width() as i32;
        let h = self.map.height() as i32;
        if !(0..w).contains(&sx)
            || !(0..h).contains(&sy)
            || !(0..w).contains(&gx)
            || !(0..h).contains(&gy)
        {
            return Some((to_x, to_y));
        }

        let walkable = |x: i32, y: i32| -> bool {
            if x < 0 || y < 0 || x >= w || y >= h {
                return false;
            }
            match self.map.get(x as u32, y as u32) {
                Some(Tile::Water) => water_walk,
                Some(t) => !t.blocks_tank(),
                None => false,
            }
        };
        if !walkable(gx, gy) {
            return Some((to_x, to_y));
        }

        let idx = |x: i32, y: i32| -> usize { (y * w + x) as usize };
        let mut came = vec![None::<(i32, i32, Dir)>; (w * h) as usize];
        let mut q = VecDeque::new();
        q.push_back((sx, sy));
        came[idx(sx, sy)] = Some((-1, -1, Dir::Up));

        const STEPS: [(Dir, i32, i32); 4] = [
            (Dir::Up, 0, -1),
            (Dir::Down, 0, 1),
            (Dir::Left, -1, 0),
            (Dir::Right, 1, 0),
        ];

        let mut found = false;
        while let Some((x, y)) = q.pop_front() {
            if x == gx && y == gy {
                found = true;
                break;
            }
            for &(dir, dx, dy) in &STEPS {
                let nx = x + dx;
                let ny = y + dy;
                if !walkable(nx, ny) {
                    continue;
                }
                let i = idx(nx, ny);
                if came[i].is_some() {
                    continue;
                }
                came[i] = Some((x, y, dir));
                q.push_back((nx, ny));
            }
        }
        if !found {
            return Some((to_x, to_y));
        }

        let mut cx = gx;
        let mut cy = gy;
        let mut first_dir = None;
        while !(cx == sx && cy == sy) {
            let Some((px, py, dir)) = came[idx(cx, cy)] else {
                break;
            };
            if px < 0 {
                break;
            }
            first_dir = Some(dir);
            if px == sx && py == sy {
                break;
            }
            cx = px;
            cy = py;
        }
        let dir = first_dir?;
        let (dx, dy) = match dir {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        };
        Some(((sx + dx) as f32 + 0.5, (sy + dy) as f32 + 0.5))
    }

    /// Tank 1990 life letter for enemies: every on-field enemy gains armor / multi-hit.
    fn armour_all_enemies(&mut self) {
        let ids: Vec<u64> = self
            .tanks
            .iter()
            .filter(|t| t.team == Team::Enemy)
            .map(|t| t.id)
            .collect();
        for eid in ids {
            if let Some(t) = self.tanks.iter_mut().find(|t| t.id == eid) {
                t.mods.armored = true;
                t.mods.upgrade = (t.mods.upgrade + 2).min(TankMods::MAX_UPGRADE).max(2);
                t.mods.sync_forest();
            }
            self.refresh_tank_fire(eid);
        }
    }

    /// If fortify/weaken placed walls under a tank, shove it to the nearest free cell.
    fn eject_blocked_tanks(&mut self) {
        let half = self.config.tank_half;
        let w = self.map.width();
        let h = self.map.height();
        let ids: Vec<u64> = self.tanks.iter().map(|t| t.id).collect();
        for id in ids {
            let Some((x, y, water_walk)) = self
                .tanks
                .iter()
                .find(|t| t.id == id)
                .map(|t| (t.x, t.y, t.mods.water_walk))
            else {
                continue;
            };
            if Self::tank_fits(&self.map, x, y, half, water_walk) {
                continue;
            }
            let mut best: Option<(f32, f32, f32)> = None; // dist, nx, ny
            for ty in 0..h {
                for tx in 0..w {
                    let nx = tx as f32 + 0.5;
                    let ny = ty as f32 + 0.5;
                    if !Self::tank_fits(&self.map, nx, ny, half, water_walk) {
                        continue;
                    }
                    if self.tank_occupies(nx, ny, half, Some(id)) {
                        continue;
                    }
                    let dist = (nx - x).hypot(ny - y);
                    let better = match best {
                        None => true,
                        Some((d, _, _)) => dist < d,
                    };
                    if better {
                        best = Some((dist, nx, ny));
                    }
                }
            }
            if let Some((_, nx, ny)) = best {
                if let Some(t) = self.tanks.iter_mut().find(|t| t.id == id) {
                    t.x = nx;
                    t.y = ny;
                }
            }
        }
    }

    /// Enemy shovel: strip player fortify and turn base-ring steel/brick into brick/empty weak walls.
    fn apply_weaken_base(&mut self) {
        if self.bonuses.fortify_secs > 0.0 {
            self.restore_fortify_walls();
        }
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
                    match self.map.get(x, y) {
                        Some(Tile::Steel) => {
                            let _ = self.map.set(x, y, Tile::Brick);
                        }
                        Some(Tile::Brick) => {
                            let _ = self.map.set(x, y, Tile::Empty);
                        }
                        _ => {}
                    }
                }
            }
        }
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
        let enemy = world
            .tanks
            .iter()
            .find(|t| t.team == Team::Enemy)
            .unwrap();
        let gap_x = (player.x - enemy.x).abs();
        let gap_y = (player.y - enemy.y).abs();
        let min_sep = world.config.tank_half * 2.0 - 0.05;
        assert!(
            gap_x >= min_sep || gap_y >= min_sep,
            "player must not overlap enemy (gap_x={gap_x}, gap_y={gap_y})"
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
                clear_grace_secs: 0.0,
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
    fn clear_grace_allows_pickup_before_victory() {
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
                clear_grace_secs: 0.5,
                bonuses: BonusCatalog::parse(include_str!("../../../data/bonuses/catalog.cfg"))
                    .unwrap(),
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
            if world.in_clear_grace() || world.outcome != RoundOutcome::Playing {
                break;
            }
        }
        assert!(world.in_clear_grace(), "must enter post-clear grace");
        assert_eq!(world.outcome, RoundOutcome::Playing);
        let lives = world.progress.lives;
        let (px, py) = {
            let p = world.player().unwrap();
            (p.x, p.y)
        };
        world.spawn_pickup_at("tank", px, py).unwrap();
        world.step(&SimInput::default(), 1.0 / 60.0);
        assert_eq!(world.progress.lives, lives + 1, "pickup during grace");
        assert_eq!(world.outcome, RoundOutcome::Playing);
        // Burn remaining grace.
        for _ in 0..60 {
            world.step(&SimInput::default(), 1.0 / 60.0);
            if world.outcome != RoundOutcome::Playing {
                break;
            }
        }
        assert_eq!(world.outcome, RoundOutcome::Victory);
    }

    #[test]
    fn clear_grace_base_hit_is_defeat() {
        let mut world = World::new(
            Map::parse(
                "\
X......
.......
...P...
...E...
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
                clear_grace_secs: 2.0,
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
            if world.in_clear_grace() {
                break;
            }
        }
        assert!(world.in_clear_grace());
        // Shoot the base during grace.
        {
            let p = world.player_mut().unwrap();
            p.x = 3.5;
            p.y = 2.5;
            p.dir = Dir::Down;
            p.fire.cooldown = 0.0;
        }
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
                clear_grace_secs: 0.0,
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
                clear_grace_secs: 0.0,
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
    fn enemy_bullets_do_not_cancel_each_other() {
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
        assert_eq!(
            world.bullets.len(),
            2,
            "enemy bullets must pass through each other"
        );
    }

    #[test]
    fn grid_settle_snaps_cross_axis_to_tile_center() {
        let mut world = tiny_world(
            "\
....
....
.P..
....
",
        );
        {
            let p = world.player_mut().unwrap();
            p.x = 1.5;
            p.y = 2.2; // off-center on Y; moving horizontally should snap Y → 2.5
        }
        world.step(
            &SimInput {
                move_dir: Some(Dir::Right),
                ..Default::default()
            },
            1.0 / 60.0,
        );
        let p = world.player().unwrap();
        assert!(
            (p.y - 2.5).abs() < 1e-3,
            "expected Y snapped to tile center, got {}",
            p.y
        );
        assert!(p.x > 1.5, "should also advance along X");
    }

    #[test]
    fn step_emits_shot_sfx() {
        let mut world = tiny_world(
            "\
...
.P.
...
",
        );
        {
            let p = world.player_mut().unwrap();
            p.fire.cooldown = 0.0;
            p.fire.config.fire_interval = 0.0;
        }
        world.step(
            &SimInput {
                fire_held: true,
                ..Default::default()
            },
            1.0 / 60.0,
        );
        assert!(
            world.sfx.contains(&SimSfx::Shot),
            "firing should queue Shot sfx"
        );
    }
}
