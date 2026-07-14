// vordar-benches — shared scenario builders for the criterion suite.
//
// Everything here is deterministic (LCG layouts, no rand) so runs are
// comparable across machines and commits. Sizes mirror the real tuning:
// cell_size 10.0 (PhysicsPlugin), AOI radius 40.0 (server net module), hitboxes
// Sphere r=0.5 (content prefabs).

use engine_app::events::EventBus;
use engine_core::components::{CellOccupant, CollisionShape, Hitbox, Solid, Transform, Velocity};
use engine_core::spatial::SpatialGrid;
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use engine_physics::broadphase::CandidatePairs;
use engine_physics::cell_update::CellUpdateSystem;
use engine_physics::narrowphase::ActivePairs;
use glam::Vec3;
use hecs::Entity;

pub const DT: f32 = 1.0 / 60.0;
/// Matches PhysicsPlugin's SpatialGrid::new(10.0).
pub const CELL_SIZE: f32 = 10.0;
/// Matches the server net module's AOI_RADIUS.
pub const AOI_RADIUS: f32 = 40.0;

/// Prefabs load from content/ relative to cwd — run as if from workspace root
/// (same trick as server/vordar-server/tests/common).
pub fn workspace_root() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    std::env::set_current_dir(root).unwrap();
}

/// Deterministic LCG — same constants as tests/soak.rs's Wander.
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(2862933555777941757).wrapping_add(3037000493))
    }

    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as u32) as f32 / (u32::MAX as f32 + 1.0)
    }
}

/// Half-extent that spreads `n` entities at a realistic zone density of
/// ~2.5 entities per 10×10 grid cell (constant density across sweep sizes).
pub fn uniform_half(n: usize) -> f32 {
    ((n as f32 / 2.5) * (CELL_SIZE * CELL_SIZE)).sqrt() / 2.0
}

#[derive(Clone, Copy)]
pub enum Layout {
    /// Spread over [-half_extent, half_extent]² on XZ — realistic zone density.
    Uniform { half_extent: f32 },
    /// Packed into one small square — the dense-cell worst case for broadphase
    /// (everyone shares a handful of grid cells) and for separation overlap.
    Clustered { half_extent: f32 },
}

pub fn positions(n: usize, layout: Layout, seed: u64) -> Vec<Vec3> {
    let half = match layout {
        Layout::Uniform { half_extent } | Layout::Clustered { half_extent } => half_extent,
    };
    let mut rng = Lcg::new(seed);
    (0..n)
        .map(|_| {
            let x = (rng.next_f32() * 2.0 - 1.0) * half;
            let z = (rng.next_f32() * 2.0 - 1.0) * half;
            Vec3::new(x, 0.0, z)
        })
        .collect()
}

/// The collidable-entity archetype the game spawns (player.ron / enemy prefabs):
/// Transform + Velocity + Hitbox(Sphere 0.5) + CellOccupant + Solid.
pub fn spawn_crowd(world: &mut World, n: usize, layout: Layout, seed: u64) -> Vec<Entity> {
    positions(n, layout, seed)
        .into_iter()
        .map(|pos| {
            world.spawn((
                Transform::new(pos),
                Velocity { linear: Vec3::ZERO },
                Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
                CellOccupant { cells: Default::default() },
                Solid,
            ))
        })
        .collect()
}

/// Everything the collision pipeline + game systems expect in Resources.
pub fn physics_resources() -> Resources {
    let mut resources = Resources::new();
    resources.insert(SpatialGrid::new(CELL_SIZE));
    resources.insert(CandidatePairs::new());
    resources.insert(ActivePairs::new());
    resources.insert(EventBus::new());
    resources.insert(DespawnQueue::new());
    resources
}

/// Run CellUpdateSystem once so the grid reflects the world.
pub fn prime_grid(world: &mut World, resources: &mut Resources) {
    use engine_app::scheduler::System;
    CellUpdateSystem::new().run(world, resources, DT);
}
