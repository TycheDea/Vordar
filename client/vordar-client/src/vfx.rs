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
use vordar_game::vfx::VfxTrail;

// ── Look tuning ─────────────────────────────────────────────────────────────

/// Cast burst: sparks per cast, launch speed, particle half-extent.
pub const CAST_COUNT: usize = 14;
pub const CAST_SPEED: f32 = 4.0;
pub const CAST_SIZE: f32 = 0.12;
/// Impact burst per point of damage (clamped) — see react.rs.
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
    pub fn burst(&mut self, pos: Vec3, color: Vec3, count: usize, speed: f32, size: f32) {
        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                return;
            }
            let dir = hemisphere_dir(&mut self.rng);
            let jitter = 0.5 + 0.5 * self.rng.next_f32();
            let ttl = BURST_TTL_MIN + (BURST_TTL_MAX - BURST_TTL_MIN) * self.rng.next_f32();
            self.particles.push(Particle {
                pos,
                vel: dir * speed * jitter,
                ttl,
                life: ttl,
                size,
                color,
                gravity: GRAVITY,
                drag: DRAG,
            });
        }
    }

    /// One trail mote: barely moving, fast-fading — a streak emerges from the
    /// emitting entity's own motion.
    pub fn trail(&mut self, pos: Vec3, color: Vec3) {
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
            vel: jitter,
            ttl: TRAIL_TTL,
            life: TRAIL_TTL,
            size: TRAIL_SIZE,
            color,
            gravity: 0.0,
            drag: 0.0,
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
/// into the color (the additive blend has no alpha) and shrinks the quad.
/// Stops at the renderer's particle cap.
pub fn fill_draw_list(particles: &[Particle], out: &mut Vec<ParticleInstance>) {
    out.clear();
    for p in particles.iter().take(MAX_PARTICLES) {
        let fade = (p.ttl / p.life).clamp(0.0, 1.0);
        let rgb = p.color * fade;
        out.push(ParticleInstance {
            position: p.pos.to_array(),
            size:     p.size * fade,
            color:    [rgb.x, rgb.y, rgb.z, 1.0],
        });
    }
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

/// Spark burst for a cast, at the caster's weapon hand when the renderer
/// published a socket for it this frame (sockets are one frame stale from
/// Phase::Input — invisible at burst speeds), else at chest height.
pub fn cast_burst(world: &World, resources: &mut Resources, entity: Entity, color: Vec3) {
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
    if let Some(sim) = resources.get_mut::<ParticleSim>() {
        sim.burst(pos, color, CAST_COUNT, CAST_SPEED, CAST_SIZE);
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

        // Trails.
        let mut seen: Vec<Entity> = Vec::new();
        {
            let emitters: Vec<(Entity, Vec3, f32)> = world
                .query::<(Entity, &Transform, &VfxTrail)>()
                .iter()
                .map(|(e, _, t)| (e, t.color, t.rate))
                .collect();
            for (entity, color, rate) in emitters {
                let Some(pos) = crate::render_position(world, entity, resources) else { continue };
                seen.push(entity);
                let owed = self.accum.entry(entity).or_insert(0.0);
                *owed += rate * delta;
                while *owed >= 1.0 {
                    *owed -= 1.0;
                    sim.trail(pos, color);
                }
            }
        }
        self.accum.retain(|e, _| seen.contains(e));

        if let Some(list) = resources.get_mut::<ParticleDrawList>() {
            fill_draw_list(&sim.particles, &mut list.instances);
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
        };
        let mut out = Vec::new();
        fill_draw_list(&[half_faded], &mut out);
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
            })
            .collect();
        fill_draw_list(&many, &mut out);
        assert_eq!(out.len(), MAX_PARTICLES, "draw list capped");
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
