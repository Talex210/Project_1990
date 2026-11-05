//! Local profiles, progress, and solo high-score board.
//!
//! Text schema is versioned so the same fields can later map to a network payload.
//! Coop / 2P gets a separate board later — not mixed into Solo.

use crate::level::LEVEL_COUNT;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// On-disk / wire schema (v2: named profiles + global solo board).
pub const SAVE_SCHEMA: u32 = 2;
/// Soft cap on local named profiles.
pub const MAX_PROFILES: usize = 8;
/// Rows in the Solo hall-of-fame table.
pub const MAX_BOARD_ROWS: usize = 10;
/// Max UTF-8 characters in a profile name.
pub const MAX_NAME_CHARS: usize = 12;

/// One row on the Solo records table (name shown for arcade feel / future sync).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardEntry {
    pub name: String,
    pub score: u32,
    pub stage: u32,
}

/// Progress for one named player. `best_run_score` = personal best (not the full table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub continue_stage: u32,
    pub highest_stage_reached: u32,
    pub highest_stage_cleared: u32,
    /// Personal best run score for this profile.
    pub best_run_score: u32,
}

impl Profile {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            continue_stage: 1,
            highest_stage_reached: 1,
            highest_stage_cleared: 0,
            best_run_score: 0,
        }
    }

    pub fn note_stage_cleared(&mut self, score: u32, stage: u32) {
        let stage = stage.clamp(1, LEVEL_COUNT);
        self.highest_stage_reached = self.highest_stage_reached.max(stage);
        self.highest_stage_cleared = self.highest_stage_cleared.max(stage);
        self.continue_stage = (stage + 1).min(LEVEL_COUNT);
        self.best_run_score = self.best_run_score.max(score);
    }

    pub fn note_game_over(&mut self, score: u32, stage: u32) {
        let stage = stage.clamp(1, LEVEL_COUNT);
        self.highest_stage_reached = self.highest_stage_reached.max(stage);
        self.continue_stage = 1;
        self.best_run_score = self.best_run_score.max(score);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveData {
    pub schema: u32,
    pub active_profile: usize,
    pub profiles: Vec<Profile>,
    /// Shared Solo leaderboard (all profiles). Coop board — later.
    pub solo_board: Vec<BoardEntry>,
}

impl Default for SaveData {
    fn default() -> Self {
        Self {
            schema: SAVE_SCHEMA,
            active_profile: 0,
            profiles: Vec::new(),
            solo_board: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    EmptyName,
    NameTaken,
    NameTooLong,
    ProfileLimit,
    NoProfiles,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileError::EmptyName => write!(f, "name is empty"),
            ProfileError::NameTaken => write!(f, "name already taken"),
            ProfileError::NameTooLong => write!(f, "name too long (max {MAX_NAME_CHARS})"),
            ProfileError::ProfileLimit => write!(f, "profile limit ({MAX_PROFILES}) reached"),
            ProfileError::NoProfiles => write!(f, "no profiles"),
        }
    }
}

#[derive(Debug)]
pub enum SaveError {
    Io(io::Error),
    Parse(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "save io: {e}"),
            SaveError::Parse(e) => write!(f, "save parse: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<io::Error> for SaveError {
    fn from(value: io::Error) -> Self {
        SaveError::Io(value)
    }
}

impl SaveData {
    pub fn has_profiles(&self) -> bool {
        !self.profiles.is_empty()
    }

    pub fn active(&self) -> Option<&Profile> {
        self.profiles.get(self.active_profile)
    }

    pub fn active_mut(&mut self) -> Option<&mut Profile> {
        self.profiles.get_mut(self.active_profile)
    }

    pub fn select_profile(&mut self, index: usize) -> bool {
        if index < self.profiles.len() {
            self.active_profile = index;
            true
        } else {
            false
        }
    }

    pub fn normalize_name(raw: &str) -> String {
        raw.trim().chars().take(MAX_NAME_CHARS).collect()
    }

    pub fn name_key(name: &str) -> String {
        Self::normalize_name(name).to_lowercase()
    }

    pub fn is_name_taken(&self, name: &str) -> bool {
        let key = Self::name_key(name);
        if key.is_empty() {
            return false;
        }
        self.profiles
            .iter()
            .any(|p| Self::name_key(&p.name) == key)
    }

    pub fn create_profile(&mut self, raw_name: &str) -> Result<usize, ProfileError> {
        let name = Self::normalize_name(raw_name);
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(ProfileError::NameTooLong);
        }
        if self.profiles.len() >= MAX_PROFILES {
            return Err(ProfileError::ProfileLimit);
        }
        if self.is_name_taken(&name) {
            return Err(ProfileError::NameTaken);
        }
        let id = format!("local-{}", next_id_suffix(&self.profiles));
        self.profiles.push(Profile::new(id, name));
        let idx = self.profiles.len() - 1;
        self.active_profile = idx;
        Ok(idx)
    }

    /// Update profile progress and push a Solo board row when the run ends.
    pub fn record_run_end(&mut self, score: u32, stage: u32, victory: bool) {
        let name = self
            .active()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Player".into());
        if let Some(p) = self.active_mut() {
            if victory {
                p.note_stage_cleared(score, stage);
            } else {
                p.note_game_over(score, stage);
            }
        }
        let finished = !victory || stage == LEVEL_COUNT;
        if finished {
            self.insert_solo_score(BoardEntry {
                name,
                score,
                stage: stage.clamp(1, LEVEL_COUNT),
            });
        }
    }

    pub fn insert_solo_score(&mut self, entry: BoardEntry) {
        if entry.score == 0 {
            return;
        }
        self.solo_board.push(entry);
        self.solo_board.sort_by(|a, b| b.score.cmp(&a.score));
        self.solo_board.truncate(MAX_BOARD_ROWS);
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# Project 1990 save schema=2 — Solo board; Coop board later\n");
        out.push_str(&format!("schema={}\n", self.schema));
        out.push_str(&format!("active_profile={}\n", self.active_profile));
        for (i, p) in self.profiles.iter().enumerate() {
            out.push_str(&format!("\n[profile:{i}]\n"));
            out.push_str(&format!("id={}\n", p.id));
            out.push_str(&format!("name={}\n", escape_name(&p.name)));
            out.push_str(&format!("continue_stage={}\n", p.continue_stage));
            out.push_str(&format!(
                "highest_stage_reached={}\n",
                p.highest_stage_reached
            ));
            out.push_str(&format!(
                "highest_stage_cleared={}\n",
                p.highest_stage_cleared
            ));
            out.push_str(&format!("best_run_score={}\n", p.best_run_score));
        }
        out.push_str("\n[solo_board]\n");
        for (j, e) in self.solo_board.iter().enumerate() {
            out.push_str(&format!(
                "row.{j}={},{},{}\n",
                escape_name(&e.name),
                e.score,
                e.stage
            ));
        }
        out
    }

    pub fn parse(text: &str) -> Result<Self, SaveError> {
        let mut data = SaveData::default();
        let mut current: Option<ParseSection> = None;

        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("[profile:") {
                let idx_str = rest.strip_suffix(']').ok_or_else(|| {
                    SaveError::Parse(format!("line {}: bad profile header", line_no + 1))
                })?;
                let idx: usize = idx_str.parse().map_err(|_| {
                    SaveError::Parse(format!("line {}: bad profile index", line_no + 1))
                })?;
                while data.profiles.len() <= idx {
                    let n = data.profiles.len();
                    data.profiles
                        .push(Profile::new(format!("local-{n}"), format!("P{n}")));
                }
                current = Some(ParseSection::Profile(idx));
                continue;
            }
            if line == "[solo_board]" {
                current = Some(ParseSection::SoloBoard);
                continue;
            }
            let (key, val) = line.split_once('=').ok_or_else(|| {
                SaveError::Parse(format!("line {}: expected key=value", line_no + 1))
            })?;
            let key = key.trim();
            let val = val.trim();
            match current {
                None => match key {
                    "schema" => {
                        data.schema = val.parse().map_err(|_| {
                            SaveError::Parse(format!("line {}: bad schema", line_no + 1))
                        })?;
                        if data.schema != SAVE_SCHEMA {
                            return Err(SaveError::Parse(format!(
                                "unsupported schema {} (want {SAVE_SCHEMA})",
                                data.schema
                            )));
                        }
                    }
                    "active_profile" => {
                        data.active_profile = val.parse().map_err(|_| {
                            SaveError::Parse(format!("line {}: bad active_profile", line_no + 1))
                        })?;
                    }
                    other => {
                        return Err(SaveError::Parse(format!(
                            "line {}: unknown top-level key {other}",
                            line_no + 1
                        )));
                    }
                },
                Some(ParseSection::Profile(idx)) => {
                    let p = &mut data.profiles[idx];
                    match key {
                        "id" => p.id = val.to_string(),
                        "name" => p.name = unescape_name(val),
                        "continue_stage" => p.continue_stage = parse_u32(val, line_no)?,
                        "highest_stage_reached" => {
                            p.highest_stage_reached = parse_u32(val, line_no)?
                        }
                        "highest_stage_cleared" => {
                            p.highest_stage_cleared = parse_u32(val, line_no)?
                        }
                        "best_run_score" => p.best_run_score = parse_u32(val, line_no)?,
                        other => {
                            return Err(SaveError::Parse(format!(
                                "line {}: unknown profile key {other}",
                                line_no + 1
                            )));
                        }
                    }
                }
                Some(ParseSection::SoloBoard) => {
                    if let Some(idx_str) = key.strip_prefix("row.") {
                        let _idx: usize = idx_str.parse().map_err(|_| {
                            SaveError::Parse(format!("line {}: bad row index", line_no + 1))
                        })?;
                        let parts: Vec<&str> = val.splitn(3, ',').collect();
                        if parts.len() != 3 {
                            return Err(SaveError::Parse(format!(
                                "line {}: row needs name,score,stage",
                                line_no + 1
                            )));
                        }
                        let score: u32 = parts[1].trim().parse().map_err(|_| {
                            SaveError::Parse(format!("line {}: bad score", line_no + 1))
                        })?;
                        let stage: u32 = parts[2].trim().parse().map_err(|_| {
                            SaveError::Parse(format!("line {}: bad stage", line_no + 1))
                        })?;
                        data.solo_board.push(BoardEntry {
                            name: unescape_name(parts[0].trim()),
                            score,
                            stage,
                        });
                    } else {
                        return Err(SaveError::Parse(format!(
                            "line {}: unknown solo_board key {key}",
                            line_no + 1
                        )));
                    }
                }
            }
        }

        if data.active_profile >= data.profiles.len() && !data.profiles.is_empty() {
            data.active_profile = 0;
        }
        for p in &mut data.profiles {
            p.continue_stage = p.continue_stage.clamp(1, LEVEL_COUNT);
        }
        data.solo_board.sort_by(|a, b| b.score.cmp(&a.score));
        data.solo_board.truncate(MAX_BOARD_ROWS);
        Ok(data)
    }

    pub fn load_path(path: &Path) -> Result<Self, SaveError> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_path(&self, path: &Path) -> Result<(), SaveError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_text())?;
        Ok(())
    }
}

enum ParseSection {
    Profile(usize),
    SoloBoard,
}

fn next_id_suffix(profiles: &[Profile]) -> u64 {
    profiles
        .iter()
        .filter_map(|p| p.id.strip_prefix("local-")?.parse::<u64>().ok())
        .max()
        .map(|n| n + 1)
        .unwrap_or(0)
}

fn parse_u32(val: &str, line_no: usize) -> Result<u32, SaveError> {
    val.parse()
        .map_err(|_| SaveError::Parse(format!("line {}: bad u32", line_no + 1)))
}

fn escape_name(name: &str) -> String {
    name.replace('\\', "\\\\")
        .replace(',', "\\c")
        .replace('\n', "\\n")
}

fn unescape_name(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('c') => out.push(','),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn default_save_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("PROJECT1990_SAVE_DIR") {
        return PathBuf::from(dir).join("save_v1.txt");
    }
    PathBuf::from("saves").join("save_v1.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_default_roundtrip() {
        let s = SaveData::default();
        let back = SaveData::parse(&s.to_text()).unwrap();
        assert!(back.profiles.is_empty());
        assert!(back.solo_board.is_empty());
        assert_eq!(back.schema, SAVE_SCHEMA);
    }

    #[test]
    fn create_profile_unique_names() {
        let mut s = SaveData::default();
        s.create_profile("Ada").unwrap();
        assert!(matches!(
            s.create_profile(" ada "),
            Err(ProfileError::NameTaken)
        ));
        s.create_profile("Bob").unwrap();
        assert_eq!(s.profiles.len(), 2);
    }

    #[test]
    fn solo_board_keeps_top_ten() {
        let mut s = SaveData::default();
        s.create_profile("Ada").unwrap();
        for score in 1..=15 {
            s.insert_solo_score(BoardEntry {
                name: "Ada".into(),
                score: score * 100,
                stage: 1,
            });
        }
        assert_eq!(s.solo_board.len(), MAX_BOARD_ROWS);
        assert_eq!(s.solo_board[0].score, 1500);
    }

    #[test]
    fn game_over_records_board_victory_mid_does_not() {
        let mut s = SaveData::default();
        s.create_profile("Ada").unwrap();
        s.record_run_end(500, 2, true);
        assert!(s.solo_board.is_empty());
        assert_eq!(s.active().unwrap().continue_stage, 3);
        s.record_run_end(800, 3, false);
        assert_eq!(s.solo_board.len(), 1);
        assert_eq!(s.solo_board[0].score, 800);
        assert_eq!(s.active().unwrap().continue_stage, 1);
        assert_eq!(s.active().unwrap().best_run_score, 800);
    }

    #[test]
    fn profile_limit() {
        let mut s = SaveData::default();
        for i in 0..MAX_PROFILES {
            s.create_profile(&format!("P{i}")).unwrap();
        }
        assert!(matches!(
            s.create_profile("Extra"),
            Err(ProfileError::ProfileLimit)
        ));
    }
}
