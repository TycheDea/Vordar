// SeparationSystem — O(active pairs) with a fresh HashMap allocation per tick
// and up to 6 world.get fetches per pair. Runs every 60 Hz tick in
// CollisionResolve; the soak test showed it halving a walker's progress
// through a 200-bot crowd.
//
// Positions are reset before every timed run (iter_custom, reset untimed):
// separation pushes entities apart, so without the reset the overlap work
// would decay across iterations and the measurement would drift.

use criterion::{criterion_group, criterion_main, Criterion};
use engine_app::scheduler::System;
use engine_core::components::Transform;
use engine_core::World;
use engine_physics::broadphase::BroadphaseSystem;
use engine_physics::narrowphase::{ActivePairs, NarrowphaseSystem};
use glam::Vec3;
use hecs::Entity;
use std::time::{Duration, Instant};
use vordar_benches::{physics_resources, prime_grid, spawn_crowd, Layout, DT};
use vordar_game::motion::SeparationSystem;

fn bench_separation(c: &mut Criterion) {
    let mut group = c.benchmark_group("separation");
    // (n, cluster half-extent) tuned for roughly 100 / 500 / 2000 active pairs;
    // the actual pair count is printed for the baseline doc.
    for (n, half) in [(100usize, 6.0f32), (200, 5.0), (400, 5.0)] {
        let mut world = World::new();
        let entities = spawn_crowd(&mut world, n, Layout::Clustered { half_extent: half }, 7);
        let mut resources = physics_resources();
        prime_grid(&mut world, &mut resources);
        BroadphaseSystem::new().run(&mut world, &mut resources, DT);
        NarrowphaseSystem::new().run(&mut world, &mut resources, DT);
        let pairs = resources.get::<ActivePairs>().unwrap().0.len();
        eprintln!("separation/n{n}: {pairs} active pairs");

        let snapshot: Vec<(Entity, Vec3)> = entities
            .iter()
            .map(|&e| (e, world.get::<&Transform>(e).unwrap().position))
            .collect();
        let mut sys = SeparationSystem;

        group.bench_function(format!("n{n}_pairs{pairs}"), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    for &(e, p) in &snapshot {
                        world.get::<&mut Transform>(e).unwrap().position = p;
                    }
                    let t = Instant::now();
                    sys.run(&mut world, &mut resources, DT);
                    total += t.elapsed();
                }
                total
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_separation);
criterion_main!(benches);
