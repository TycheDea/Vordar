// Chapter 1 — a content module. Proves the engine / vordar-game / chapter split:
// this crate ships its own chapter definition, its own prefab directory, its
// enemy archetype modules (enemies/), one chapter-specific component, and one
// system — and wires in as just another plugin. Cross-module communication is
// purely EventBus (HealthDepleted, Killed); nothing in the engine or
// vordar-game knows this crate exists.

pub mod enemies;

use engine_app::app::App;
use engine_app::events::EventBus;
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::traits::Resources;
use engine_core::World;
use vordar_game::chapter::ChapterModule;
use vordar_game::combat::death::DeathSystem;
use vordar_game::events::Killed;

/// This chapter as a linked module — registered with the binaries'
/// ChapterRegistry. First chapter: requires nothing.
pub fn module() -> ChapterModule {
    ChapterModule {
        name: "chapter01",
        requires: &[],
        install: |app| {
            app.add_plugin(Chapter01Plugin);
        },
        install_content: |app| {
            app.add_plugin(Chapter01ContentPlugin);
        },
    }
}

/// Chapter-specific component — granted to the player when this entity dies.
/// Attached via prefab data (see prefabs/grunt.ron), registered by this plugin.
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
            let bus = resources.get::<EventBus>().expect("EventBus not in resources");
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

/// Registration-only subset: component loaders + prefab definitions, no
/// simulation. The networked client needs this to display replicated chapter
/// entities — the chapter's systems and spawning run on the server only.
pub struct Chapter01ContentPlugin;

impl Plugin for Chapter01ContentPlugin {
    fn build(&self, app: &mut App) {
        app.register_component::<XpReward>("XpReward")
            .add_prefab_dir("content/chapters/chapter01/prefabs");
    }
}

pub struct Chapter01Plugin;

impl Plugin for Chapter01Plugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(Chapter01ContentPlugin)
            .insert_resource(vordar_game::chapter::load_chapter("content/chapters/chapter01/chapter.ron"))
            .add_system(XpGrantSystem, Phase::CollisionResolve, SystemOrder::after::<DeathSystem>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::components::Health;
    use engine_core::traits::DespawnQueue;
    use vordar_game::events::DamageDealt;

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
