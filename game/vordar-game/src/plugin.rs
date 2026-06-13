// CoreGamePlugin — registers all generic game systems and the game component
// types for data-driven spawning. Content (prefab values, camps, chapter
// tuning) lives in RON files and chapter plugins, never here.
//
// GameComponentsPlugin is the registration-only subset: a networked client
// needs the component loaders and prefab definitions to display replicated
// entities, but must NOT run the simulation (the server is authoritative).

use crate::combat::contact_damage::{ContactDamage, ContactDamageSystem};
use crate::combat::death::DeathSystem;
use crate::combat::projectile::{ProjectileHitSystem, ProjectileTtlSystem};
use crate::enemies::{BehaviorRegistry, Enemy, EnemyAISystem};
use crate::motion::{MovementSystem, SeparationSystem};
use crate::player::{Player, PlayerMovementSystem};
use crate::world::camp::CampSystem;
use crate::world::wave_spawner::{ChapterSetupSystem, WaveSpawnerSystem};
use engine_app::app::App;
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, SystemOrder};

/// Component registrations + shared prefab definitions, no systems.
pub struct GameComponentsPlugin;

impl Plugin for GameComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.register_component::<Player>("Player")
            .register_component::<Enemy>("Enemy")
            .register_component::<ContactDamage>("ContactDamage")
            // Shared prefabs (chapter plugins add their own dirs on top).
            .add_prefab_dir("content/prefabs");
    }
}

pub struct CoreGamePlugin;

impl Plugin for CoreGamePlugin {
    fn build(&self, app: &mut App) {
        // Ensure the per-archetype behavior registry exists; chapter plugins
        // contribute overrides via the same resource_or_default.
        app.resource_or_default::<BehaviorRegistry>();
        app.add_plugin(GameComponentsPlugin)
            // Chapter start (once)
            .add_system(ChapterSetupSystem::new(), Phase::PreUpdate,   SystemOrder::First)
            // Logic
            .add_system(PlayerMovementSystem, Phase::Update,           SystemOrder::First)
            .add_system(EnemyAISystem,        Phase::Update,           SystemOrder::Default)
            .add_system(WaveSpawnerSystem,    Phase::Update,           SystemOrder::Default)
            // World-resident enemy populations; no-op without ActiveChapter.
            .add_system(CampSystem::new(),    Phase::Update,           SystemOrder::Default)
            .add_system(ProjectileTtlSystem,  Phase::Update,           SystemOrder::Default)
            // No-op unless WorldTimeRes + WorldEventsDef resources exist.
            .add_system(crate::world::WorldEventSystem::new(), Phase::Update, SystemOrder::Default)
            .add_system(MovementSystem,       Phase::Update,           SystemOrder::Last)
            // Collision response — separation first, then projectile hits
            // (despawn the bolt before contact damage looks at the pair),
            // then contact damage, then death
            .add_system(SeparationSystem,     Phase::CollisionResolve, SystemOrder::First)
            .add_system(ProjectileHitSystem,  Phase::CollisionResolve, SystemOrder::before::<ContactDamageSystem>())
            .add_system(ContactDamageSystem,  Phase::CollisionResolve, SystemOrder::Default)
            .add_system(DeathSystem::new(),   Phase::CollisionResolve, SystemOrder::after::<ContactDamageSystem>());
    }
}
