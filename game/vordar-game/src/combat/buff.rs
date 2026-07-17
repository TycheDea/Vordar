// The Ravager's passives — deliberately NOT a general aura/debuff framework,
// just the two effects the class needs (CHARACTER-SYSTEM.md: "rage-trance
// duelist... relentless pressure"):
//
//   Rage: every hit the Ravager lands adds a stack (capped); each stack adds
//   flat power; any new stack refreshes the decay timer. BuffStack is
//   runtime-only state — never authored in RON.
//
//   Finishing Blow: +40% damage against targets below 30% health.
//
// Both plug into the damage sites through `ravager_mods` so the generic
// compute_damage formula stays class-agnostic.

use crate::events::DamageDealt;
use crate::player::class::ClassId;
use engine_app::events::EventBus;
use engine_app::scheduler::System;
use engine_core::components::Health;
use engine_core::traits::Resources;
use engine_core::World;
use hecs::Entity;

/// Rage: flat power added per stack.
pub const RAGE_POWER_PER_STACK: i32 = 2;
/// Rage: stack cap.
pub const RAGE_MAX_STACKS: u32 = 5;
/// Rage: seconds until all stacks drop, refreshed by every new stack.
pub const RAGE_DURATION_SECS: f32 = 4.0;
/// Finishing Blow: outgoing multiplier against low targets.
pub const FINISHING_MULT: f32 = 1.4;
/// Finishing Blow: the target counts as "low" strictly below this health
/// percentage (integer math — the boundary is exact).
pub const FINISHING_THRESHOLD_PCT: i32 = 30;

/// A stacking, decaying combat buff. Runtime-only (code-inserted).
pub struct BuffStack {
    pub stacks: u32,
    /// Seconds left; all stacks drop at zero.
    pub remaining: f32,
}

/// The Ravager's passive modifiers for one attack: (bonus power, outgoing
/// multiplier). (0, 1.0) for every other class — a no-op at the damage sites.
pub fn ravager_mods(world: &World, attacker: Entity, target: Entity) -> (i32, f32) {
    let is_ravager = world
        .get::<&ClassId>(attacker)
        .map(|c| c.id == "ravager")
        .unwrap_or(false);
    if !is_ravager {
        return (0, 1.0);
    }
    let bonus = world
        .get::<&BuffStack>(attacker)
        .map(|b| b.stacks as i32 * RAGE_POWER_PER_STACK)
        .unwrap_or(0);
    let mult = world
        .get::<&Health>(target)
        .map(|h| {
            if h.current * 100 < h.max * FINISHING_THRESHOLD_PCT {
                FINISHING_MULT
            } else {
                1.0
            }
        })
        .unwrap_or(1.0);
    (bonus, mult)
}

/// Grants Rage stacks from DamageDealt events (Ravager attackers only).
/// Server-registered at PostUpdate after MechanicResolveSystem so one run
/// sees the tick's contact/projectile hits (CollisionResolve) AND mechanic
/// hits (PostUpdate) before ClearEvents wipes them next Input.
pub struct RavagerRageSystem;

impl System for RavagerRageSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let attackers: Vec<Entity> = {
            let bus = resources.expect::<EventBus>();
            bus.read::<DamageDealt>().map(|e| e.attacker).collect()
        };
        for attacker in attackers {
            let is_ravager = world
                .get::<&ClassId>(attacker)
                .map(|c| c.id == "ravager")
                .unwrap_or(false);
            if !is_ravager {
                continue;
            }
            let stacked = match world.get::<&mut BuffStack>(attacker) { Ok(mut buff) => {
                buff.stacks = (buff.stacks + 1).min(RAGE_MAX_STACKS);
                buff.remaining = RAGE_DURATION_SECS;
                true
            } _ => {
                false
            }};
            if !stacked {
                let _ = world.insert_one(attacker, BuffStack { stacks: 1, remaining: RAGE_DURATION_SECS });
            }
        }
    }
}

/// Winds every BuffStack down and drops it at zero. Pure decay, no clock —
/// safe everywhere CoreGamePlugin runs (server and sandbox).
pub struct BuffDecaySystem;

impl System for BuffDecaySystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, delta: f32) {
        let mut expired: Vec<Entity> = Vec::new();
        for (entity, buff) in world.query::<(Entity, &mut BuffStack)>().iter() {
            buff.remaining -= delta;
            if buff.remaining <= 0.0 {
                expired.push(entity);
            }
        }
        for entity in expired {
            let _ = world.remove_one::<BuffStack>(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::contact_damage::{ContactDamage, ContactDamageSystem};
    use engine_app::events::CollisionStarted;

    const DT: f32 = 1.0 / 60.0;

    fn ravager(world: &mut World) -> Entity {
        world.spawn((ClassId { id: "ravager".into() },))
    }

    fn hit(bus_res: &mut Resources, attacker: Entity, target: Entity) {
        bus_res
            .get_mut::<EventBus>()
            .unwrap()
            .emit(DamageDealt { attacker, target, amount: 10 });
    }

    #[test]
    fn rage_stacks_cap_and_refresh() {
        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(EventBus::new());
        let atk = ravager(&mut world);
        let victim = world.spawn((Health { current: 100, max: 100 },));

        let mut rage = RavagerRageSystem;
        for _ in 0..(RAGE_MAX_STACKS + 3) {
            hit(&mut resources, atk, victim);
        }
        rage.run(&mut world, &mut resources, DT);
        assert_eq!(world.get::<&BuffStack>(atk).unwrap().stacks, RAGE_MAX_STACKS, "stacks cap");

        // Decay almost out, then one hit refreshes the timer with stacks kept.
        let mut decay = BuffDecaySystem;
        decay.run(&mut world, &mut resources, RAGE_DURATION_SECS - 0.1);
        resources.get_mut::<EventBus>().unwrap().clear();
        hit(&mut resources, atk, victim);
        rage.run(&mut world, &mut resources, DT);
        let buff = world.get::<&BuffStack>(atk).unwrap();
        assert_eq!(buff.stacks, RAGE_MAX_STACKS);
        assert_eq!(buff.remaining, RAGE_DURATION_SECS, "any new stack refreshes the timer");
    }

    #[test]
    fn rage_expires_and_ignores_other_classes() {
        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(EventBus::new());
        let atk = ravager(&mut world);
        let human = world.spawn((ClassId { id: "human".into() },));
        let victim = world.spawn((Health { current: 100, max: 100 },));

        hit(&mut resources, atk, victim);
        hit(&mut resources, human, victim);
        RavagerRageSystem.run(&mut world, &mut resources, DT);
        assert!(world.get::<&BuffStack>(atk).is_ok());
        assert!(world.get::<&BuffStack>(human).is_err(), "rage is the Ravager's passive");

        BuffDecaySystem.run(&mut world, &mut resources, RAGE_DURATION_SECS + 0.01);
        assert!(world.get::<&BuffStack>(atk).is_err(), "all stacks drop at zero");
    }

    #[test]
    fn finishing_blow_boundary() {
        let mut world = World::new();
        let atk = ravager(&mut world);
        let low = world.spawn((Health { current: 29, max: 100 },));
        let ok = world.spawn((Health { current: 30, max: 100 },));

        assert_eq!(ravager_mods(&world, atk, low).1, FINISHING_MULT, "below 30% is low");
        assert_eq!(ravager_mods(&world, atk, ok).1, 1.0, "exactly 30% is not");

        let human = world.spawn((ClassId { id: "human".into() },));
        assert_eq!(ravager_mods(&world, human, low), (0, 1.0), "other classes get nothing");
    }

    /// The integration the passives exist for: with rage stacks up, the same
    /// contact hit lands harder the second time.
    #[test]
    fn second_hit_lands_harder_with_rage() {
        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(EventBus::new());
        let atk = world.spawn((
            ClassId { id: "ravager".into() },
            ContactDamage { amount: 10, damage_type: Default::default() },
        ));
        let victim = world.spawn((Health { current: 200, max: 200 },));

        // First contact: no stacks yet → base 10.
        resources.get_mut::<EventBus>().unwrap().emit(CollisionStarted { a: atk, b: victim });
        ContactDamageSystem.run(&mut world, &mut resources, DT);
        let after_first = world.get::<&Health>(victim).unwrap().current;
        assert_eq!(after_first, 190);

        // Rage from the first hit, then the second contact hits for more.
        RavagerRageSystem.run(&mut world, &mut resources, DT);
        resources.get_mut::<EventBus>().unwrap().clear();
        resources.get_mut::<EventBus>().unwrap().emit(CollisionStarted { a: atk, b: victim });
        ContactDamageSystem.run(&mut world, &mut resources, DT);
        let second_hit = after_first - world.get::<&Health>(victim).unwrap().current;
        assert_eq!(second_hit, 10 + RAGE_POWER_PER_STACK, "one stack of rage power");
    }
}
