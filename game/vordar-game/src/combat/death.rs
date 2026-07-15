// DeathSystem — fires OnDeath callbacks and queues despawn for entities with Health <= 0.
//
// Runs after ContactDamageSystem in Phase::CollisionResolve.
// Separating damage application from death handling keeps both systems focused.

use crate::events::{DamageDealt, Killed};
use engine_app::events::{EventBus, HealthDepleted};
use engine_app::scheduler::System;
use engine_core::components::Health;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use hecs::Entity;

/// Callback fired exactly once when this entity's Health reaches zero.
/// Receives a SpawnContext so it can spawn effects, emit events, etc.
/// Code-only (not data-driven) — prefab-spawned entities react to death via
/// the HealthDepleted event instead.
pub struct OnDeath(pub Box<dyn FnOnce(&mut SpawnContext) + Send + Sync>);

pub struct DeathSystem {
    dead: Vec<Entity>,
}

impl DeathSystem {
    pub fn new() -> Self { Self { dead: Vec::new() } }
}

impl System for DeathSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Collect dead entities — world borrow must end before we mutate it below.
        self.dead.clear();
        self.dead.extend(
            world.query::<(Entity, &Health)>().iter()
                .filter(|(_, health)| health.current <= 0)
                .map(|(entity, _)| entity)
        );

        for entity in self.dead.drain(..) {
            // Last-hit attribution: the most recent DamageDealt targeting this
            // entity this tick is credited with the kill.
            let killer = resources
                .get::<EventBus>()
                .unwrap()
                .read::<DamageDealt>()
                .filter(|d| d.target == entity)
                .last()
                .map(|d| d.attacker);

            // Emit event first so other systems in the same phase can react this frame.
            let bus = resources.get_mut::<EventBus>().unwrap();
            bus.emit(HealthDepleted { entity });
            if let Some(killer) = killer {
                bus.emit(Killed { victim: entity, killer });
            }

            // Take the OnDeath component out of the world (consumed — fires once).
            let on_death = world.remove::<(OnDeath,)>(entity).ok().map(|(od,)| od.0);
            if let Some(callback) = on_death {
                callback(&mut SpawnContext { world, resources });
            }

            // Queue despawn; render slot is freed by RenderSlotDespawnSystem in engine-renderer.
            resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
        }
    }
}
