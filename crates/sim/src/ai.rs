//! Enemy goal selection (Phase 4.1) + seek actions / anti-stuck (Phase 4.2+).
//!
//! Goals are chosen in the headless sim so priorities stay unit-testable.
//! Movement / shoot-through-brick still uses world path helpers.

use crate::world::Dir;

/// How to act toward the current waypoint (phase 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekAction {
    /// Drive one step.
    Move(Dir),
    /// Face a destructible brick and fire (do not drive into it).
    Shoot(Dir),
}

/// What an enemy is trying to do this moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnemyGoal {
    /// Push toward the HQ (eagle).
    Base,
    /// Hunt the player tank.
    Player,
    /// Race for a letter pickup enemies are allowed to take.
    Bonus,
    /// Strafe / duck when threatened (should keep moving, not park).
    Cover,
    /// Flank when the straight path to the primary target is blocked.
    Bypass,
}

/// Resolved goal + world-space aim point (tile centers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoalTarget {
    pub goal: EnemyGoal,
    pub x: f32,
    pub y: f32,
}

/// Snapshot used by pure priority logic (no World borrow).
#[derive(Debug, Clone, Copy)]
pub struct EnemyGoalInput {
    /// Stable slot for role mix (0 → prefer player hunt, else base).
    pub slot: u32,
    /// This enemy is the designated letter seeker (at most one per tick).
    pub assigned_bonus: Option<(f32, f32)>,
    pub base: Option<(f32, f32)>,
    pub player: Option<(f32, f32)>,
    /// Optional cover aim (brick neighbour). Used as a brief waypoint, not a parking lot.
    pub cover: Option<(f32, f32)>,
    /// Primary (base/player) path is blocked → flank.
    pub primary_blocked: bool,
    /// Player aiming / nearby player bullet.
    pub under_threat: bool,
    /// Seconds without meaningful movement (reserved for future priority tweaks).
    #[allow(dead_code)]
    pub stuck_secs: f32,
}

/// Fallback time-commit if tile commit is unavailable (seconds).
pub const AI_COMMIT_SECS: f32 = 0.28;

/// Priority: Bonus → Bypass → Base/Player.
/// Cover is no longer a parking goal (strafe is handled separately, rarely).
pub fn pick_enemy_goal(input: EnemyGoalInput) -> Option<GoalTarget> {
    let _ = (input.cover, input.under_threat);
    if let Some((x, y)) = input.assigned_bonus {
        return Some(GoalTarget {
            goal: EnemyGoal::Bonus,
            x,
            y,
        });
    }

    let prefer_player = input.slot % 3 == 0;
    let primary = if prefer_player {
        input
            .player
            .map(|(x, y)| GoalTarget {
                goal: EnemyGoal::Player,
                x,
                y,
            })
            .or_else(|| {
                input.base.map(|(x, y)| GoalTarget {
                    goal: EnemyGoal::Base,
                    x,
                    y,
                })
            })
    } else {
        input
            .base
            .map(|(x, y)| GoalTarget {
                goal: EnemyGoal::Base,
                x,
                y,
            })
            .or_else(|| {
                input.player.map(|(x, y)| GoalTarget {
                    goal: EnemyGoal::Player,
                    x,
                    y,
                })
            })
    };

    if input.primary_blocked {
        if let Some(p) = primary {
            let (bx, by) = bypass_flank(p.x, p.y, input.slot);
            return Some(GoalTarget {
                goal: EnemyGoal::Bypass,
                x: bx,
                y: by,
            });
        }
    }

    primary
}

/// Cheap flank offset when the primary lane is closed.
fn bypass_flank(tx: f32, ty: f32, slot: u32) -> (f32, f32) {
    let side = if slot % 2 == 0 { -2.0 } else { 2.0 };
    let along = if slot % 4 < 2 { -2.0 } else { 2.0 };
    (tx + side, ty + along)
}

/// After being stuck on any non-bonus goal, switch to a different objective.
pub fn retarget_when_stuck(
    stuck_secs: f32,
    current: EnemyGoal,
    slot: u32,
    player: Option<(f32, f32)>,
    base: Option<(f32, f32)>,
) -> Option<GoalTarget> {
    if stuck_secs < STUCK_RETARGET_SECS {
        return None;
    }
    if current == EnemyGoal::Bonus {
        return None;
    }
    // Alternate pressure: anything stuck near HQ / flank / cover → hunt player;
    // stuck hunting player → push base (or a fresh flank).
    let _ = slot;
    let want_player = match current {
        EnemyGoal::Player => false,
        EnemyGoal::Base | EnemyGoal::Bypass | EnemyGoal::Cover => true,
        EnemyGoal::Bonus => true, // unreachable due to early return
    };
    if want_player {
        player
            .map(|(x, y)| GoalTarget {
                goal: EnemyGoal::Player,
                x,
                y,
            })
            .or_else(|| {
                base.map(|(x, y)| {
                    let (bx, by) = bypass_flank(x, y, slot.wrapping_add(1));
                    GoalTarget {
                        goal: EnemyGoal::Bypass,
                        x: bx,
                        y: by,
                    }
                })
            })
    } else {
        base.map(|(x, y)| GoalTarget {
            goal: EnemyGoal::Base,
            x,
            y,
        })
        .or_else(|| {
            player.map(|(x, y)| {
                let (bx, by) = bypass_flank(x, y, slot.wrapping_add(2));
                GoalTarget {
                    goal: EnemyGoal::Bypass,
                    x: bx,
                    y: by,
                }
            })
        })
    }
}

/// Time without meaningful movement before forcing a lateral unstick.
pub const STUCK_UNSTICK_SECS: f32 = 0.75;
/// Switch objective if still not progressing.
pub const STUCK_RETARGET_SECS: f32 = 1.4;
/// Max time spent shooting the same brick before we must step aside.
pub const STUCK_SHOOT_MAX_SECS: f32 = 0.85;
/// Movement below this (tiles) counts as no progress this tick window.
pub const STUCK_MOVE_EPS: f32 = 0.04;

/// Facing from `from` toward `to` (axis-aligned), if far enough.
pub fn facing_toward(from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> Option<Dir> {
    let dx = to_x - from_x;
    let dy = to_y - from_y;
    const EPS: f32 = 0.12;
    if dx.abs() < EPS && dy.abs() < EPS {
        return None;
    }
    if dx.abs() >= dy.abs() {
        if dx > 0.0 {
            Some(Dir::Right)
        } else {
            Some(Dir::Left)
        }
    } else if dy > 0.0 {
        Some(Dir::Down)
    } else {
        Some(Dir::Up)
    }
}

/// True when `dir` points roughly at the target (same row/col corridor).
#[allow(dead_code)]
pub fn aims_at(dir: Dir, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> bool {
    let dx = to_x - from_x;
    let dy = to_y - from_y;
    const ALIGN: f32 = 0.55;
    match dir {
        Dir::Up => dy < -0.2 && dx.abs() <= ALIGN,
        Dir::Down => dy > 0.2 && dx.abs() <= ALIGN,
        Dir::Left => dx < -0.2 && dy.abs() <= ALIGN,
        Dir::Right => dx > 0.2 && dy.abs() <= ALIGN,
    }
}

/// Perpendicular dodge dirs relative to a threat facing.
pub fn strafe_dirs(threat_dir: Dir) -> [Dir; 2] {
    match threat_dir {
        Dir::Up | Dir::Down => [Dir::Left, Dir::Right],
        Dir::Left | Dir::Right => [Dir::Up, Dir::Down],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_input() -> EnemyGoalInput {
        EnemyGoalInput {
            slot: 1,
            assigned_bonus: None,
            base: Some((7.5, 14.5)),
            player: Some((7.5, 12.5)),
            cover: Some((3.5, 10.5)),
            primary_blocked: false,
            under_threat: false,
            stuck_secs: 0.0,
        }
    }

    #[test]
    fn bonus_outranks_everything() {
        let mut i = base_input();
        i.assigned_bonus = Some((1.5, 1.5));
        i.under_threat = true;
        i.primary_blocked = true;
        let g = pick_enemy_goal(i).unwrap();
        assert_eq!(g.goal, EnemyGoal::Bonus);
        assert_eq!((g.x, g.y), (1.5, 1.5));
    }

    #[test]
    fn threat_does_not_park_on_cover() {
        let mut i = base_input();
        i.under_threat = true;
        let g = pick_enemy_goal(i).unwrap();
        assert_eq!(g.goal, EnemyGoal::Base, "cover is no longer a stop goal");
    }

    #[test]
    fn bypass_when_primary_blocked() {
        let mut i = base_input();
        i.primary_blocked = true;
        let g = pick_enemy_goal(i).unwrap();
        assert_eq!(g.goal, EnemyGoal::Bypass);
        assert_ne!((g.x, g.y), (7.5, 14.5));
    }

    #[test]
    fn slot_zero_prefers_player() {
        let mut i = base_input();
        i.slot = 0;
        let g = pick_enemy_goal(i).unwrap();
        assert_eq!(g.goal, EnemyGoal::Player);
    }

    #[test]
    fn slot_nonzero_prefers_base() {
        let i = base_input();
        let g = pick_enemy_goal(i).unwrap();
        assert_eq!(g.goal, EnemyGoal::Base);
    }

    #[test]
    fn aims_at_corridor() {
        assert!(aims_at(Dir::Down, 2.5, 1.5, 2.5, 8.5));
        assert!(!aims_at(Dir::Down, 2.5, 1.5, 5.5, 8.5));
    }

    #[test]
    fn retarget_switches_off_base() {
        let g = retarget_when_stuck(
            STUCK_RETARGET_SECS,
            EnemyGoal::Base,
            1,
            Some((1.0, 1.0)),
            Some((7.0, 7.0)),
        )
        .unwrap();
        assert_eq!(g.goal, EnemyGoal::Player);
    }

    #[test]
    fn retarget_switches_off_player() {
        let g = retarget_when_stuck(
            STUCK_RETARGET_SECS,
            EnemyGoal::Player,
            0,
            Some((1.0, 1.0)),
            Some((7.0, 7.0)),
        )
        .unwrap();
        assert_eq!(g.goal, EnemyGoal::Base);
    }
}
