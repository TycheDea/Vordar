/// Telegraph visuals: scheduled-ability indicators that count down to the
/// mechanic's resolve time. Purely client-local — never in the replication map;
/// despawns at T. Fill is a pure function of synced server time vs resolve_at
/// (DESIGN.md §3) — zero per-frame network updates, and the visual completes
/// exactly at T (the hit-test moment) on every client.

use engine_app::scheduler::System;
use engine_core::components::{RenderShape, Transform};
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use glam::Vec3;
use hecs::Entity;

use crate::net::NetClientState;

/// A telegraph visual: counts down to the mechanic's resolve time. Purely
/// client-local — never in the replication map; despawns itself at T.
pub(crate) struct TelegraphVisual {
    pub(crate) resolve_at_micros: u64,
    pub(crate) duration_micros: u64,
}

const TELEGRAPH_DIM: Vec3 = Vec3::new(0.45, 0.08, 0.08);
// Components above 1.0 are HDR emissive (VQ-C3): an about-to-resolve
// telegraph blooms threat red-orange (VQ-A4).
const TELEGRAPH_BRIGHT: Vec3 = Vec3::new(2.2, 0.45, 0.15);

pub(crate) fn spawn_telegraph(
    world: &mut World,
    resources: &mut Resources,
    prefab: &str,
    pos: Vec3,
    radius: f32,
    resolve_at_micros: u64,
    duration_micros: u64,
) {
    match spawn_prefab(prefab, pos, &mut SpawnContext { world, resources }) {
        Ok(entity) => {
            if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
                transform.scale = Vec3::new(radius * 2.0, 0.1, radius * 2.0);
            }
            let _ = world.insert_one(entity, TelegraphVisual { resolve_at_micros, duration_micros });
        }
        Err(e) => log::error!("telegraph spawn '{prefab}' failed: {e}"),
    }
}

/// Animates telegraph fill as a PURE FUNCTION of synced server time vs
/// resolve_at (DESIGN.md §3) — zero per-frame network updates, and the visual
/// completes exactly at T (the hit-test moment) on every client. Runs once
/// per display frame so the fill is smooth at any refresh rate.
pub struct TelegraphFillSystem;

impl System for TelegraphFillSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let Some(now) = resources.get::<NetClientState>().unwrap().server_now_micros() else {
            return;
        };
        let mut finished: Vec<Entity> = Vec::new();
        for (entity, telegraph, shape) in world.query::<(Entity, &TelegraphVisual, &mut RenderShape)>().iter() {
            if now >= telegraph.resolve_at_micros {
                finished.push(entity);
                continue;
            }
            let remaining = (telegraph.resolve_at_micros - now) as f32;
            let fill = 1.0 - (remaining / telegraph.duration_micros as f32).clamp(0.0, 1.0);
            shape.color = TELEGRAPH_DIM.lerp(TELEGRAPH_BRIGHT, fill);
        }
        // The scheduled-ability impact beat (VQ-E1/E4): the resolve moment
        // pops threat-colored sparks + ground dust where the telegraph was.
        let impact_positions: Vec<Vec3> = finished
            .iter()
            .filter_map(|&e| world.get::<&Transform>(e).ok().map(|t| t.position))
            .collect();
        if let Some(sim) = resources.get_mut::<crate::vfx::ParticleSim>() {
            for pos in impact_positions {
                sim.burst_def(pos, &vordar_game::vfx::BurstDef {
                    count: 20,
                    speed: 4.5,
                    size:  0.13,
                    color: TELEGRAPH_BRIGHT,
                    cell:  1,
                    blend: vordar_game::vfx::ParticleBlend::Additive,
                    ttl: (0.25, 0.5),
                    gravity: -7.0,
                    drag: 2.5,
                    stretch: 0.0,
                });
                sim.burst_def(pos, &vordar_game::vfx::BurstDef {
                    count: 8,
                    speed: 1.6,
                    size:  0.35,
                    color: Vec3::new(0.35, 0.28, 0.24),
                    cell:  3,
                    blend: vordar_game::vfx::ParticleBlend::Alpha,
                    ttl: (0.6, 1.0),
                    gravity: -1.0,
                    drag: 1.5,
                    stretch: 0.0,
                });
            }
        }
        for entity in finished {
            resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
        }
    }
}
