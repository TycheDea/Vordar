// Combat — the shared damage mechanics every entity type uses: projectiles,
// contact damage, death, and the scheduled-snapshot Mechanic component.

pub mod buff;
pub mod contact_damage;
pub mod death;
pub mod leap;
pub mod mechanic;
pub mod projectile;
pub mod stats;

pub use buff::BuffStack;
pub use contact_damage::ContactDamage;
pub use death::OnDeath;
pub use leap::LeapImpulse;
pub use mechanic::Mechanic;
pub use projectile::Projectile;
pub use stats::CombatStats;
