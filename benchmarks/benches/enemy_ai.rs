// EnemyAISystem — grid-based nearest-player targeting, every 60 Hz tick.
//
// The grid path only runs at p200 (below GRID_PLAYER_MIN players the O(P)
// scan is cheaper and the system falls back to it), so each group's p200
// entry is the grid measurement and p1/p50 cover the scan fallback.
//
// `idle`    — players parked far outside every aggro range: per-enemy cost is
//             the aggro-radius grid query that comes back empty.
// `aggro`   — aggro 12, players mixed into the crowd: grid query +
//             player-view filter + nearest selection, most enemies engage.
// `engaged` — aggro 1e9 (> GRID_AGGRO_MAX): the global-scan fallback path,
//             every enemy engaged. AttackKind::Melee throughout so the
//             data-driven behavior only sets a chase velocity (no projectile
//             spawns polluting the measurement, world stays stationary).

use criterion::{criterion_group, criterion_main, Criterion};
use engine_app::scheduler::System;
use engine_core::components::{CellOccupant, CollisionShape, Hitbox, Transform, Velocity};
use engine_core::World;
use glam::Vec3;
use vordar_benches::{physics_resources, positions, prime_grid, uniform_half, Layout, DT};
use vordar_game::player::Player;
use vordar_game::{AttackKind, Enemy};
use vordar_game::enemies::EnemyAISystem;

fn scenario(enemies: usize, players: usize, aggro: f32, player_offset: Vec3) -> World {
    let mut world = World::new();
    let half = uniform_half(enemies + players);

    for pos in positions(enemies, Layout::Uniform { half_extent: half }, 7) {
        world.spawn((
            Transform::new(pos),
            Velocity { linear: Vec3::ZERO },
            Enemy { speed: 2.0, aggro_range: aggro, attack: AttackKind::Melee, cooldown_left: 0.0 },
            Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
            CellOccupant { cells: Default::default() },
        ));
    }
    for pos in positions(players, Layout::Uniform { half_extent: half }, 13) {
        world.spawn((
            Transform::new(pos + player_offset),
            Player { speed: 6.0 },
            Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
            CellOccupant { cells: Default::default() },
        ));
    }
    world
}

fn bench_enemy_ai(c: &mut Criterion) {
    let sweep: [(usize, usize); 7] =
        [(50, 1), (200, 1), (200, 50), (200, 200), (1000, 1), (1000, 50), (1000, 200)];

    // Idle: park players far outside every aggro range — grid queries all come
    // back empty, no enemy engages.
    let far = Vec3::new(1e6, 0.0, 0.0);
    for (variant, aggro, offset) in
        [("idle", 8.0, far), ("aggro", 12.0, Vec3::ZERO), ("engaged", 1e9, Vec3::ZERO)]
    {
        let mut group = c.benchmark_group(format!("enemy_ai/{variant}"));
        for (e, p) in sweep {
            let mut world = scenario(e, p, aggro, offset);
            let mut resources = physics_resources();
            prime_grid(&mut world, &mut resources);
            let mut sys = EnemyAISystem::new();
            group.bench_function(format!("e{e}_p{p}"), |b| {
                b.iter(|| sys.run(&mut world, &mut resources, DT));
            });
        }
        group.finish();
    }
}

criterion_group!(benches, bench_enemy_ai);
criterion_main!(benches);
