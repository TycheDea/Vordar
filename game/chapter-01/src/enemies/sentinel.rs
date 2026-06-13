// Sentinel — passive ranged turret (aggro 0): a violet obelisk that opens
// fire with shard bolts only when Provoked. Stats + model:
// ../../../content/chapters/chapter01/prefabs/sentinel.ron (projectile:
// shard_bolt.ron).

use vordar_game::enemies::BehaviorRegistry;

pub const PREFAB: &str = "sentinel";

pub fn register(_registry: &mut BehaviorRegistry) {
    // Data-driven default (ranged retaliation) — nothing to override yet.
}
