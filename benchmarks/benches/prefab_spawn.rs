// Prefab spawn cost — WEAKPOINTS gap A. spawn_prefab re-parses RON per
// component per spawn, and spawn_projectile goes straight through it, so
// every bolt from every player and ranged enemy is a multi-component parse
// at combat rate. The churn group adds the despawn side: the flush's
// SpatialGrid::remove is O(cell occupancy), so death waves in a dense pile
// pay both ends.

use criterion::{criterion_group, criterion_main, Criterion};
use engine_app::scheduler::System;
use engine_core::components::CellOccupant;
use engine_core::prefab::{register_core_components, spawn_prefab, ComponentRegistry, PrefabLibrary};
use engine_core::spatial::SpatialGrid;
use engine_core::traits::{Resources, SpawnContext};
use engine_core::World;
use engine_physics::cell_update::CellUpdateSystem;
use glam::Vec3;
use hecs::Entity;
use std::time::{Duration, Instant};
use vordar_benches::{physics_resources, prime_grid, spawn_crowd, workspace_root, Layout, DT};

/// physics_resources + the prefab machinery. bolt.ron declares only core
/// components (Transform, Velocity, RenderShape, Hitbox), so the core
/// loaders suffice.
fn prefab_resources() -> Resources {
    let mut resources = physics_resources();
    let mut registry = ComponentRegistry::new();
    register_core_components(&mut registry);
    let mut library = PrefabLibrary::new();
    library.load_dir("content/prefabs");
    resources.insert(registry);
    resources.insert(library);
    resources
}

/// The per-spawn cost alone: registry lookups + RON parses + build + spawn.
/// Despawn is untimed.
fn bench_spawn(c: &mut Criterion) {
    workspace_root();
    let mut world = World::new();
    let mut resources = prefab_resources();

    c.bench_function("prefab/spawn/bolt", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let t = Instant::now();
                let entity = spawn_prefab(
                    "bolt",
                    Vec3::ZERO,
                    &mut SpawnContext { world: &mut world, resources: &mut resources },
                )
                .expect("bolt spawns");
                total += t.elapsed();
                world.despawn(entity).unwrap();
            }
            total
        });
    });
}

/// Combat-rate churn in a dense pile: N bolts spawned and despawned per tick
/// with 500 bystanders packed into a few cells. Spawn side is the RON parse;
/// despawn side mirrors DespawnFlushSystem (engine-app flush.rs, pub(crate))
/// — grid.remove retains a Vec holding most of the pile, once per cell.
fn bench_churn(c: &mut Criterion) {
    workspace_root();
    let mut group = c.benchmark_group("prefab/churn");
    group.sample_size(30);
    for n in [8usize, 32] {
        let mut world = World::new();
        let mut resources = prefab_resources();
        spawn_crowd(&mut world, 500, Layout::Clustered { half_extent: 5.0 }, 7);
        prime_grid(&mut world, &mut resources);

        let mut cell_update = CellUpdateSystem::new();
        group.bench_function(format!("n{n}"), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    // Timed: the spawn side (spawn_projectile → spawn_prefab).
                    let t = Instant::now();
                    let bolts: Vec<Entity> = (0..n)
                        .map(|_| {
                            spawn_prefab(
                                "bolt",
                                Vec3::new(1.0, 0.0, 1.0),
                                &mut SpawnContext { world: &mut world, resources: &mut resources },
                            )
                            .expect("bolt spawns")
                        })
                        .collect();
                    total += t.elapsed();
                    // Untimed: the tick's grid rebuild registers the bolts.
                    cell_update.run(&mut world, &mut resources, DT);
                    // Timed: the despawn side, mirroring DespawnFlushSystem.
                    let t = Instant::now();
                    for entity in bolts {
                        if let Ok(occupant) = world.get::<&CellOccupant>(entity) {
                            if let Some(grid) = resources.get_mut::<SpatialGrid>() {
                                for cell in &occupant.cells {
                                    grid.remove(*cell, entity);
                                }
                            }
                        }
                        world.despawn(entity).ok();
                    }
                    total += t.elapsed();
                }
                total
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_spawn, bench_churn);
criterion_main!(benches);
