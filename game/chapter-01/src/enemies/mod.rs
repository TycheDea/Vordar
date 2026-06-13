// Chapter 1's enemy archetypes — one module per enemy. Each owns everything
// that makes its enemy ITS enemy: the doc of how it plays, a pointer to its
// stats/model RON, and (when it outgrows the data-driven default) a custom
// EnemyBehavior registered here. Shared mechanics (engagement, projectiles,
// contact damage, death) stay in vordar-game.

pub mod cinder_imp;
pub mod grunt;
pub mod mossback;
pub mod sentinel;

use vordar_game::enemies::BehaviorRegistry;

/// Called by Chapter01Plugin — every archetype gets its registration hook,
/// data-driven or not, so adding a custom behavior never touches shared code.
pub fn register_behaviors(registry: &mut BehaviorRegistry) {
    grunt::register(registry);
    mossback::register(registry);
    cinder_imp::register(registry);
    sentinel::register(registry);
}
