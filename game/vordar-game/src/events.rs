// Gameplay intent events — the only way input reaches the simulation.
//
// Locally the client's input plugin emits these from keyboard + camera state;
// on the server the network layer emits them from validated client messages.
// Gameplay systems never read devices or the renderer directly (DESIGN.md §6:
// intent-driven updates).

use glam::Vec2;
use hecs::Entity;

/// Desired movement direction for one player entity on the world XZ plane
/// (≤ unit length). Emitted each Input tick; a player with no intent this
/// tick stands still.
#[derive(Clone, Copy, Debug)]
pub struct MoveIntent {
    pub entity: Entity,
    pub dir: Vec2,
}
