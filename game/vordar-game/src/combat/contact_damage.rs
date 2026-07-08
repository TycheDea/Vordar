// ContactDamageSystem — applies damage from ContactDamage bearers to their collision targets.
//
// A collision between A and B:
//   - If A has ContactDamage → deal A.amount to B's Health
//   - If B has ContactDamage → deal B.amount to A's Health
//   - If neither has ContactDamage → no damage (e.g. two Solid walls touching)

use crate::combat::buff::ravager_mods;
use crate::combat::stats::{compute_damage, CombatStats, DamageType};
use crate::events::DamageDealt;
use engine_app::events::{CollisionStarted, EventBus};
use engine_app::scheduler::System;
use engine_core::components::Health;
use engine_core::traits::Resources;
use engine_core::World;
use hecs::Entity;

/// Deals this much damage to whatever this entity touches.
/// Entities without this component cannot deal contact damage.
#[derive(Clone, serde::Deserialize)]
pub struct ContactDamage {
    pub amount: i32,
    /// Untyped content defaults to Physical.
    #[serde(default)]
    pub damage_type: DamageType,
}

pub struct ContactDamageSystem;

impl System for ContactDamageSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Collect first: emitting DamageDealt below needs the bus mutably.
        let contacts: Vec<(Entity, Entity, i32, DamageType)> = {
            let bus = resources.get::<EventBus>().expect("EventBus not in resources");
            bus.read::<CollisionStarted>()
                .flat_map(|e| [(e.a, e.b), (e.b, e.a)])
                .filter_map(|(attacker, target)| {
                    let c = world.get::<&ContactDamage>(attacker).ok()?;
                    Some((attacker, target, c.amount, c.damage_type))
                })
                .collect()
        };
        for (attacker, target, amount, damage_type) in contacts {
            apply(world, resources, attacker, target, amount, damage_type);
        }
    }
}

fn apply(
    world: &World,
    resources: &mut Resources,
    attacker: Entity,
    target: Entity,
    amount: i32,
    damage_type: DamageType,
) {
    let dmg = {
        let atk = world.get::<&CombatStats>(attacker).ok();
        let def = world.get::<&CombatStats>(target).ok();
        let seed = attacker.to_bits().get() ^ target.to_bits().get().rotate_left(21);
        let (bonus_power, mult) = ravager_mods(world, attacker, target);
        let base = compute_damage(amount + bonus_power, damage_type, atk.as_deref(), def.as_deref(), seed);
        (base as f32 * mult).round() as i32
    };
    if let Ok(mut health) = world.get::<&mut Health>(target) {
        health.current -= dmg;
        resources
            .get_mut::<EventBus>()
            .unwrap()
            .emit(DamageDealt { attacker, target, amount: dmg });
    }
}
