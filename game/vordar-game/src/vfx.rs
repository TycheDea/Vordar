// Cosmetic VFX markers — data only, no systems. They live in the shared game
// crate (not the client) so prefabs that carry them parse on the server too;
// the server simply never reads them. The client's vfx module turns them into
// particles.

use glam::Vec3;
use serde::Deserialize;

/// Emit a particle trail from this entity while it moves (projectiles).
/// `rate` is particles per second.
#[derive(Clone, Debug, Deserialize)]
pub struct VfxTrail {
    pub color: Vec3,
    pub rate:  f32,
}
