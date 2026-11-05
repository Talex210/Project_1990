//! Data-driven bonuses: catalog parse + effect kinds (see `docs/BONUSES.md`).

use crate::world::Team;
use std::fmt;

/// Who may collect a pickup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupWho {
    Player,
    Enemy,
    Both,
}

impl PickupWho {
    pub fn allows(self, team: Team) -> bool {
        match self {
            PickupWho::Player => team == Team::Player,
            PickupWho::Enemy => team == Team::Enemy,
            PickupWho::Both => true,
        }
    }
}

/// Named effect interpreted by the sim (not hard-coded per letter in physics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectKind {
    Invuln,
    FreezeOpponents,
    FortifyBase,
    KillOpponents,
    TankUpgrade,
    MaxUpgrade,
    ExtraLife,
    WaterWalk,
    ForestClearShots,
}

impl EffectKind {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "Invuln" => Self::Invuln,
            "FreezeOpponents" => Self::FreezeOpponents,
            "FortifyBase" => Self::FortifyBase,
            "KillOpponents" => Self::KillOpponents,
            "TankUpgrade" => Self::TankUpgrade,
            "MaxUpgrade" => Self::MaxUpgrade,
            "ExtraLife" => Self::ExtraLife,
            "WaterWalk" => Self::WaterWalk,
            "ForestClearShots" => Self::ForestClearShots,
            _ => return None,
        })
    }
}

/// When a timed/permanent bonus is stripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CancelFlags {
    pub on_death: bool,
    pub on_level_end: bool,
}

/// One bonus definition from data.
#[derive(Debug, Clone, PartialEq)]
pub struct BonusDef {
    pub id: String,
    pub letter: char,
    /// Short human label for HUD (from `name=` in catalog).
    pub name: String,
    pub who: PickupWho,
    pub duration_secs: f32,
    pub effects: Vec<EffectKind>,
    pub cancel: CancelFlags,
    pub score: u32,
    /// Cap for `TankUpgrade` stacks (`0` = use engine default 3).
    pub max_stacks: u32,
    pub exclusive_group: Option<String>,
}

impl BonusDef {
    /// e.g. `H Helmet`
    pub fn label(&self) -> String {
        format!("{} {}", self.letter, self.name)
    }
}

/// Full catalog loaded from `data/bonuses/catalog.cfg`.
#[derive(Debug, Clone, PartialEq)]
pub struct BonusCatalog {
    pub drop_every_kills: u32,
    /// Field pickup despawn time (classic-style timed letter).
    pub pickup_lifetime_secs: f32,
    pub defs: Vec<BonusDef>,
}

impl Default for BonusCatalog {
    fn default() -> Self {
        Self {
            drop_every_kills: 4,
            pickup_lifetime_secs: 18.0,
            defs: Vec::new(),
        }
    }
}

impl BonusCatalog {
    pub fn get(&self, id: &str) -> Option<&BonusDef> {
        self.defs.iter().find(|d| d.id == id)
    }

    pub fn by_index(&self, index: usize) -> Option<&BonusDef> {
        if self.defs.is_empty() {
            return None;
        }
        self.defs.get(index % self.defs.len())
    }

    /// Parse catalog text (`[id]` sections + top-level keys).
    pub fn parse(text: &str) -> Result<Self, BonusParseError> {
        let mut catalog = BonusCatalog::default();
        let mut current: Option<BonusDefBuilder> = None;

        for (line_no, raw) in text.lines().enumerate() {
            let line_no = line_no + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }

            if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                if let Some(builder) = current.take() {
                    catalog.defs.push(builder.finish(line_no)?);
                }
                let id = name.trim();
                if id.is_empty() {
                    return Err(BonusParseError::BadSection { line: line_no });
                }
                current = Some(BonusDefBuilder::new(id));
                continue;
            }

            let (key, val) = line
                .split_once('=')
                .ok_or(BonusParseError::BadLine { line: line_no })?;
            let key = key.trim();
            let val = val.trim();

            if let Some(builder) = current.as_mut() {
                builder.set(key, val, line_no)?;
            } else {
                match key {
                    "drop_every_kills" => {
                        catalog.drop_every_kills =
                            parse_u32(val, line_no, key)?.max(1);
                    }
                    "pickup_lifetime_secs" => {
                        catalog.pickup_lifetime_secs = parse_f32(val, line_no, key)?.max(1.0);
                    }
                    other => {
                        return Err(BonusParseError::UnknownKey {
                            line: line_no,
                            key: other.to_string(),
                        });
                    }
                }
            }
        }

        if let Some(builder) = current.take() {
            catalog.defs.push(builder.finish(text.lines().count().max(1))?);
        }
        if catalog.defs.is_empty() {
            return Err(BonusParseError::EmptyCatalog);
        }
        Ok(catalog)
    }
}

#[derive(Debug, Clone)]
struct BonusDefBuilder {
    id: String,
    letter: Option<char>,
    name: Option<String>,
    who: PickupWho,
    duration_secs: f32,
    effects: Vec<EffectKind>,
    cancel: CancelFlags,
    score: u32,
    max_stacks: u32,
    exclusive_group: Option<String>,
}

impl BonusDefBuilder {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            letter: None,
            name: None,
            who: PickupWho::Both,
            duration_secs: 0.0,
            effects: Vec::new(),
            cancel: CancelFlags::default(),
            score: 0,
            max_stacks: 0,
            exclusive_group: None,
        }
    }

    fn set(&mut self, key: &str, val: &str, line: usize) -> Result<(), BonusParseError> {
        match key {
            "letter" => {
                let mut chars = val.chars();
                let c = chars.next().ok_or(BonusParseError::BadValue {
                    line,
                    key: key.to_string(),
                })?;
                if chars.next().is_some() {
                    return Err(BonusParseError::BadValue {
                        line,
                        key: key.to_string(),
                    });
                }
                self.letter = Some(c);
            }
            "name" => {
                if val.is_empty() {
                    return Err(BonusParseError::BadValue {
                        line,
                        key: key.to_string(),
                    });
                }
                self.name = Some(val.to_string());
            }
            "who" => {
                self.who = match val {
                    "player" => PickupWho::Player,
                    "enemy" => PickupWho::Enemy,
                    "both" => PickupWho::Both,
                    _ => {
                        return Err(BonusParseError::BadValue {
                            line,
                            key: key.to_string(),
                        });
                    }
                };
            }
            "duration_secs" => {
                self.duration_secs = parse_f32(val, line, key)?;
            }
            "effects" => {
                self.effects.clear();
                for part in val.split(',') {
                    let name = part.trim();
                    if name.is_empty() {
                        continue;
                    }
                    let kind = EffectKind::parse(name).ok_or(BonusParseError::UnknownEffect {
                        line,
                        name: name.to_string(),
                    })?;
                    self.effects.push(kind);
                }
            }
            "cancel" => {
                self.cancel = CancelFlags::default();
                for part in val.split(',') {
                    let flag = part.trim();
                    if flag.is_empty() {
                        continue;
                    }
                    match flag {
                        "death" => self.cancel.on_death = true,
                        "level_end" => self.cancel.on_level_end = true,
                        _ => {
                            return Err(BonusParseError::BadValue {
                                line,
                                key: key.to_string(),
                            });
                        }
                    }
                }
            }
            "score" => self.score = parse_u32(val, line, key)?,
            "max_stacks" => self.max_stacks = parse_u32(val, line, key)?,
            "exclusive_group" => {
                self.exclusive_group = if val.is_empty() {
                    None
                } else {
                    Some(val.to_string())
                };
            }
            other => {
                return Err(BonusParseError::UnknownKey {
                    line,
                    key: other.to_string(),
                });
            }
        }
        Ok(())
    }

    fn finish(self, line: usize) -> Result<BonusDef, BonusParseError> {
        let letter = self.letter.ok_or(BonusParseError::MissingField {
            line,
            id: self.id.clone(),
            field: "letter",
        })?;
        if self.effects.is_empty() {
            return Err(BonusParseError::MissingField {
                line,
                id: self.id.clone(),
                field: "effects",
            });
        }
        let name = self.name.unwrap_or_else(|| self.id.clone());
        Ok(BonusDef {
            id: self.id,
            letter,
            name,
            who: self.who,
            duration_secs: self.duration_secs,
            effects: self.effects,
            cancel: self.cancel,
            score: self.score,
            max_stacks: self.max_stacks,
            exclusive_group: self.exclusive_group,
        })
    }
}

fn parse_u32(val: &str, line: usize, key: &str) -> Result<u32, BonusParseError> {
    val.parse().map_err(|_| BonusParseError::BadValue {
        line,
        key: key.to_string(),
    })
}

fn parse_f32(val: &str, line: usize, key: &str) -> Result<f32, BonusParseError> {
    val.parse().map_err(|_| BonusParseError::BadValue {
        line,
        key: key.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BonusParseError {
    BadLine { line: usize },
    BadSection { line: usize },
    BadValue { line: usize, key: String },
    UnknownKey { line: usize, key: String },
    UnknownEffect { line: usize, name: String },
    MissingField {
        line: usize,
        id: String,
        field: &'static str,
    },
    EmptyCatalog,
}

impl fmt::Display for BonusParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BonusParseError::BadLine { line } => {
                write!(f, "bonuses catalog line {line}: expected key=value or [id]")
            }
            BonusParseError::BadSection { line } => {
                write!(f, "bonuses catalog line {line}: empty [section]")
            }
            BonusParseError::BadValue { line, key } => {
                write!(f, "bonuses catalog line {line}: bad value for {key}")
            }
            BonusParseError::UnknownKey { line, key } => {
                write!(f, "bonuses catalog line {line}: unknown key {key}")
            }
            BonusParseError::UnknownEffect { line, name } => {
                write!(f, "bonuses catalog line {line}: unknown effect {name}")
            }
            BonusParseError::MissingField { line, id, field } => {
                write!(f, "bonuses catalog near line {line}: [{id}] missing {field}")
            }
            BonusParseError::EmptyCatalog => write!(f, "bonuses catalog has no definitions"),
        }
    }
}

impl std::error::Error for BonusParseError {}

/// Pickup sitting on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct BonusPickup {
    pub id: u64,
    pub bonus_id: String,
    pub letter: char,
    pub x: f32,
    pub y: f32,
    /// Seconds until the letter despawns from the field.
    pub lifetime_secs: f32,
}

/// Timed / sticky effect currently applied to a team member or global tempo.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveBonus {
    pub bonus_id: String,
    pub letter: char,
    pub name: String,
    pub team: Team,
    pub tank_id: Option<u64>,
    pub remaining_secs: Option<f32>,
    pub cancel: CancelFlags,
    pub exclusive_group: Option<String>,
    pub effects: Vec<EffectKind>,
}

/// Per-tank combat mods derived from stars / ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TankMods {
    /// Star / gun level 0…4 (4 = forest clear).
    pub upgrade: u8,
    pub water_walk: bool,
    pub forest_clear: bool,
    pub invuln: bool,
}

impl TankMods {
    /// Fire / steel power caps here; ★4 is forest-only bump.
    pub const MAX_FIRE_UPGRADE: u8 = 3;
    /// Absolute cap: 4th star (or 2nd gun) unlocks forest clear.
    pub const MAX_UPGRADE: u8 = 4;
    pub const FOREST_AT: u8 = 4;

    pub fn with_upgrade(mut self, level: u8) -> Self {
        self.upgrade = level.min(Self::MAX_UPGRADE);
        self.forest_clear = self.upgrade >= Self::FOREST_AT;
        self
    }

    pub fn sync_forest(&mut self) {
        self.forest_clear = self.upgrade >= Self::FOREST_AT;
    }
}

/// Runtime bonus field state (pickups + actives + drop counter).
#[derive(Debug, Clone, PartialEq)]
pub struct BonusState {
    pub catalog: BonusCatalog,
    pub pickups: Vec<BonusPickup>,
    pub active: Vec<ActiveBonus>,
    pub kills_since_drop: u32,
    /// Round-robin index into catalog for which letter drops next.
    pub next_drop_index: usize,
    /// Seconds opponents stay frozen (`Team` = who is frozen).
    pub freeze_player_secs: f32,
    pub freeze_enemy_secs: f32,
    /// Shovel fortify timer (player-side steel walls).
    pub fortify_secs: f32,
    /// Saved base-ring tiles to restore when fortify ends: (x, y, tile_as_u8).
    pub fortify_backup: Vec<(u32, u32, u8)>,
}

impl BonusState {
    pub fn new(catalog: BonusCatalog) -> Self {
        Self {
            catalog,
            pickups: Vec::new(),
            active: Vec::new(),
            kills_since_drop: 0,
            next_drop_index: 0,
            freeze_player_secs: 0.0,
            freeze_enemy_secs: 0.0,
            fortify_secs: 0.0,
            fortify_backup: Vec::new(),
        }
    }

    pub fn tick_timers(&mut self, dt: f32) {
        if self.freeze_player_secs > 0.0 {
            self.freeze_player_secs = (self.freeze_player_secs - dt).max(0.0);
        }
        if self.freeze_enemy_secs > 0.0 {
            self.freeze_enemy_secs = (self.freeze_enemy_secs - dt).max(0.0);
        }
        if self.fortify_secs > 0.0 {
            self.fortify_secs = (self.fortify_secs - dt).max(0.0);
        }

        for p in &mut self.pickups {
            p.lifetime_secs = (p.lifetime_secs - dt).max(0.0);
        }
        self.pickups.retain(|p| p.lifetime_secs > 0.0);

        for a in &mut self.active {
            if let Some(rem) = a.remaining_secs.as_mut() {
                *rem = (*rem - dt).max(0.0);
            }
        }
        self.active.retain(|a| match a.remaining_secs {
            Some(r) => r > 0.0,
            None => true,
        });
    }

    pub fn opponents_frozen(&self, team: Team) -> bool {
        match team {
            Team::Player => self.freeze_player_secs > 0.0,
            Team::Enemy => self.freeze_enemy_secs > 0.0,
        }
    }

    pub fn hud_letters(&self, team: Team, tank_id: Option<u64>) -> String {
        self.hud_active_labels(team, tank_id)
            .into_iter()
            .map(|s| s.chars().next().unwrap_or('?'))
            .collect()
    }

    /// Human-readable active bonuses, e.g. `["H Helmet (9s)", "* Star"]`.
    pub fn hud_active_labels(&self, team: Team, tank_id: Option<u64>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for a in &self.active {
            if a.team != team {
                continue;
            }
            if !(a.tank_id.is_none() || a.tank_id == tank_id || tank_id.is_none()) {
                continue;
            }
            let mut label = format!("{} {}", a.letter, a.name);
            if let Some(rem) = a.remaining_secs {
                label.push_str(&format!(" ({:.0}s)", rem.ceil()));
            }
            out.push(label);
        }
        if self.fortify_secs > 0.0 && team == Team::Player {
            let label = format!("L Shovel ({:.0}s)", self.fortify_secs.ceil());
            if !out.iter().any(|s| s.starts_with("L ")) {
                out.push(label);
            }
        }
        let clock = match team {
            Team::Player if self.freeze_enemy_secs > 0.0 => Some(self.freeze_enemy_secs),
            Team::Enemy if self.freeze_player_secs > 0.0 => Some(self.freeze_player_secs),
            _ => None,
        };
        if let Some(secs) = clock {
            let label = format!("C Clock ({:.0}s)", secs.ceil());
            if !out.iter().any(|s| s.starts_with("C ")) {
                out.push(label);
            }
        }
        out
    }

    /// Append permanent power labels (stars / ship) for HUD.
    pub fn hud_power_labels(upgrade: u8, ship: bool) -> Vec<String> {
        let mut out = Vec::new();
        if upgrade > 0 {
            let forest = if upgrade_clears_forest(upgrade) {
                " +Forest"
            } else {
                ""
            };
            out.push(format!("* Star ★{upgrade}{forest}"));
        }
        if ship {
            out.push("B Ship".into());
        }
        out
    }

    /// Labels for pickups still on the field.
    pub fn hud_field_labels(&self) -> Vec<String> {
        self.pickups
            .iter()
            .map(|p| {
                let base = self
                    .catalog
                    .get(&p.bonus_id)
                    .map(|d| d.label())
                    .unwrap_or_else(|| format!("{} ?", p.letter));
                format!("{base} ({:.0}s)", p.lifetime_secs.ceil())
            })
            .collect()
    }

    pub fn cancel_on_death(&mut self, team: Team, tank_id: u64) {
        self.active.retain(|a| {
            !(a.cancel.on_death && a.team == team && a.tank_id == Some(tank_id))
        });
    }

    pub fn cancel_on_level_end(&mut self) {
        self.active.retain(|a| !a.cancel.on_level_end);
        self.freeze_player_secs = 0.0;
        self.freeze_enemy_secs = 0.0;
        self.fortify_secs = 0.0;
        // Leave pickups on the field until the next World is built so a
        // last-kill drop can still be collected on the victory tick.
    }
}

/// Fire tunables derived from star upgrade level (caps at ★3).
pub fn fire_for_upgrade(base_interval: f32, base_max: usize, upgrade: u8) -> (f32, usize) {
    match upgrade.min(TankMods::MAX_FIRE_UPGRADE) {
        0 => (base_interval, base_max.max(1)),
        1 => (base_interval * 0.85, base_max.max(1)),
        2 => (base_interval * 0.7, 2.max(base_max)),
        _ => (base_interval * 0.55, 2.max(base_max)),
    }
}

/// Bullet travel speed scales with stars (base = no bonus; caps at ★3).
pub fn bullet_speed_for_upgrade(base_speed: f32, upgrade: u8) -> f32 {
    match upgrade.min(TankMods::MAX_FIRE_UPGRADE) {
        0 => base_speed,
        1 => base_speed * 1.25,
        2 => base_speed * 1.5,
        _ => base_speed * 1.85,
    }
}

pub fn upgrade_breaks_steel(upgrade: u8) -> bool {
    upgrade >= TankMods::MAX_FIRE_UPGRADE
}

pub fn upgrade_clears_forest(upgrade: u8) -> bool {
    upgrade >= TankMods::FOREST_AT
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
drop_every_kills=2

[helmet]
letter=H
name=Helmet
who=both
duration_secs=10
effects=Invuln
cancel=death,level_end
score=500
exclusive_group=protect

[star]
letter=*
name=Star
who=both
duration_secs=0
effects=TankUpgrade
cancel=death
score=500
max_stacks=3
"#;

    #[test]
    fn parse_sample_catalog() {
        let cat = BonusCatalog::parse(SAMPLE).unwrap();
        assert_eq!(cat.drop_every_kills, 2);
        assert_eq!(cat.defs.len(), 2);
        let h = cat.get("helmet").unwrap();
        assert_eq!(h.letter, 'H');
        assert_eq!(h.name, "Helmet");
        assert_eq!(h.label(), "H Helmet");
        assert_eq!(h.effects, vec![EffectKind::Invuln]);
        assert!(h.cancel.on_death);
        assert_eq!(h.exclusive_group.as_deref(), Some("protect"));
        let s = cat.get("star").unwrap();
        assert_eq!(s.max_stacks, 3);
    }

    #[test]
    fn who_allows_teams() {
        assert!(PickupWho::Both.allows(Team::Player));
        assert!(PickupWho::Player.allows(Team::Player));
        assert!(!PickupWho::Player.allows(Team::Enemy));
    }

    #[test]
    fn fire_upgrade_curve() {
        let (i0, m0) = fire_for_upgrade(0.4, 1, 0);
        assert!((i0 - 0.4).abs() < f32::EPSILON);
        assert_eq!(m0, 1);
        let (i3, m3) = fire_for_upgrade(0.4, 1, 3);
        assert!(i3 < i0);
        assert_eq!(m3, 2);
        assert!(upgrade_breaks_steel(3));
        assert!(!upgrade_breaks_steel(2));
        assert!(upgrade_clears_forest(4));
        assert!(!upgrade_clears_forest(3));
        assert!(bullet_speed_for_upgrade(6.0, 3) > bullet_speed_for_upgrade(6.0, 0));
    }
}
