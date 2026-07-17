// Per-player progression mechanics — XP accumulation and reward attribution.
// XP is a core game component earned via kill events, persistent across respawns
// and relogins, and applicable to any chapter's enemies. Progression lives here
// (core game code) rather than in a content crate, because the server's
// persistence layer cannot depend on chapter crates.

use crate::events::Killed;
use engine_app::events::EventBus;
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;

/// Reward granted to an enemy when it dies — applied via prefab data
/// (e.g. `"XpReward": (amount: 5)` in a grunt.ron), registered by
/// GameComponentsPlugin so every chapter's prefabs carry it.
#[derive(Clone, serde::Deserialize)]
pub struct XpReward {
    pub amount: u32,
}

/// Running XP total, attributed to the player entity that earned it — not a
/// shared world total, so two players in the same zone accrue independently.
pub struct Xp(pub u32);

/// Reads Killed events (emitted by DeathSystem from same-tick DamageDealt
/// attribution) and grants the victim's XpReward to the killer. The killer
/// gains an Xp component on its first kill.
pub struct XpGrantSystem;

impl System for XpGrantSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let grants: Vec<_> = {
            let bus = resources.expect::<EventBus>();
            bus.read::<Killed>()
                .filter_map(|k| world.get::<&XpReward>(k.victim).ok().map(|r| (k.killer, r.amount)))
                .collect()
        };
        for (killer, amount) in grants {
            if world.get::<&Xp>(killer).is_err() {
                let _ = world.insert_one(killer, Xp(0));
            }
            if let Ok(mut xp) = world.get::<&mut Xp>(killer) {
                xp.0 += amount;
                log::info!("XP: {} (+{amount}) for {killer:?}", xp.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::death::DeathSystem;
    use crate::events::DamageDealt;
    use engine_core::components::Health;
    use engine_core::traits::DespawnQueue;

    const DT: f32 = 1.0 / 60.0;

    /// The scenario XP attribution exists for: with two players in the zone,
    /// only whoever dealt the killing blow gains XP — a bystander who never
    /// touched the victim gains none.
    #[test]
    fn only_the_killer_gains_xp() {
        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(EventBus::new());
        resources.insert(DespawnQueue::new());

        let killer = world.spawn(());
        let bystander = world.spawn(());
        let victim = world.spawn((Health { current: 0, max: 50 }, XpReward { amount: 25 }));
        resources
            .get_mut::<EventBus>()
            .unwrap()
            .emit(DamageDealt { attacker: killer, target: victim, amount: 50 });

        DeathSystem::new().run(&mut world, &mut resources, DT);
        XpGrantSystem.run(&mut world, &mut resources, DT);

        assert_eq!(world.get::<&Xp>(killer).unwrap().0, 25, "the killer gains the victim's XpReward");
        assert!(world.get::<&Xp>(bystander).is_err(), "a player who did not land the kill gains no XP");
    }
}
