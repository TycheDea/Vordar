// ContactDamageSystem — applies damage from ContactDamage bearers to their collision targets.
//
// A collision between A and B:
//   - If A has ContactDamage and A/B are on opposite sides (Enemy vs Player)
//     → deal A.amount to B's Health
//   - If B has ContactDamage and B/A are on opposite sides → deal B.amount to A's Health
//   - Same-side contact (enemy-enemy, player-player) never lands, mirroring
//     the projectile side rule in combat/projectile.rs.
//   - If neither has ContactDamage → no damage (e.g. two Solid walls touching)

use crate::combat::buff::ravager_mods;
use crate::combat::stats::{compute_damage, CombatStats, DamageType};
use crate::enemies::Enemy;
use crate::events::DamageDealt;
use crate::player::Player;
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
            let bus = resources.expect::<EventBus>();
            bus.read::<CollisionStarted>()
                .flat_map(|e| [(e.a, e.b), (e.b, e.a)])
                .filter_map(|(attacker, target)| {
                    let c = world.get::<&ContactDamage>(attacker).ok()?;
                    let opposite_sides = (world.get::<&Enemy>(attacker).is_ok()
                        && world.get::<&Player>(target).is_ok())
                        || (world.get::<&Player>(attacker).is_ok()
                            && world.get::<&Enemy>(target).is_ok());
                    if !opposite_sides {
                        return None;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn base_resources() -> Resources {
        let mut resources = Resources::new();
        resources.insert(EventBus::new());
        resources
    }

    fn idle_enemy() -> Enemy {
        Enemy { speed: 2.0, aggro_range: 0.0, attack: Default::default(), cooldown_left: 0.0 }
    }

    #[test]
    fn enemy_on_enemy_contact_passes_through() {
        let mut world = World::new();
        let mut resources = base_resources();
        let attacker = world.spawn((
            idle_enemy(),
            ContactDamage { amount: 10, damage_type: Default::default() },
        ));
        let victim = world.spawn((idle_enemy(), Health { current: 30, max: 30 }));

        resources.get_mut::<EventBus>().unwrap().emit(CollisionStarted { a: attacker, b: victim });
        ContactDamageSystem.run(&mut world, &mut resources, DT);

        assert_eq!(world.get::<&Health>(victim).unwrap().current, 30, "no enemy friendly fire");
    }

    #[test]
    fn player_on_player_contact_passes_through() {
        let mut world = World::new();
        let mut resources = base_resources();
        let attacker = world.spawn((
            Player { speed: 6.0 },
            ContactDamage { amount: 10, damage_type: Default::default() },
        ));
        let victim = world.spawn((Player { speed: 6.0 }, Health { current: 30, max: 30 }));

        resources.get_mut::<EventBus>().unwrap().emit(CollisionStarted { a: attacker, b: victim });
        ContactDamageSystem.run(&mut world, &mut resources, DT);

        assert_eq!(world.get::<&Health>(victim).unwrap().current, 30, "no PvP contact damage");
    }
}
