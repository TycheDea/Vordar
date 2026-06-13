// Enemies — the Enemy component, the shared engagement model, and the AI
// system. WHAT an engaged enemy does is per-archetype: see behavior.rs (the
// trait + data-driven default) and each chapter's enemies/ modules (the
// archetype files that own custom behaviors).

pub mod behavior;

pub use behavior::{Action, BehaviorCtx, BehaviorRegistry, EnemyBehavior, FireOrder};

use crate::combat::projectile::spawn_projectile;
use crate::player::Player;
use behavior::DATA_DRIVEN;
use engine_app::scheduler::System;
use engine_core::components::{Transform, Velocity};
use engine_core::prefab::PrefabId;
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;

/// Marker + behavior profile for enemy entities (Phase 7.5).
///
/// Engagement model: an enemy ENGAGES (chases / attacks) when it is Provoked
/// (took damage) or a player stands within `aggro_range`. `aggro_range: 0`
/// = passive — it only ever retaliates. Idle enemies stand still.
#[derive(serde::Deserialize)]
pub struct Enemy {
    pub speed: f32,
    /// Engage when a player is within this range; 0 = passive.
    #[serde(default)]
    pub aggro_range: f32,
    #[serde(default)]
    pub attack: AttackKind,
    /// Runtime ranged-attack cooldown accumulator — never authored in RON.
    #[serde(skip, default)]
    pub cooldown_left: f32,
}

/// How an engaged enemy deals damage (the data-driven behavior profile).
#[derive(serde::Deserialize, Default)]
pub enum AttackKind {
    /// Walk into the target — damage via the existing ContactDamage path.
    #[default]
    Melee,
    /// Keep `range`, fire `prefab` projectiles every `cooldown` seconds.
    Ranged { prefab: String, speed: f32, damage: i32, cooldown: f32, range: f32 },
}

/// Permanent aggro marker — inserted when the entity takes targeted damage
/// (projectile or mechanic). Makes passive enemies retaliate. Code-only.
pub struct Provoked;

/// Engagement-driven enemy AI (Phase 7.5).
///
/// Enemies live in the world and IDLE until engaged: either a player walks
/// into `aggro_range` (aggressive types) or the enemy takes targeted damage
/// (`Provoked` — the only way to wake a passive type). The engaged action is
/// delegated to the archetype's behavior (BehaviorRegistry by prefab name;
/// data-driven default). Deterministic: no clocks, no RNG; nearest-player
/// selection via total_cmp (ties resolve by stable query order).
pub struct EnemyAISystem;

struct PendingShot {
    order: FireOrder,
    origin: Vec3,
    caster: hecs::Entity,
}

impl System for EnemyAISystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        // Collect player positions first to avoid holding a world borrow during
        // enemy iteration.
        let players: Vec<Vec3> = world
            .query::<(&Transform, &Player)>()
            .iter()
            .map(|(t, _)| t.position)
            .collect();

        let mut shots: Vec<PendingShot> = Vec::new();
        {
            let registry = resources.get::<BehaviorRegistry>();
            for (entity, transform, velocity, enemy, prefab) in world
                .query::<(hecs::Entity, &Transform, &mut Velocity, &mut Enemy, Option<&PrefabId>)>()
                .iter()
            {
                // Cooldown winds down even while idle — a sentinel that lost its
                // target doesn't fire the instant it re-engages.
                enemy.cooldown_left = (enemy.cooldown_left - delta).max(0.0);

                let Some(target) = players.iter().copied().min_by(|a, b| {
                    a.distance_squared(transform.position)
                        .total_cmp(&b.distance_squared(transform.position))
                }) else {
                    velocity.linear = Vec3::ZERO;
                    continue;
                };
                let dist = target.distance(transform.position);

                let provoked = world.get::<&Provoked>(entity).is_ok();
                let engaged = provoked || (enemy.aggro_range > 0.0 && dist <= enemy.aggro_range);
                if !engaged {
                    velocity.linear = Vec3::ZERO;
                    continue;
                }

                let behavior = match (registry, prefab) {
                    (Some(reg), Some(id)) => reg.get(&id.0),
                    _ => &DATA_DRIVEN,
                };
                let position = transform.position;
                let action = behavior.engaged(&mut BehaviorCtx { enemy, position, target, dt: delta });
                match action {
                    Action::Hold => velocity.linear = Vec3::ZERO,
                    Action::Move(v) => velocity.linear = v,
                    Action::Fire(order) => {
                        velocity.linear = Vec3::ZERO;
                        shots.push(PendingShot { order, origin: position, caster: entity });
                    }
                }
            }
        }

        for shot in shots {
            spawn_projectile(
                world,
                resources,
                &shot.order.prefab,
                shot.origin + shot.order.dir * 0.9,
                shot.order.dir,
                shot.order.speed,
                shot.order.damage,
                shot.order.ttl,
                shot.caster,
                true, // enemy-fired: damages players only
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::Projectile;
    use engine_core::prefab::{register_core_components, ComponentRegistry, PrefabDef, PrefabLibrary};
    use engine_core::traits::DespawnQueue;

    const DT: f32 = 1.0 / 60.0;

    fn resources_with_test_bolt() -> Resources {
        let mut registry = ComponentRegistry::new();
        register_core_components(&mut registry);
        let mut library = PrefabLibrary::new();
        library.insert(
            "test_bolt",
            ron::from_str::<PrefabDef>(r#"(components: { "Transform": (), "Velocity": () })"#).unwrap(),
        );
        let mut resources = Resources::new();
        resources.insert(registry);
        resources.insert(library);
        resources.insert(DespawnQueue::new());
        resources
    }

    fn spawn_player(world: &mut World, pos: Vec3) -> hecs::Entity {
        world.spawn((Transform::new(pos), Player { speed: 6.0 }))
    }

    fn spawn_enemy(world: &mut World, pos: Vec3, aggro_range: f32, attack: AttackKind) -> hecs::Entity {
        world.spawn((
            Transform::new(pos),
            Velocity { linear: Vec3::ZERO },
            Enemy { speed: 2.0, aggro_range, attack, cooldown_left: 0.0 },
        ))
    }

    fn velocity_of(world: &World, e: hecs::Entity) -> Vec3 {
        world.get::<&Velocity>(e).unwrap().linear
    }

    #[test]
    fn passive_enemy_ignores_nearby_player() {
        let mut world = World::new();
        let mut resources = Resources::new();
        spawn_player(&mut world, Vec3::new(2.0, 0.0, 0.0));
        let enemy = spawn_enemy(&mut world, Vec3::ZERO, 0.0, AttackKind::Melee);
        EnemyAISystem.run(&mut world, &mut resources, DT);
        assert_eq!(velocity_of(&world, enemy), Vec3::ZERO);
    }

    #[test]
    fn aggressive_enemy_engages_inside_aggro_range_only() {
        let mut world = World::new();
        let mut resources = Resources::new();
        spawn_player(&mut world, Vec3::new(20.0, 0.0, 0.0));
        let enemy = spawn_enemy(&mut world, Vec3::ZERO, 10.0, AttackKind::Melee);
        EnemyAISystem.run(&mut world, &mut resources, DT);
        assert_eq!(velocity_of(&world, enemy), Vec3::ZERO, "player at 20 > aggro 10");

        world.get::<&mut Transform>(enemy).unwrap().position = Vec3::new(14.0, 0.0, 0.0);
        EnemyAISystem.run(&mut world, &mut resources, DT);
        let v = velocity_of(&world, enemy);
        assert!(v.x > 0.0, "must chase toward the player, got {v}");
    }

    #[test]
    fn provoked_overrides_passivity() {
        let mut world = World::new();
        let mut resources = Resources::new();
        spawn_player(&mut world, Vec3::new(5.0, 0.0, 0.0));
        let enemy = spawn_enemy(&mut world, Vec3::ZERO, 0.0, AttackKind::Melee);
        world.insert_one(enemy, Provoked).unwrap();
        EnemyAISystem.run(&mut world, &mut resources, DT);
        assert!(velocity_of(&world, enemy).x > 0.0, "provoked passive must retaliate");
    }

    #[test]
    fn ranged_enemy_holds_range_and_fires_once_per_cooldown() {
        let mut world = World::new();
        let mut resources = resources_with_test_bolt();
        spawn_player(&mut world, Vec3::new(5.0, 0.0, 0.0));
        let attack = AttackKind::Ranged {
            prefab: "test_bolt".into(),
            speed: 10.0,
            damage: 5,
            cooldown: 0.5,
            range: 8.0,
        };
        let enemy = spawn_enemy(&mut world, Vec3::ZERO, 12.0, attack);

        fn projectiles(world: &mut World) -> usize {
            world.query::<&Projectile>().iter().count()
        }

        EnemyAISystem.run(&mut world, &mut resources, DT);
        assert_eq!(velocity_of(&world, enemy), Vec3::ZERO, "inside range: stop and shoot");
        assert_eq!(projectiles(&mut world), 1, "first engaged tick fires");

        EnemyAISystem.run(&mut world, &mut resources, DT);
        assert_eq!(projectiles(&mut world), 1, "cooldown gates the second shot");

        // Burn through the 0.5 s cooldown.
        for _ in 0..31 {
            EnemyAISystem.run(&mut world, &mut resources, DT);
        }
        assert_eq!(projectiles(&mut world), 2, "fires again after the cooldown");
    }

    #[test]
    fn ranged_enemy_approaches_until_in_range() {
        let mut world = World::new();
        let mut resources = resources_with_test_bolt();
        spawn_player(&mut world, Vec3::new(20.0, 0.0, 0.0));
        let attack = AttackKind::Ranged {
            prefab: "test_bolt".into(),
            speed: 10.0,
            damage: 5,
            cooldown: 0.5,
            range: 8.0,
        };
        let enemy = spawn_enemy(&mut world, Vec3::ZERO, 0.0, attack);
        world.insert_one(enemy, Provoked).unwrap();
        EnemyAISystem.run(&mut world, &mut resources, DT);
        assert!(velocity_of(&world, enemy).x > 0.0, "out of range: close the gap");
        assert_eq!(world.query::<&Projectile>().iter().count(), 0);
    }

    /// A registered custom behavior overrides the data-driven default —
    /// pins the per-archetype divergence seam.
    #[test]
    fn registry_behavior_overrides_data_driven_default() {
        struct Coward;
        impl EnemyBehavior for Coward {
            fn engaged(&self, ctx: &mut BehaviorCtx) -> Action {
                let away = (ctx.position - ctx.target).normalize_or_zero();
                Action::Move(away * ctx.enemy.speed)
            }
        }

        let mut world = World::new();
        let mut resources = Resources::new();
        let mut registry = BehaviorRegistry::default();
        registry.register("coward", Coward);
        resources.insert(registry);

        spawn_player(&mut world, Vec3::new(5.0, 0.0, 0.0));
        let enemy = spawn_enemy(&mut world, Vec3::ZERO, 10.0, AttackKind::Melee);
        world.insert_one(enemy, PrefabId("coward".into())).unwrap();

        EnemyAISystem.run(&mut world, &mut resources, DT);
        assert!(
            velocity_of(&world, enemy).x < 0.0,
            "custom behavior must run instead of the melee chase"
        );
    }
}
