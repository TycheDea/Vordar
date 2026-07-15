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

/// Damage landed on a target — emitted by every damage-application site
/// (contact, projectile, mechanic) right after the health change. Consumer:
/// the Ravager's Rage stacks.
#[derive(Clone, Copy, Debug)]
pub struct DamageDealt {
    pub attacker: Entity,
    pub target: Entity,
    pub amount: i32,
}

/// Kill attribution — emitted by DeathSystem for the entity whose
/// DamageDealt most recently targeted the victim this tick (last-hit wins).
/// Consumer: per-player progression stats (e.g. chapter-01's XpGrantSystem).
#[derive(Clone, Copy, Debug)]
pub struct Killed {
    pub victim: Entity,
    pub killer: Entity,
}
