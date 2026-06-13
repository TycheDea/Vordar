// Razorling grunt — aggressive melee swarmer. The chapter's bread-and-butter
// threat: engages on sight (aggro 10) and runs the target down for contact
// damage. Stats + model: ../../../content/chapters/chapter01/prefabs/grunt.ron
// (prefab name "grunt" is load-bearing: tests and the blood-moon event spawn it).

use vordar_game::enemies::BehaviorRegistry;

pub const PREFAB: &str = "grunt";

pub fn register(_registry: &mut BehaviorRegistry) {
    // Data-driven default (melee chase) — nothing to override yet. Register a
    // custom EnemyBehavior here the day the grunt learns pack tactics.
}
