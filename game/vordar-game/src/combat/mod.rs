// Combat — the shared damage mechanics every entity type uses: projectiles,
// contact damage, death, and the scheduled-snapshot Mechanic component.

pub mod contact_damage;
pub mod death;
pub mod projectile;

pub use contact_damage::ContactDamage;
pub use death::OnDeath;
pub use projectile::Projectile;

/// A scheduled area mechanic (DESIGN.md §3): "was entity E inside this area
/// at server time `resolve_at_micros`" — resolved once, authoritatively, by
/// the server. Lives on a server-local entity (with Transform for the
/// center); no Hitbox/PrefabId, so it neither collides nor replicates.
/// Code-only for now; boss timelines (roadmap P11) will spawn these from data.
#[derive(Clone, Copy)]
pub struct Mechanic {
    pub id: u64,
    pub radius: f32,
    pub damage: i32,
    pub resolve_at_micros: u64,
    /// Excluded from the hit test (you can't blast yourself).
    pub caster: hecs::Entity,
}
