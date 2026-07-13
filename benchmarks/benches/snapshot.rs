// Server snapshot fan-out — the clients-per-zone scaling curve (10 Hz per
// client, staggered over the 60 Hz PostUpdate phase): per client: AOI grid
// query into reused scratch, one query-view lookup per candidate, known-set
// diff, select_states sort, postcard encode, channel send.
//
// A real NetServer is bound on 127.0.0.1:0 with ZERO connected clients and
// the NetServerState gets fabricated ConnIds — engine-net's router silently
// drops sends to unknown conns, so the timed loop measures the full
// sim-thread cost with no network I/O.
//
// Also: select_states micro (the O(A log A) per-client sort) and
// MechanicResolveSystem (O(targets × conns) linear conn scan per due
// mechanic, with full 32-entry history rewind per player target).

use criterion::{criterion_group, criterion_main, Criterion};
use engine_app::scheduler::System;
use engine_core::components::{CellOccupant, CollisionShape, Health, Hitbox, Solid, Transform};
use engine_core::prefab::PrefabId;
use engine_core::World;
use engine_net::NetServer;
use glam::Vec3;
use hecs::Entity;
use std::time::{Duration, Instant};
use vordar_benches::{physics_resources, positions, prime_grid, Lcg, Layout, DT};
use vordar_game::player::Player;
use vordar_game::Mechanic;
use vordar_protocol::PROTOCOL_VERSION;
use vordar_server::db::DbWorker;
use vordar_server::net_plugin::{bench as seam, MechanicResolveSystem, SnapshotBroadcastSystem};

/// Everyone inside half_extent 14 is mutually within AOI_RADIUS=40 (max
/// pairwise distance ≈ 39.6) — the soak test's worst-case crowd shape.
const CROWD: Layout = Layout::Clustered { half_extent: 14.0 };

fn spawn_player_entity(world: &mut World, pos: Vec3) -> Entity {
    world.spawn((
        Transform::new(pos),
        Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
        CellOccupant { cells: Default::default() },
        Solid,
        PrefabId("player".into()),
        Player { speed: 6.0 },
        Health::new(100),
    ))
}

fn spawn_npc(world: &mut World, pos: Vec3) {
    world.spawn((
        Transform::new(pos),
        Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
        CellOccupant { cells: Default::default() },
        Solid,
        PrefabId("enemy_sentinel".into()),
    ));
}

fn bench_select_states(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot/select_states");
    for a in [64usize, 200, 1000] {
        let mut rng = Lcg::new(7);
        let entries: Vec<(u32, f32)> = (0..a as u32).map(|id| (id, rng.next_f32() * 1600.0)).collect();
        group.bench_function(format!("a{a}"), |b| {
            b.iter(|| seam::select_states(&entries, 0, seam::MAX_STATES, seam::NEAREST));
        });
    }
    group.finish();
}

fn bench_broadcast(c: &mut Criterion) {
    // Broadcast is staggered: each run serves the conns whose id falls in
    // this tick's slice. `broadcast` times STAGGER_TICKS consecutive runs
    // (every conn served exactly once — comparable to the pre-stagger
    // baseline); `broadcast_slice` times a single run (the per-tick cost
    // that actually lands on the 60 Hz sim loop).
    for slice in [false, true] {
        let name = if slice { "snapshot/broadcast_slice" } else { "snapshot/broadcast" };
        let mut group = c.benchmark_group(name);
        group.sample_size(30);
        for (clients, npcs) in [(10usize, 0usize), (50, 0), (200, 0), (50, 500)] {
            let worker = DbWorker::spawn(":memory:").unwrap();
            let server = NetServer::bind("127.0.0.1:0".parse().unwrap(), PROTOCOL_VERSION).unwrap();

            let mut world = World::new();
            let players: Vec<Entity> = positions(clients, CROWD, 7)
                .into_iter()
                .map(|pos| spawn_player_entity(&mut world, pos))
                .collect();
            for pos in positions(npcs, CROWD, 13) {
                spawn_npc(&mut world, pos);
            }

            let mut resources = physics_resources();
            prime_grid(&mut world, &mut resources);
            resources.insert(seam::state_with_fake_conns(server, worker.handle(), &players));

            let mut sys = SnapshotBroadcastSystem::new();
            // One full stagger round populates every conn's known set —
            // bench the steady state.
            for _ in 0..seam::STAGGER_TICKS {
                sys.run(&mut world, &mut resources, DT);
            }

            group.bench_function(format!("c{clients}_npc{npcs}"), |b| {
                if slice {
                    b.iter(|| sys.run(&mut world, &mut resources, DT));
                } else {
                    b.iter(|| {
                        for _ in 0..seam::STAGGER_TICKS {
                            sys.run(&mut world, &mut resources, DT);
                        }
                    });
                }
            });
        }
        group.finish();
    }
}

fn bench_mechanic_resolve(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot/mechanic_resolve");
    group.sample_size(30);
    for clients in [50usize, 200] {
        let worker = DbWorker::spawn(":memory:").unwrap();
        let server = NetServer::bind("127.0.0.1:0".parse().unwrap(), PROTOCOL_VERSION).unwrap();

        let mut world = World::new();
        let players: Vec<Entity> = positions(clients, CROWD, 7)
            .into_iter()
            .map(|pos| spawn_player_entity(&mut world, pos))
            .collect();
        let caster = world.spawn(()); // excluded from the hit test, never a target

        let mut resources = physics_resources();
        let mut state = seam::state_with_fake_conns(server, worker.handle(), &players);
        // Stamps far in the future force the full per-target history rewind.
        seam::fill_histories(&mut state, u64::MAX / 2);
        resources.insert(state);

        // The system despawns each resolved mechanic — respawn it untimed.
        // A fresh system per iteration keeps its 10 Hz self-gate open (only
        // the first run after construction resolves unconditionally).
        group.bench_function(format!("c{clients}"), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    world.spawn((
                        Transform::new(Vec3::ZERO),
                        // radius 50 covers the whole crowd; damage 0 keeps the
                        // world stationary across iterations.
                        Mechanic { id: 1, radius: 50.0, damage: 0, damage_type: Default::default(), resolve_at_micros: 0, caster },
                    ));
                    let mut sys = MechanicResolveSystem::new();
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

criterion_group!(benches, bench_select_states, bench_broadcast, bench_mechanic_resolve);
criterion_main!(benches);
