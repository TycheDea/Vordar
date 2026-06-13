// Projectile flight + hits (Phase 7.5).
//
// Projectiles are ordinary replicated entities: the prefab supplies visuals
// and a small Hitbox (never Solid), code attaches Projectile + Velocity at
// spawn. Damage is plain collision — no rewind, favor-the-shooter — which is
// fine for slow, dodgeable bolts; the scheduled-snapshot Mechanic path stays
// the model for anything that needs "position at T" fairness.

use crate::enemies::{Enemy, Provoked};
use crate::player::Player;
use engine_app::events::{CollisionStarted, EventBus};
use engine_app::scheduler::System;
use engine_core::components::{Health, Velocity};
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use glam::Vec3;
use hecs::Entity;
use std::collections::HashSet;

/// A projectile in flight. Code-inserted at spawn (the caster entity can't
/// come from RON); the prefab supplies visuals + Hitbox (and no Solid, so
/// projectiles never push or get pushed).
pub struct Projectile {
    pub damage: i32,
    /// Skipped by the hit test (you can't shoot yourself).
    pub caster: hecs::Entity,
    /// Seconds of flight left; despawned at zero.
    pub ttl: f32,
    /// Side rule: true = fired by an enemy, damages only players;
    /// false = fired by a player, damages only enemies.
    pub hits_players: bool,
}

/// Spawn a projectile immediately: prefab visuals + code-attached flight
/// state. `dir` must be unit length on the XZ plane.
pub fn spawn_projectile(
    world: &mut World,
    resources: &mut Resources,
    prefab: &str,
    origin: Vec3,
    dir: Vec3,
    speed: f32,
    damage: i32,
    ttl: f32,
    caster: Entity,
    hits_players: bool,
) -> Option<Entity> {
    let entity = match spawn_prefab(prefab, origin, &mut SpawnContext { world, resources }) {
        Ok(e) => e,
        Err(e) => {
            log::error!("projectile spawn '{prefab}' failed: {e}");
            return None;
        }
    };
    if let Ok(mut velocity) = world.get::<&mut Velocity>(entity) {
        velocity.linear = dir * speed;
    }
    let _ = world.insert_one(entity, Projectile { damage, caster, ttl, hits_players });
    Some(entity)
}

/// Counts a projectile's flight time down and despawns it at zero.
pub struct ProjectileTtlSystem;

impl System for ProjectileTtlSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let mut expired: Vec<Entity> = Vec::new();
        for (entity, projectile) in world.query::<(Entity, &mut Projectile)>().iter() {
            if projectile.ttl <= 0.0 {
                continue; // already queued; fixed phases can step twice before a flush
            }
            projectile.ttl -= delta;
            if projectile.ttl <= 0.0 {
                expired.push(entity);
            }
        }
        let queue = resources.get_mut::<DespawnQueue>().unwrap();
        for entity in expired {
            queue.push(entity, None);
        }
    }
}

/// Resolves projectile contacts: damage the right side, provoke enemies,
/// despawn the projectile. Wrong-side contacts pass through (a bolt grazing
/// an ally keeps flying), so there is no PvP and no enemy friendly fire.
pub struct ProjectileHitSystem;

impl System for ProjectileHitSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        struct Hit {
            projectile: Entity,
            victim: Entity,
            damage: i32,
            provoke: bool,
        }

        let mut hits: Vec<Hit> = Vec::new();
        // One projectile can collide with several entities the same tick —
        // only the first valid contact lands.
        let mut spent: HashSet<Entity> = HashSet::new();
        {
            let bus = resources.get::<EventBus>().expect("EventBus not in resources");
            for e in bus.read::<CollisionStarted>() {
                for (this, other) in [(e.a, e.b), (e.b, e.a)] {
                    let Ok(projectile) = world.get::<&Projectile>(this) else { continue };
                    if spent.contains(&this)
                        || other == projectile.caster
                        || world.get::<&Projectile>(other).is_ok()
                    {
                        continue;
                    }
                    let valid = if projectile.hits_players {
                        world.get::<&Player>(other).is_ok()
                    } else {
                        world.get::<&Enemy>(other).is_ok()
                    };
                    if !valid {
                        continue;
                    }
                    spent.insert(this);
                    hits.push(Hit {
                        projectile: this,
                        victim: other,
                        damage: projectile.damage,
                        provoke: !projectile.hits_players,
                    });
                }
            }
        }

        for hit in &hits {
            if let Ok(mut health) = world.get::<&mut Health>(hit.victim) {
                health.current -= hit.damage;
            }
            if hit.provoke {
                let _ = world.insert_one(hit.victim, Provoked);
            }
        }
        let queue = resources.get_mut::<DespawnQueue>().unwrap();
        for hit in hits {
            queue.push(hit.projectile, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::components::Transform;

    const DT: f32 = 1.0 / 60.0;

    fn base_resources() -> Resources {
        let mut resources = Resources::new();
        resources.insert(DespawnQueue::new());
        resources.insert(EventBus::new());
        resources
    }

    fn despawn_count(resources: &Resources) -> usize {
        resources.get::<DespawnQueue>().unwrap().0.len()
    }

    fn idle_enemy() -> Enemy {
        Enemy { speed: 2.0, aggro_range: 0.0, attack: Default::default(), cooldown_left: 0.0 }
    }

    #[test]
    fn ttl_expiry_queues_despawn() {
        let mut world = World::new();
        let mut resources = base_resources();
        let caster = world.spawn((Player { speed: 6.0 },));
        world.spawn((Projectile { damage: 5, caster, ttl: 0.5, hits_players: false },));

        for _ in 0..29 {
            ProjectileTtlSystem.run(&mut world, &mut resources, DT);
        }
        assert_eq!(despawn_count(&resources), 0, "still flying");
        ProjectileTtlSystem.run(&mut world, &mut resources, DT);
        ProjectileTtlSystem.run(&mut world, &mut resources, DT);
        assert_eq!(despawn_count(&resources), 1, "expired exactly once");
    }

    #[test]
    fn player_bolt_damages_provokes_and_despawns() {
        let mut world = World::new();
        let mut resources = base_resources();
        let caster = world.spawn((Player { speed: 6.0 },));
        let enemy = world.spawn((
            Transform::new(Vec3::ZERO),
            idle_enemy(),
            Health { current: 30, max: 30 },
        ));
        let bolt = world.spawn((Projectile { damage: 12, caster, ttl: 1.0, hits_players: false },));

        resources.get_mut::<EventBus>().unwrap().emit(CollisionStarted { a: bolt, b: enemy });
        ProjectileHitSystem.run(&mut world, &mut resources, DT);

        assert_eq!(world.get::<&Health>(enemy).unwrap().current, 18);
        assert!(world.get::<&Provoked>(enemy).is_ok(), "hit must provoke");
        assert_eq!(despawn_count(&resources), 1, "bolt spent");
    }

    #[test]
    fn caster_is_immune_to_own_projectile() {
        let mut world = World::new();
        let mut resources = base_resources();
        let caster = world.spawn((
            Player { speed: 6.0 },
            Health { current: 100, max: 100 },
        ));
        let bolt = world.spawn((Projectile { damage: 12, caster, ttl: 1.0, hits_players: true },));

        resources.get_mut::<EventBus>().unwrap().emit(CollisionStarted { a: caster, b: bolt });
        ProjectileHitSystem.run(&mut world, &mut resources, DT);

        assert_eq!(world.get::<&Health>(caster).unwrap().current, 100);
        assert_eq!(despawn_count(&resources), 0, "passes through its caster");
    }

    #[test]
    fn wrong_side_contact_passes_through() {
        let mut world = World::new();
        let mut resources = base_resources();
        let imp = world.spawn((idle_enemy(),));
        // Enemy-fired bolt grazing another enemy: no friendly fire, keeps flying.
        let other_enemy = world.spawn((idle_enemy(), Health { current: 80, max: 80 }));
        let bolt = world.spawn((Projectile { damage: 8, caster: imp, ttl: 1.0, hits_players: true },));

        resources.get_mut::<EventBus>().unwrap().emit(CollisionStarted { a: bolt, b: other_enemy });
        ProjectileHitSystem.run(&mut world, &mut resources, DT);

        assert_eq!(world.get::<&Health>(other_enemy).unwrap().current, 80);
        assert_eq!(despawn_count(&resources), 0);
    }

    #[test]
    fn one_projectile_lands_once_across_multiple_pairs() {
        let mut world = World::new();
        let mut resources = base_resources();
        let caster = world.spawn((Player { speed: 6.0 },));
        let e1 = world.spawn((idle_enemy(), Health { current: 30, max: 30 }));
        let e2 = world.spawn((idle_enemy(), Health { current: 30, max: 30 }));
        let bolt = world.spawn((Projectile { damage: 12, caster, ttl: 1.0, hits_players: false },));

        let bus = resources.get_mut::<EventBus>().unwrap();
        bus.emit(CollisionStarted { a: bolt, b: e1 });
        bus.emit(CollisionStarted { a: bolt, b: e2 });
        ProjectileHitSystem.run(&mut world, &mut resources, DT);

        let h1 = world.get::<&Health>(e1).unwrap().current;
        let h2 = world.get::<&Health>(e2).unwrap().current;
        assert_eq!(h1 + h2, 48, "exactly one of the two takes the hit");
        assert_eq!(despawn_count(&resources), 1);
    }
}
