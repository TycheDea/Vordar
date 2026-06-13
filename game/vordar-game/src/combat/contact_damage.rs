// ContactDamageSystem — applies damage from ContactDamage bearers to their collision targets.
//
// A collision between A and B:
//   - If A has ContactDamage → deal A.amount to B's Health
//   - If B has ContactDamage → deal B.amount to A's Health
//   - If neither has ContactDamage → no damage (e.g. two Solid walls touching)

use engine_app::events::{CollisionStarted, EventBus};
use engine_app::scheduler::System;
use engine_core::components::Health;
use engine_core::traits::Resources;
use engine_core::World;
use hecs::Entity;

/// Deals this much damage to whatever this entity touches.
/// Entities without this component cannot deal contact damage.
#[derive(serde::Deserialize)]
pub struct ContactDamage {
    pub amount: i32,
}

pub struct ContactDamageSystem;

impl System for ContactDamageSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // resources and world are separate borrows — hold bus reference while calling world.get.
        let bus = resources.get::<EventBus>().expect("EventBus not in resources");
        for e in bus.read::<CollisionStarted>() {
            let (a, b) = (e.a, e.b);
            let dmg_a = world.get::<&ContactDamage>(a).ok().map(|c| c.amount);
            let dmg_b = world.get::<&ContactDamage>(b).ok().map(|c| c.amount);

            if let Some(amount) = dmg_a { apply(world, b, amount); }
            if let Some(amount) = dmg_b { apply(world, a, amount); }
        }
    }
}

fn apply(world: &World, target: Entity, amount: i32) {
    if let Ok(mut health) = world.get::<&mut Health>(target) {
        health.current -= amount;
    }
}
