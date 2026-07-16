// Client particle sim — cosmetic only, never replicated, never RON.
//
// Particles are plain structs in a resource (not entities): thousands of
// short-lived sparks would churn render slots and the despawn queue for no
// benefit. One system (`VfxSystem`, Phase::RenderSync) integrates them, emits
// trails for `VfxTrail` entities, and rebuilds the renderer's
// `ParticleDrawList` each display frame; the engine's additive billboard pass
// does the rest.
//
// All look-tuning constants live at the top of this file.

use engine_app::scheduler::System;
use engine_core::components::Transform;
use engine_core::traits::Resources;
use engine_core::World;
use engine_renderer::{ParticleDrawList, ParticleInstance, MAX_PARTICLES};
use glam::Vec3;
use hecs::Entity;
use std::collections::HashMap;
use vordar_game::vfx::{BurstDef, ParticleBlend, VfxLibrary, VfxTrail};

// ── Look tuning ─────────────────────────────────────────────────────────────

/// Cast burst: sparks per cast, launch speed, particle half-extent.
pub const CAST_COUNT: usize = 14;
pub const CAST_SPEED: f32 = 4.0;
pub const CAST_SIZE: f32 = 0.12;
/// Impact burst per point of damage (clamped) — see hit_react.rs.
pub const IMPACT_SIZE: f32 = 0.10;
pub const IMPACT_SPEED: f32 = 3.2;
/// Death burst.
pub const DEATH_COUNT: usize = 32;
pub const DEATH_SPEED: f32 = 5.0;
pub const DEATH_SIZE: f32 = 0.14;
/// Trail particles: lifetime and half-extent.
const TRAIL_TTL: f32 = 0.28;
const TRAIL_SIZE: f32 = 0.07;
/// Burst particle lifetime range (scaled by rng).
const BURST_TTL_MIN: f32 = 0.30;
const BURST_TTL_MAX: f32 = 0.55;
const GRAVITY: f32 = -7.0;
const DRAG: f32 = 2.5;

// ── Sim ─────────────────────────────────────────────────────────────────────

pub struct Particle {
    pub pos:     Vec3,
    pub vel:     Vec3,
    pub ttl:     f32,
    pub life:    f32,
    pub size:    f32,
    pub color:   Vec3,
    pub gravity: f32,
    pub drag:    f32,
    /// Atlas cell (VQ-E3): 0 soft glow, 1 core glow, 2 streak, 3 smoke.
    pub cell:    u32,
    pub blend:   ParticleBlend,
    /// Velocity-stretch factor (0 = round billboard) along `axis`.
    pub stretch: f32,
    /// World direction the stretched billboard elongates along (the emitter's
    /// velocity for trail motes, own velocity for sparks).
    pub axis:    Vec3,
}

/// Tiny deterministic RNG — cosmetic randomness needs no crate and no seeding
/// ceremony, just cheap decorrelated floats.
pub struct XorShift32(u32);

impl XorShift32 {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// The particle pool + rng, inserted as a resource in both client plugins.
pub struct ParticleSim {
    pub particles: Vec<Particle>,
    rng: XorShift32,
}

impl ParticleSim {
    pub fn new() -> Self {
        Self { particles: Vec::new(), rng: XorShift32::new(0x9e37_79b9) }
    }

    /// A radial spark burst: `count` particles fly out on the up-biased unit
    /// sphere at `speed` (jittered), shrinking and fading over their lifetime.
    /// The legacy tuned look — hot core-glow sparks.
    pub fn burst(&mut self, pos: Vec3, color: Vec3, count: usize, speed: f32, size: f32) {
        self.burst_def(pos, &BurstDef {
            count,
            speed,
            size,
            color,
            cell: 1, // core glow
            blend: ParticleBlend::Additive,
            ttl: (BURST_TTL_MIN, BURST_TTL_MAX),
            gravity: GRAVITY,
            drag: DRAG,
            stretch: 0.0,
        });
    }

    /// A data-driven burst — the cast/impact beats from `content/vfx` defs
    /// and projectile prefabs (VQ-E1).
    pub fn burst_def(&mut self, pos: Vec3, def: &BurstDef) {
        for _ in 0..def.count {
            if self.particles.len() >= MAX_PARTICLES {
                return;
            }
            let dir = hemisphere_dir(&mut self.rng);
            let jitter = 0.5 + 0.5 * self.rng.next_f32();
            let ttl = def.ttl.0 + (def.ttl.1 - def.ttl.0) * self.rng.next_f32();
            let vel = dir * def.speed * jitter;
            self.particles.push(Particle {
                pos,
                vel,
                ttl,
                life: ttl,
                size: def.size,
                color: def.color,
                gravity: def.gravity,
                drag: def.drag,
                cell: def.cell,
                blend: def.blend,
                stretch: def.stretch,
                axis: vel,
            });
        }
    }

    /// One trail mote (the travel beat). `vel` is the emitter's velocity —
    /// with `stretch > 0` the mote elongates along it (projectile streaks).
    pub fn trail(&mut self, pos: Vec3, color: Vec3, vel: Vec3, cell: u32, blend: ParticleBlend, stretch: f32) {
        if self.particles.len() >= MAX_PARTICLES {
            return;
        }
        let jitter = Vec3::new(
            self.rng.next_f32() - 0.5,
            self.rng.next_f32() - 0.5,
            self.rng.next_f32() - 0.5,
        ) * 0.4;
        self.particles.push(Particle {
            pos,
            vel: jitter + vel * 0.05, // motes barely inherit the emitter's motion
            ttl: TRAIL_TTL,
            life: TRAIL_TTL,
            size: TRAIL_SIZE,
            color,
            gravity: 0.0,
            drag: 0.0,
            cell,
            blend,
            stretch: if stretch > 0.0 && vel.length_squared() > 1e-4 { stretch } else { 0.0 },
            axis: vel,
        });
    }
}

/// Integrate one frame: velocity (with gravity + drag), position, lifetime.
/// Expired particles are removed.
pub fn step(particles: &mut Vec<Particle>, dt: f32) {
    for p in particles.iter_mut() {
        p.vel.y += p.gravity * dt;
        let damp = (1.0 - p.drag * dt).max(0.0);
        p.vel *= damp;
        p.pos += p.vel * dt;
        p.ttl -= dt;
    }
    particles.retain(|p| p.ttl > 0.0);
}

/// Convert live particles to GPU instances: fade (ttl/life) is premultiplied
/// into the color and shrinks the quad; alpha carries the fade for the
/// premultiplied-alpha variant. Additive particles fill the front of the
/// list (the renderer draws `[..additive_count]` additively, the rest with
/// alpha blending); additive is order-independent, but the alpha partition
/// is sorted back-to-front by distance from `eye` so overlapping alpha
/// particles composite correctly regardless of pool order. Stops at the
/// renderer's particle cap. Returns `additive_count`.
pub fn fill_draw_list(particles: &[Particle], eye: Vec3, out: &mut Vec<ParticleInstance>) -> usize {
    out.clear();
    let instance = |p: &Particle| {
        let fade = (p.ttl / p.life).clamp(0.0, 1.0);
        let rgb = p.color * fade;
        ParticleInstance {
            position: p.pos.to_array(),
            size:     p.size * fade,
            color:    [rgb.x, rgb.y, rgb.z, fade],
            stretch:  [p.axis.x, p.axis.y, p.axis.z, p.stretch],
            cell:     p.cell,
            _pad:     [0; 3],
        }
    };
    for p in particles.iter().filter(|p| p.blend == ParticleBlend::Additive).take(MAX_PARTICLES) {
        out.push(instance(p));
    }
    let additive_count = out.len();
    let mut alpha: Vec<&Particle> = particles.iter().filter(|p| p.blend == ParticleBlend::Alpha).collect();
    alpha.sort_by(|a, b| b.pos.distance_squared(eye).total_cmp(&a.pos.distance_squared(eye)));
    for p in alpha {
        if out.len() >= MAX_PARTICLES {
            break;
        }
        out.push(instance(p));
    }
    additive_count
}

/// Random unit vector biased upward (y ≥ ~-0.2) — sparks fly out and up.
fn hemisphere_dir(rng: &mut XorShift32) -> Vec3 {
    loop {
        let v = Vec3::new(
            rng.next_f32() * 2.0 - 1.0,
            rng.next_f32(),
            rng.next_f32() * 2.0 - 1.0,
        );
        let len_sq = v.length_squared();
        if len_sq > 1e-4 && len_sq <= 1.0 {
            return v / len_sq.sqrt();
        }
    }
}

// ── Gameplay hooks ──────────────────────────────────────────────────────────

/// The class's mesh tint — the color identity its ability VFX inherit
/// (ravager ember-red, wayfarer steel-cyan). White for untinted classes.
pub fn class_tint(resources: &Resources, class: &str) -> Vec3 {
    resources
        .get::<vordar_game::class::ClassLibrary>()
        .and_then(|lib| lib.class(class).and_then(|c| c.tint))
        .unwrap_or(Vec3::ONE)
}

/// The cast beat (VQ-E1), at the caster's weapon hand when the renderer
/// published a socket for it this frame (sockets are one frame stale from
/// Phase::Input — invisible at burst speeds), else at chest height. Uses the
/// ability's authored `content/vfx/<id>.ron` def; falls back to the legacy
/// tinted spark burst for unauthored abilities.
pub fn cast_burst(world: &World, resources: &mut Resources, entity: Entity, ability_id: &str, tint: Vec3) {
    let socket = resources.get::<engine_renderer::SocketTransforms>().and_then(|sockets| {
        sockets
            .0
            .get(&entity)
            .and_then(|bones| bones.get("handslot.r"))
            .map(|m| m.w_axis.truncate())
    });
    let pos = socket.or_else(|| {
        world
            .get::<&Transform>(entity)
            .ok()
            .map(|t| t.position + Vec3::Y * 1.1)
    });
    let Some(pos) = pos else { return };
    let def = resources
        .get::<VfxLibrary>()
        .and_then(|lib| lib.get(ability_id))
        .and_then(|d| d.cast.clone());
    if let Some(sim) = resources.get_mut::<ParticleSim>() {
        match def {
            Some(burst) => sim.burst_def(pos, &burst),
            None => sim.burst(pos, tint, CAST_COUNT, CAST_SPEED, CAST_SIZE),
        }
    }
}

/// The impact beat for projectiles (VQ-E1/E2): entities queued for despawn
/// that carry a `VfxTrail` with an authored impact burst where they died.
/// Register in Phase::DespawnFlush, SystemOrder::First (before the flush).
pub struct ImpactBurstSystem;

impl System for ImpactBurstSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let entities: Vec<Entity> = resources
            .get::<engine_core::traits::DespawnQueue>()
            .map(|q| q.0.iter().map(|(e, _)| *e).collect())
            .unwrap_or_default();
        if entities.is_empty() {
            return;
        }
        let mut bursts: Vec<(Vec3, BurstDef)> = Vec::new();
        for entity in entities {
            let (Ok(trail), Ok(transform)) =
                (world.get::<&VfxTrail>(entity), world.get::<&Transform>(entity))
            else {
                continue;
            };
            if let Some(impact) = &trail.impact {
                bursts.push((transform.position, impact.clone()));
            }
        }
        if let Some(sim) = resources.get_mut::<ParticleSim>() {
            for (pos, def) in bursts {
                sim.burst_def(pos, &def);
            }
        }
    }
}

// ── System ──────────────────────────────────────────────────────────────────

/// Steps the sim, emits `VfxTrail` particles at each emitter's interpolated
/// render position, and rebuilds the renderer's `ParticleDrawList`.
/// Phase::RenderSync, after MeshRenderSyncSystem (bursts spawned earlier this
/// frame still render this frame; trails use the same render positions the
/// meshes were drawn at).
pub struct VfxSystem {
    /// Fractional particles owed per trail emitter (rate × dt accumulator).
    accum: HashMap<Entity, f32>,
}

impl VfxSystem {
    pub fn new() -> Self {
        Self { accum: HashMap::new() }
    }
}

impl System for VfxSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let Some(mut sim) = resources.get_mut::<ParticleSim>().map(std::mem::take) else {
            return;
        };

        step(&mut sim.particles, delta);

        // Trails (the travel beat) — motes stretch along the emitter's velocity.
        let mut seen: Vec<Entity> = Vec::new();
        {
            let emitters: Vec<(Entity, VfxTrail, Vec3)> = world
                .query::<(Entity, &Transform, &VfxTrail, Option<&engine_core::components::Velocity>)>()
                .iter()
                .map(|(e, _, t, v)| (e, t.clone(), v.map(|v| v.linear).unwrap_or(Vec3::ZERO)))
                .collect();
            for (entity, trail, vel) in emitters {
                let Some(pos) = crate::render_position(world, entity, resources) else { continue };
                seen.push(entity);
                let owed = self.accum.entry(entity).or_insert(0.0);
                *owed += trail.rate * delta;
                while *owed >= 1.0 {
                    *owed -= 1.0;
                    sim.trail(pos, trail.color, vel, trail.cell, trail.blend, trail.stretch);
                }
            }
        }
        self.accum.retain(|e, _| seen.contains(e));

        let eye = engine_renderer::camera_eye(resources);
        if let Some(list) = resources.get_mut::<ParticleDrawList>() {
            list.additive_count = fill_draw_list(&sim.particles, eye, &mut list.instances);
        }
        resources.insert(sim);
    }
}

impl Default for ParticleSim {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_is_deterministic_and_in_range() {
        let mut a = XorShift32::new(7);
        let mut b = XorShift32::new(7);
        for _ in 0..100 {
            let (x, y) = (a.next_f32(), b.next_f32());
            assert_eq!(x, y);
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn hemisphere_dirs_are_unit_and_up_biased() {
        let mut rng = XorShift32::new(42);
        for _ in 0..100 {
            let d = hemisphere_dir(&mut rng);
            assert!((d.length() - 1.0).abs() < 1e-4);
            assert!(d.y >= 0.0, "burst sparks fly upward: {d}");
        }
    }

    #[test]
    fn step_integrates_gravity_and_retires_expired() {
        let mut particles = vec![Particle {
            pos:     Vec3::ZERO,
            vel:     Vec3::new(1.0, 0.0, 0.0),
            ttl:     0.1,
            life:    0.1,
            size:    0.1,
            color:   Vec3::ONE,
            gravity: -10.0,
            drag:    0.0,
            cell:    0,
            blend:   ParticleBlend::Additive,
            stretch: 0.0,
            axis:    Vec3::ZERO,
        }];
        step(&mut particles, 0.05);
        assert_eq!(particles.len(), 1);
        assert!(particles[0].pos.x > 0.0, "moved along velocity");
        assert!(particles[0].vel.y < 0.0, "gravity pulled velocity down");
        step(&mut particles, 0.1);
        assert!(particles.is_empty(), "expired particle removed");
    }

    #[test]
    fn fill_premultiplies_fade_and_caps() {
        let half_faded = Particle {
            pos:     Vec3::new(1.0, 2.0, 3.0),
            vel:     Vec3::ZERO,
            ttl:     0.5,
            life:    1.0,
            size:    0.2,
            color:   Vec3::new(1.0, 0.5, 0.0),
            gravity: 0.0,
            drag:    0.0,
            cell:    0,
            blend:   ParticleBlend::Additive,
            stretch: 0.0,
            axis:    Vec3::ZERO,
        };
        let mut out = Vec::new();
        fill_draw_list(&[half_faded], Vec3::ZERO, &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0].color[0] - 0.5).abs() < 1e-6, "rgb scaled by fade");
        assert!((out[0].size - 0.1).abs() < 1e-6, "size shrinks with fade");

        let many: Vec<Particle> = (0..MAX_PARTICLES + 100)
            .map(|_| Particle {
                pos:     Vec3::ZERO,
                vel:     Vec3::ZERO,
                ttl:     1.0,
                life:    1.0,
                size:    0.1,
                color:   Vec3::ONE,
                gravity: 0.0,
                drag:    0.0,
                cell:    0,
                blend:   ParticleBlend::Additive,
                stretch: 0.0,
                axis:    Vec3::ZERO,
            })
            .collect();
        fill_draw_list(&many, Vec3::ZERO, &mut out);
        assert_eq!(out.len(), MAX_PARTICLES, "draw list capped");
    }

    #[test]
    fn fill_partitions_additive_before_alpha() {
        let make = |blend: ParticleBlend, r: f32| Particle {
            pos:     Vec3::ZERO,
            vel:     Vec3::ZERO,
            ttl:     1.0,
            life:    1.0,
            size:    0.1,
            color:   Vec3::new(r, 0.0, 0.0),
            gravity: 0.0,
            drag:    0.0,
            cell:    0,
            blend,
            stretch: 0.0,
            axis:    Vec3::ZERO,
        };
        // Interleaved blends must come out partitioned, additive first.
        let particles = vec![
            make(ParticleBlend::Alpha, 0.1),
            make(ParticleBlend::Additive, 0.2),
            make(ParticleBlend::Alpha, 0.3),
            make(ParticleBlend::Additive, 0.4),
        ];
        let mut out = Vec::new();
        let additive = fill_draw_list(&particles, Vec3::ZERO, &mut out);
        assert_eq!(additive, 2);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].color[0], 0.2);
        assert_eq!(out[1].color[0], 0.4);
        assert_eq!(out[2].color[0], 0.1);
        assert_eq!(out[3].color[0], 0.3);
    }

    #[test]
    fn fill_sorts_alpha_partition_far_first_regardless_of_pool_order() {
        let make = |blend: ParticleBlend, pos: Vec3, r: f32| Particle {
            pos,
            vel:     Vec3::ZERO,
            ttl:     1.0,
            life:    1.0,
            size:    0.1,
            color:   Vec3::new(r, 0.0, 0.0),
            gravity: 0.0,
            drag:    0.0,
            cell:    0,
            blend,
            stretch: 0.0,
            axis:    Vec3::ZERO,
        };
        let eye = Vec3::ZERO;
        // Pool order puts the near puff first and the far puff second — the
        // opposite of the back-to-front draw order the alpha blend needs.
        let particles = vec![
            make(ParticleBlend::Alpha, Vec3::new(0.0, 0.0, 2.0), 0.2),  // near
            make(ParticleBlend::Alpha, Vec3::new(0.0, 0.0, 10.0), 0.8), // far
        ];
        let mut out = Vec::new();
        fill_draw_list(&particles, eye, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].color[0], 0.8, "far particle draws first (back-to-front)");
        assert_eq!(out[1].color[0], 0.2, "near particle draws last");

        // Swap-remove on expiry can flip pool order; the sort must not care.
        let particles_swapped = vec![
            make(ParticleBlend::Alpha, Vec3::new(0.0, 0.0, 10.0), 0.8), // far
            make(ParticleBlend::Alpha, Vec3::new(0.0, 0.0, 2.0), 0.2),  // near
        ];
        let mut out2 = Vec::new();
        fill_draw_list(&particles_swapped, eye, &mut out2);
        assert_eq!(out2[0].color[0], 0.8, "far-first holds regardless of pool order");
        assert_eq!(out2[1].color[0], 0.2);
    }

    #[test]
    fn burst_spawns_count_and_respects_cap() {
        let mut sim = ParticleSim::new();
        sim.burst(Vec3::ZERO, Vec3::ONE, 20, 4.0, 0.1);
        assert_eq!(sim.particles.len(), 20);
        sim.burst(Vec3::ZERO, Vec3::ONE, MAX_PARTICLES, 4.0, 0.1);
        assert_eq!(sim.particles.len(), MAX_PARTICLES, "burst stops at the cap");
    }
}
