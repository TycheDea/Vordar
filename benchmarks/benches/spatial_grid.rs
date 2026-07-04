// SpatialGrid micro-benchmarks: full rebuild cost (CellUpdate pays this every
// 60 Hz tick) and AOI-radius queries — allocating query_radius (what the
// snapshot path calls per client) vs buffer-reusing query_radius_into.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use engine_core::spatial::SpatialGrid;
use engine_core::World;
use glam::Vec3;
use vordar_benches::{positions, uniform_half, Layout, AOI_RADIUS, CELL_SIZE};

fn bench_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("spatial_grid/rebuild");
    for n in [200usize, 1000, 5000] {
        let mut world = World::new();
        let pos = positions(n, Layout::Uniform { half_extent: uniform_half(n) }, 7);
        let items: Vec<_> = pos.into_iter().map(|p| (world.spawn(()), p)).collect();
        let mut grid = SpatialGrid::new(CELL_SIZE);
        group.bench_with_input(BenchmarkId::from_parameter(n), &items, |b, items| {
            b.iter(|| {
                grid.clear();
                for &(e, p) in items {
                    grid.insert(e, p);
                }
            });
        });
    }
    group.finish();
}

fn bench_query(c: &mut Criterion) {
    for n in [1000usize, 5000] {
        let mut world = World::new();
        let mut grid = SpatialGrid::new(CELL_SIZE);
        for p in positions(n, Layout::Uniform { half_extent: uniform_half(n) }, 7) {
            grid.insert(world.spawn(()), p);
        }

        let mut group = c.benchmark_group(format!("spatial_grid/query_aoi/{n}"));
        // What SnapshotBroadcastSystem pays per client per snapshot.
        group.bench_function("query_radius_alloc", |b| {
            b.iter(|| grid.query_radius(Vec3::ZERO, AOI_RADIUS));
        });
        group.bench_function("query_radius_into_reused", |b| {
            let mut buf: Vec<hecs::Entity> = Vec::new();
            b.iter(|| {
                buf.clear();
                grid.query_radius_into(Vec3::ZERO, AOI_RADIUS, &mut buf);
                buf.len()
            });
        });
        group.finish();
    }
}

criterion_group!(benches, bench_rebuild, bench_query);
criterion_main!(benches);
