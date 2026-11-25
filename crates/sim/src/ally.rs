//! AI ally (P2 slot) — phase 4.4.
//!
//! Roles: Guard (base threat) → Support (help player / letter) → Hunt (nearest enemy).

use crate::world::Dir;

/// Behaviour mode for the AI ally this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AllyRole {
    /// Stay near HQ / engage enemies threatening the base.
    Guard,
    /// Chase the nearest enemy.
    #[default]
    Hunt,
    /// Stick with the human player or race for a friendly letter.
    Support,
}

/// Pure input for ally goal selection (no World borrow).
#[derive(Debug, Clone, Copy)]
pub struct AllyGoalInput {
    pub ally_x: f32,
    pub ally_y: f32,
    pub base: Option<(f32, f32)>,
    pub player: Option<(f32, f32)>,
    /// Nearest enemy to the base (if any within `guard_radius`).
    pub enemy_near_base: Option<(f32, f32)>,
    /// Nearest enemy to the ally.
    pub nearest_enemy: Option<(f32, f32)>,
    /// True when a player-aimed threat / nearby enemy bullet corridor.
    pub player_threatened: bool,
    /// Pickup the ally may take (player/both).
    pub bonus: Option<(f32, f32)>,
    pub guard_radius: f32,
    pub support_leash: f32,
}

/// Resolved role + aim point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AllyGoal {
    pub role: AllyRole,
    pub x: f32,
    pub y: f32,
}

/// Default distance (tiles) at which an enemy counts as a base threat.
pub const ALLY_GUARD_RADIUS: f32 = 5.5;
/// If farther from the human than this, prefer Support (regroup).
pub const ALLY_SUPPORT_LEASH: f32 = 6.0;

/// Priority: Guard → Support → Hunt.
pub fn pick_ally_goal(input: AllyGoalInput) -> Option<AllyGoal> {
    if let Some((ex, ey)) = input.enemy_near_base {
        return Some(AllyGoal {
            role: AllyRole::Guard,
            x: ex,
            y: ey,
        });
    }

    let far_from_player = match input.player {
        Some((px, py)) => (px - input.ally_x).hypot(py - input.ally_y) > input.support_leash,
        None => false,
    };

    if input.player_threatened || far_from_player || input.bonus.is_some() {
        if let Some((bx, by)) = input.bonus {
            return Some(AllyGoal {
                role: AllyRole::Support,
                x: bx,
                y: by,
            });
        }
        if let Some((px, py)) = input.player {
            return Some(AllyGoal {
                role: AllyRole::Support,
                x: px,
                y: py,
            });
        }
    }

    if let Some((ex, ey)) = input.nearest_enemy {
        return Some(AllyGoal {
            role: AllyRole::Hunt,
            x: ex,
            y: ey,
        });
    }

    // Idle: loiter near base or player.
    if let Some((bx, by)) = input.base {
        return Some(AllyGoal {
            role: AllyRole::Guard,
            x: bx,
            y: by,
        });
    }
    input.player.map(|(px, py)| AllyGoal {
        role: AllyRole::Support,
        x: px,
        y: py,
    })
}

/// Facing helper shared with enemy AI style.
pub fn ally_face_toward(from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> Option<Dir> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn base_input() -> AllyGoalInput {
        AllyGoalInput {
            ally_x: 7.5,
            ally_y: 12.5,
            base: Some((7.5, 14.5)),
            player: Some((5.5, 12.5)),
            enemy_near_base: None,
            nearest_enemy: Some((2.5, 2.5)),
            player_threatened: false,
            bonus: None,
            guard_radius: ALLY_GUARD_RADIUS,
            support_leash: ALLY_SUPPORT_LEASH,
        }
    }

    #[test]
    fn guard_beats_hunt_when_base_threatened() {
        let mut i = base_input();
        i.enemy_near_base = Some((7.5, 11.5));
        let g = pick_ally_goal(i).unwrap();
        assert_eq!(g.role, AllyRole::Guard);
        assert_eq!((g.x, g.y), (7.5, 11.5));
    }

    #[test]
    fn hunt_when_clear() {
        let g = pick_ally_goal(base_input()).unwrap();
        assert_eq!(g.role, AllyRole::Hunt);
    }

    #[test]
    fn support_when_player_threatened() {
        let mut i = base_input();
        i.player_threatened = true;
        let g = pick_ally_goal(i).unwrap();
        assert_eq!(g.role, AllyRole::Support);
        assert_eq!((g.x, g.y), (5.5, 12.5));
    }

    #[test]
    fn support_prefers_bonus_over_player() {
        let mut i = base_input();
        i.player_threatened = true;
        i.bonus = Some((1.5, 1.5));
        let g = pick_ally_goal(i).unwrap();
        assert_eq!(g.role, AllyRole::Support);
        assert_eq!((g.x, g.y), (1.5, 1.5));
    }
}
