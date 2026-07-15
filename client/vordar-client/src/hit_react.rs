// Damage/death reactions — client-only presentation for "something got hurt":
//   • HitReactSystem watches every Health for decreases (works identically in
//     the sandbox, where the sim runs locally, and online once snapshots carry
//     hp) → impact sparks + a Hit_A flinch that never interrupts an attack.
//   • CorpseOnDeathSystem catches HealthDepleted in Phase::DespawnFlush First —
//     the sim despawns dead entities the same tick, so this is the last moment
//     the dying entity's mesh/transform exist. It spawns a cosmetic corpse that
//     plays the death clip once, holds the final frame, and fades out of the
//     world after a short TTL.

use crate::locomotion::{AnimController, LocomotionClips};
use crate::presentation::HudHidden;
use crate::vfx::{self, ParticleSim};
use engine_app::events::{EventBus, HealthDepleted};
use engine_app::scheduler::System;
use engine_core::components::{AnimationPlayer, Health, RenderMesh, RenderShape, ShapeGroup, Transform};
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use glam::Vec3;
use hecs::Entity;
use vordar_game::class::ClassId;

/// How long a hit flinch latches locomotion (shorter than an attack latch —
/// getting tagged shouldn't root you visibly).
const HIT_LATCH: f32 = 0.35;
const HIT_BLEND: f32 = 0.06;
/// Corpse lifetime before it sinks out of the world.
const CORPSE_SECS: f32 = 2.5;
/// Impact sparks per point of damage, clamped.
const IMPACT_MIN: usize = 4;
const IMPACT_MAX: usize = 24;

/// Last-seen health, attached lazily to everything with a `Health`.
pub struct HealthWatch {
    pub last: i32,
}

/// The dying/damaged entity's VFX color: its class tint when it has a tinted
/// class, else its body color, else white.
fn entity_color(
    class: Option<&str>,
    shape_color: Option<Vec3>,
    resources: &Resources,
) -> Vec3 {
    class
        .and_then(|c| {
            resources
                .get::<vordar_game::class::ClassLibrary>()
                .and_then(|lib| lib.class(c).and_then(|d| d.tint))
        })
        .or(shape_color)
        .unwrap_or(Vec3::ONE)
}

/// Watches `Health` decreases → impact burst + hit-react flinch. Runs in
/// Phase::RenderSync before LocomotionSystem so the flinch clip wins the
/// frame it lands. Lethal damage is handled by CorpseOnDeathSystem instead
/// (the entity is despawned before RenderSync ever sees hp ≤ 0).
pub struct HitReactSystem;

impl System for HitReactSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        struct Hit {
            entity: Entity,
            pos: Vec3,
            damage: i32,
            class: Option<String>,
            shape_color: Option<Vec3>,
        }
        let mut hits: Vec<Hit> = Vec::new();
        let mut unwatched: Vec<(Entity, i32)> = Vec::new();

        for (entity, health, watch, transform, class, shape, group) in world
            .query::<(
                Entity,
                &Health,
                Option<&mut HealthWatch>,
                &Transform,
                Option<&ClassId>,
                Option<&RenderShape>,
                Option<&ShapeGroup>,
            )>()
            .iter()
        {
            let Some(watch) = watch else {
                unwatched.push((entity, health.current));
                continue;
            };
            if health.current < watch.last {
                hits.push(Hit {
                    entity,
                    pos: transform.position + Vec3::Y,
                    damage: watch.last - health.current,
                    class: class.map(|c| c.id.clone()),
                    shape_color: shape
                        .map(|s| s.color)
                        .or_else(|| group.and_then(|g| g.shapes.first().map(|s| s.color))),
                });
            }
            watch.last = health.current;
        }

        for (entity, current) in unwatched {
            let _ = world.insert_one(entity, HealthWatch { last: current });
        }

        for hit in hits {
            // Flinch — but never cancel an attack in progress, and never a corpse.
            if let (Ok(clips), Ok(mut ctrl)) = (
                world.get::<&LocomotionClips>(hit.entity),
                world.get::<&mut AnimController>(hit.entity),
            ) {
                if !ctrl.dead && ctrl.oneshot <= 0.0 && !clips.hit.is_empty() {
                    ctrl.oneshot = HIT_LATCH;
                    if let Ok(mut player) = world.get::<&mut AnimationPlayer>(hit.entity) {
                        player.transition_to(&clips.hit, false, HIT_BLEND);
                    }
                }
            }
            let color = entity_color(hit.class.as_deref(), hit.shape_color, resources);
            let count = (hit.damage as usize).clamp(IMPACT_MIN, IMPACT_MAX);
            if let Some(sim) = resources.get_mut::<ParticleSim>() {
                sim.burst(hit.pos, color, count, vfx::IMPACT_SPEED, vfx::IMPACT_SIZE);
            }
        }
    }
}

/// A cosmetic corpse's remaining seconds.
pub struct CorpseTtl(pub f32);

/// Spawn the corpse for a freshly-dead mesh character: cloned pose + mesh,
/// death clip played once (non-looping holds the last frame). Shared by the
/// sandbox death path and the networked EntityDied path.
pub fn spawn_corpse(world: &mut World, transform: Transform, mesh: RenderMesh, death_clip: &str) {
    world.spawn((
        transform,
        mesh,
        AnimationPlayer {
            clip: death_clip.to_owned(),
            looping: false,
            ..AnimationPlayer::default()
        },
        CorpseTtl(CORPSE_SECS),
        HudHidden,
    ));
}

/// Runs in Phase::DespawnFlush, SystemOrder::First — after DeathSystem emitted
/// HealthDepleted (CollisionResolve) but before DespawnFlushSystem removes the
/// entity. Reads the dying entity's visual one last time: death burst for
/// everything, plus a corpse for mesh characters.
pub struct CorpseOnDeathSystem;

impl System for CorpseOnDeathSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        struct Death {
            pos: Vec3,
            class: Option<String>,
            shape_color: Option<Vec3>,
            corpse: Option<(Transform, RenderMesh, String)>,
        }
        let dead: Vec<Entity> = resources
            .get::<EventBus>()
            .map(|bus| bus.read::<HealthDepleted>().map(|e| e.entity).collect())
            .unwrap_or_default();
        if dead.is_empty() {
            return;
        }

        let mut deaths: Vec<Death> = Vec::new();
        for entity in dead {
            let Ok(transform) = world.get::<&Transform>(entity).map(|t| Transform::clone(&t)) else {
                continue;
            };
            let corpse = match (
                world.get::<&RenderMesh>(entity),
                world.get::<&LocomotionClips>(entity),
            ) {
                (Ok(mesh), Ok(clips)) if !clips.death.is_empty() => {
                    Some((transform.clone(), RenderMesh::clone(&mesh), clips.death.clone()))
                }
                _ => None,
            };
            deaths.push(Death {
                pos: transform.position + Vec3::Y,
                class: world.get::<&ClassId>(entity).ok().map(|c| c.id.clone()),
                shape_color: world
                    .get::<&RenderShape>(entity)
                    .ok()
                    .map(|s| s.color)
                    .or_else(|| {
                        world
                            .get::<&ShapeGroup>(entity)
                            .ok()
                            .and_then(|g| g.shapes.first().map(|s| s.color))
                    }),
                corpse,
            });
        }

        for death in deaths {
            let color = entity_color(death.class.as_deref(), death.shape_color, resources);
            if let Some(sim) = resources.get_mut::<ParticleSim>() {
                sim.burst(death.pos, color, vfx::DEATH_COUNT, vfx::DEATH_SPEED, vfx::DEATH_SIZE);
            }
            if let Some((transform, mesh, clip)) = death.corpse {
                spawn_corpse(world, transform, mesh, &clip);
            }
        }
    }
}

/// Despawns corpses when their TTL runs out (Phase::Update).
pub struct CorpseTtlSystem;

impl System for CorpseTtlSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let mut expired: Vec<Entity> = Vec::new();
        for (entity, ttl) in world.query::<(Entity, &mut CorpseTtl)>().iter() {
            ttl.0 -= delta;
            if ttl.0 <= 0.0 {
                expired.push(entity);
            }
        }
        if expired.is_empty() {
            return;
        }
        let queue = resources.get_mut::<DespawnQueue>().unwrap();
        for entity in expired {
            queue.push(entity, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::traits::DespawnQueue;

    fn base_resources() -> Resources {
        let mut resources = Resources::new();
        resources.insert(ParticleSim::new());
        resources.insert(EventBus::new());
        resources.insert(DespawnQueue::new());
        resources
    }

    fn animated_bundle() -> (LocomotionClips, AnimController, AnimationPlayer) {
        (
            LocomotionClips { hit: "Hit_A".into(), death: "Death_A".into(), ..Default::default() },
            AnimController::default(),
            AnimationPlayer::default(),
        )
    }

    #[test]
    fn health_drop_plays_hit_clip_and_bursts() {
        let mut world = World::new();
        let mut resources = base_resources();
        let (clips, ctrl, player) = animated_bundle();
        let e = world.spawn((Transform::default(), Health::new(100), clips, ctrl, player));

        let mut sys = HitReactSystem;
        sys.run(&mut world, &mut resources, 0.016); // attaches the watch
        world.get::<&mut Health>(e).unwrap().current = 90;
        sys.run(&mut world, &mut resources, 0.016);

        assert_eq!(world.get::<&AnimationPlayer>(e).unwrap().clip, "Hit_A");
        assert!(world.get::<&AnimController>(e).unwrap().oneshot > 0.0, "flinch latched");
        assert!(
            !resources.get::<ParticleSim>().unwrap().particles.is_empty(),
            "impact burst spawned"
        );

        // Same health → no re-trigger.
        let n = resources.get::<ParticleSim>().unwrap().particles.len();
        sys.run(&mut world, &mut resources, 0.016);
        assert_eq!(resources.get::<ParticleSim>().unwrap().particles.len(), n);
    }

    #[test]
    fn hit_react_never_cancels_an_attack_in_progress() {
        let mut world = World::new();
        let mut resources = base_resources();
        let (clips, mut ctrl, mut player) = animated_bundle();
        ctrl.oneshot = 0.5; // mid-attack
        player.clip = "1H_Melee_Attack_Chop".into();
        let e = world.spawn((Transform::default(), Health::new(100), clips, ctrl, player));

        let mut sys = HitReactSystem;
        sys.run(&mut world, &mut resources, 0.016);
        world.get::<&mut Health>(e).unwrap().current = 80;
        sys.run(&mut world, &mut resources, 0.016);

        assert_eq!(
            world.get::<&AnimationPlayer>(e).unwrap().clip,
            "1H_Melee_Attack_Chop",
            "attack keeps playing"
        );
        assert!(
            !resources.get::<ParticleSim>().unwrap().particles.is_empty(),
            "sparks still fly"
        );
    }

    #[test]
    fn shape_only_entity_bursts_without_animation() {
        let mut world = World::new();
        let mut resources = base_resources();
        let e = world.spawn((
            Transform::default(),
            Health::new(25),
            RenderShape {
                shape: engine_core::components::RenderShapeType::Sphere,
                color: Vec3::new(1.0, 0.4, 0.1),
            },
        ));

        let mut sys = HitReactSystem;
        sys.run(&mut world, &mut resources, 0.016);
        world.get::<&mut Health>(e).unwrap().current = 10;
        sys.run(&mut world, &mut resources, 0.016);

        let sim = resources.get::<ParticleSim>().unwrap();
        assert!(!sim.particles.is_empty());
        assert!(
            sim.particles[0].color.abs_diff_eq(Vec3::new(1.0, 0.4, 0.1), 1e-6),
            "burst inherits the body color"
        );
    }

    #[test]
    fn death_event_spawns_corpse_holding_death_clip() {
        let mut world = World::new();
        let mut resources = base_resources();
        let (clips, ctrl, player) = animated_bundle();
        let e = world.spawn((
            Transform::default(),
            Health { current: 0, max: 100 },
            RenderMesh { asset: "content/models/human.glb".into(), tint: Vec3::ONE },
            clips,
            ctrl,
            player,
        ));
        resources.get_mut::<EventBus>().unwrap().emit(HealthDepleted { entity: e });

        CorpseOnDeathSystem.run(&mut world, &mut resources, 0.016);

        let corpse = world
            .query::<(Entity, &CorpseTtl)>()
            .iter()
            .map(|(c, _)| c)
            .next()
            .expect("corpse spawned");
        let player = world.get::<&AnimationPlayer>(corpse).unwrap();
        assert_eq!(player.clip, "Death_A");
        assert!(!player.looping, "death clip holds its last frame");
        assert!(world.get::<&RenderMesh>(corpse).is_ok(), "corpse keeps the mesh");
        assert!(
            !resources.get::<ParticleSim>().unwrap().particles.is_empty(),
            "death burst spawned"
        );
    }

    #[test]
    fn corpse_ttl_expires_into_the_despawn_queue() {
        let mut world = World::new();
        let mut resources = base_resources();
        world.spawn((Transform::default(), CorpseTtl(0.05)));

        let mut sys = CorpseTtlSystem;
        sys.run(&mut world, &mut resources, 0.016);
        assert!(resources.get::<DespawnQueue>().unwrap().0.is_empty(), "still alive");
        sys.run(&mut world, &mut resources, 0.1);
        assert_eq!(resources.get::<DespawnQueue>().unwrap().0.len(), 1, "queued at ttl 0");
    }
}
