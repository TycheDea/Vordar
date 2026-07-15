// Chapter 1 — a content module. Proves the engine / vordar-game / chapter split:
// this crate ships its own chapter definition, its own prefab directory, its
// enemy archetype modules (enemies/), one chapter-specific component, and one
// system — and wires in as just another plugin. Cross-module communication is
// purely EventBus (HealthDepleted); nothing in the engine or vordar-game
// knows this crate exists.

pub mod enemies;

use engine_app::app::App;
use engine_app::events::{EventBus, HealthDepleted};
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::traits::Resources;
use engine_core::World;
use vordar_game::chapter::ChapterModule;
use vordar_game::combat::death::DeathSystem;

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

/// Running XP total for the player.
pub struct PlayerXp(pub u32);

/// Reads HealthDepleted events (emitted by DeathSystem; the entity is still
/// alive until DespawnFlush, so its components remain readable) and grants XP
/// for entities carrying XpReward.
pub struct XpGrantSystem;

impl System for XpGrantSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let gained: u32 = {
            let bus = resources.get::<EventBus>().expect("EventBus not in resources");
            bus.read::<HealthDepleted>()
                .filter_map(|e| world.get::<&XpReward>(e.entity).ok().map(|r| r.amount))
                .sum()
        };
        if gained > 0 {
            if let Some(xp) = resources.get_mut::<PlayerXp>() {
                xp.0 += gained;
                log::info!("XP: {} (+{gained})", xp.0);
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
            .insert_resource(PlayerXp(0))
            .insert_resource(vordar_game::chapter::load_chapter("content/chapters/chapter01/chapter.ron"))
            .add_system(XpGrantSystem, Phase::CollisionResolve, SystemOrder::after::<DeathSystem>());
    }
}
