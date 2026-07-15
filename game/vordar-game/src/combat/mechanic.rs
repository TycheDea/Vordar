// Scheduled area mechanic — resolves hit/miss authoritatively at a given time.

use super::stats::DamageType;

/// A scheduled area mechanic (DESIGN.md §3): "was entity E inside this area
/// at server time `resolve_at_micros`" — resolved once, authoritatively, by
/// the server. Lives on a server-local entity (with Transform for the
/// center); no Hitbox/PrefabId, so it neither collides nor replicates.
/// Constructed directly by ability/combat code — no authored (RON) path
/// builds one.
#[derive(Clone, Copy)]
pub struct Mechanic {
    pub id: u64,
    pub radius: f32,
    pub damage: i32,
    pub damage_type: DamageType,
    pub resolve_at_micros: u64,
    /// Excluded from the hit test (you can't blast yourself).
    pub caster: hecs::Entity,
}
