// Cinder imp — aggressive ranged harasser: engages on sight (aggro 12),
// closes to range 9, stops, and lobs ember bolts on a 1.8 s cooldown.
// Stats + model: ../../../content/chapters/chapter01/prefabs/cinder_imp.ron
// (projectile: ember_bolt.ron).

use vordar_game::enemies::BehaviorRegistry;

pub const PREFAB: &str = "cinder_imp";

pub fn register(_registry: &mut BehaviorRegistry) {
    // Data-driven default (ranged hold-and-fire) — nothing to override yet.
    // Kiting (backpedal when the target closes in) would land here.
}
