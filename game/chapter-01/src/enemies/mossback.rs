// Mossback — passive melee tank (aggro 0: retaliates only when Provoked).
// A walking XP piñata that punishes careless farming with high contact
// damage. Stats + model: ../../../content/chapters/chapter01/prefabs/mossback.ron

use vordar_game::enemies::BehaviorRegistry;

pub const PREFAB: &str = "mossback";

pub fn register(_registry: &mut BehaviorRegistry) {
    // Data-driven default (melee retaliation) — nothing to override yet.
}
