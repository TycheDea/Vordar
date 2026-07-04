// Collision-chain benchmarks (runs at 60 Hz on server and client):
//   CellUpdate  — full grid rebuild, O(N · cells-per-entity)
//   Broadphase  — candidate pairs, O(Σ cell-occupancy²) — quadratic in dense cells
//   Narrowphase — shape tests, O(pairs) with 4 world.get + shape.clone per pair
//
// Uniform layouts model realistic zone density; Clustered packs everyone into
// one ~10×10 region (a boss pile) — the broadphase worst case. 200 clustered
// is the soak-test design point.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;
use engine_physics::broadphase::{BroadphaseSystem, CandidatePairs};
use engine_physics::cell_update::CellUpdateSystem;
use engine_physics::narrowphase::NarrowphaseSystem;
use vordar_benches::{physics_resources, prime_grid, spawn_crowd, uniform_half, Layout, DT};

fn scenario(n: usize, layout: Layout) -> (World, Resources) {
    let mut world = World::new();
    spawn_crowd(&mut world, n, layout, 7);
    (world, physics_resources())
}

const CLUSTER: Layout = Layout::Clustered { half_extent: 5.0 };

fn bench_cell_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("physics/cell_update");
    for n in [200usize, 1000, 5000] {
        let (mut world, mut resources) = scenario(n, Layout::Uniform { half_extent: uniform_half(n) });
        let mut sys = CellUpdateSystem::new();
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| sys.run(&mut world, &mut resources, DT));
        });
    }
    group.finish();
}

fn bench_broadphase(c: &mut Criterion) {
    let mut group = c.benchmark_group("physics/broadphase");
    for n in [1000usize, 5000] {
        let (mut world, mut resources) = scenario(n, Layout::Uniform { half_extent: uniform_half(n) });
        prime_grid(&mut world, &mut resources);
        let mut sys = BroadphaseSystem::new();
        group.bench_with_input(BenchmarkId::new("uniform", n), &n, |b, _| {
            b.iter(|| sys.run(&mut world, &mut resources, DT));
        });
    }
    for n in [100usize, 200, 500] {
        let (mut world, mut resources) = scenario(n, CLUSTER);
        prime_grid(&mut world, &mut resources);
        let mut sys = BroadphaseSystem::new();
        sys.run(&mut world, &mut resources, DT);
        let pairs = resources.get::<CandidatePairs>().unwrap().0.len();
        eprintln!("broadphase/cluster/{n}: {pairs} candidate pairs");
        group.bench_with_input(BenchmarkId::new("cluster", n), &n, |b, _| {
            b.iter(|| sys.run(&mut world, &mut resources, DT));
        });
    }
    group.finish();
}

fn bench_narrowphase(c: &mut Criterion) {
    let mut group = c.benchmark_group("physics/narrowphase");
    for n in [100usize, 200, 500] {
        let (mut world, mut resources) = scenario(n, CLUSTER);
        prime_grid(&mut world, &mut resources);
        BroadphaseSystem::new().run(&mut world, &mut resources, DT);
        let mut sys = NarrowphaseSystem::new();
        group.bench_with_input(BenchmarkId::new("cluster", n), &n, |b, _| {
            b.iter(|| sys.run(&mut world, &mut resources, DT));
        });
    }
    group.finish();
}

fn bench_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("physics/chain");
    let cases: Vec<(String, usize, Layout)> = vec![
        ("uniform-1000".into(), 1000, Layout::Uniform { half_extent: uniform_half(1000) }),
        ("uniform-5000".into(), 5000, Layout::Uniform { half_extent: uniform_half(5000) }),
        ("cluster-200".into(), 200, CLUSTER),
    ];
    for (name, n, layout) in cases {
        let (mut world, mut resources) = scenario(n, layout);
        let mut cell = CellUpdateSystem::new();
        let mut broad = BroadphaseSystem::new();
        let mut narrow = NarrowphaseSystem::new();
        group.bench_function(&name, |b| {
            b.iter(|| {
                cell.run(&mut world, &mut resources, DT);
                broad.run(&mut world, &mut resources, DT);
                narrow.run(&mut world, &mut resources, DT);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_cell_update, bench_broadphase, bench_narrowphase, bench_chain);
criterion_main!(benches);
