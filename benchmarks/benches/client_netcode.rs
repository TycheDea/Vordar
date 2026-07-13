// Client netcode hot paths — WEAKPOINTS gap B. Everything headless:
// apply_snapshot / reconcile_own are plain functions over (World, Resources)
// fed fabricated snapshot payloads through vordar-client's bench-internals
// seam; the NetClientState's socket points nowhere.
//
// apply_snapshot clones the whole id→entity map per snapshot and restarts
// every remote entity's NetLerp (two world.gets each); reconcile_own replays
// up to 240 pending intents. The client runs on the weakest hardware in the
// system, so these are foundation numbers.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use engine_core::components::Transform;
use engine_core::prefab::{register_core_components, ComponentRegistry, PrefabLibrary};
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use glam::{Vec2, Vec3};
use std::time::{Duration, Instant};
use vordar_benches::{positions, workspace_root, Layout, DT};
use vordar_client::net::bench as seam;
use vordar_game::player::movement_velocity;
use vordar_game::Player;
use vordar_protocol::{EntityPos, EntityState, WirePos};

/// Everyone inside half_extent 14 is mutually within AOI — the same crowd
/// shape as the server-side snapshot bench.
const CROWD: Layout = Layout::Clustered { half_extent: 14.0 };

/// Resources for a replicating client: prefab machinery + despawn queue.
/// bolt.ron uses only core loaders, so no game components are needed.
fn client_resources() -> Resources {
    let mut resources = Resources::new();
    resources.insert(DespawnQueue::new());
    let mut registry = ComponentRegistry::new();
    register_core_components(&mut registry);
    let mut library = PrefabLibrary::new();
    library.load_dir("content/prefabs");
    resources.insert(registry);
    resources.insert(library);
    resources
}

/// Steady state at A ∈ {64, 200} replicated entities: every snapshot restarts
/// each entity's lerp (and today clones the whole entities map). The input
/// Vec is rebuilt untimed per iteration, matching decode's fresh allocation.
fn bench_apply_states(c: &mut Criterion) {
    workspace_root();
    let mut group = c.benchmark_group("client/apply_snapshot");
    for a in [64usize, 200] {
        let mut world = World::new();
        let mut resources = client_resources();
        let mut client_state = seam::state_for_bench(None, false);
        seam::set_prefab_table(&mut client_state, vec!["bolt".into()]);
        resources.insert(client_state);
        // Build the replicated set through the real enters path so every
        // entity carries NetLerp.
        let spots = positions(a, CROWD, 7);
        let enters: Vec<EntityState> = spots
            .iter()
            .enumerate()
            .map(|(i, &pos)| EntityState { id: i as u32 + 1, prefab: 0, pos: WirePos(pos), hp: None })
            .collect();
        seam::apply_snapshot(&mut world, &mut resources, 0, enters, Vec::new(), Vec::new());

        let states: Vec<EntityPos> = spots
            .iter()
            .enumerate()
            .map(|(i, &pos)| EntityPos { id: i as u32 + 1, pos: WirePos(pos + Vec3::X), hp: None })
            .collect();
        group.bench_function(format!("states_a{a}"), |b| {
            b.iter_batched(
                || states.clone(),
                |s| seam::apply_snapshot(&mut world, &mut resources, 0, Vec::new(), Vec::new(), s),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// A 64-entity AOI enter wave (zone-in / teleport worst case): one prefab
/// spawn per enter. Cleanup (leaves + despawn flush) is untimed.
fn bench_apply_enters(c: &mut Criterion) {
    workspace_root();
    let mut world = World::new();
    let mut resources = client_resources();
    let mut client_state = seam::state_for_bench(None, false);
    seam::set_prefab_table(&mut client_state, vec!["bolt".into()]);
    resources.insert(client_state);
    let spots = positions(64, CROWD, 7);

    c.bench_function("client/apply_snapshot/enters_64", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            let mut next_id = 1u32;
            for _ in 0..iters {
                let enters: Vec<EntityState> = spots
                    .iter()
                    .enumerate()
                    .map(|(i, &pos)| EntityState { id: next_id + i as u32, prefab: 0, pos: WirePos(pos), hp: None })
                    .collect();
                let t = Instant::now();
                seam::apply_snapshot(&mut world, &mut resources, 0, enters, Vec::new(), Vec::new());
                total += t.elapsed();
                // Untimed: leave + flush so the map stays small and ids fresh.
                let leaves: Vec<u32> = (next_id..next_id + 64).collect();
                next_id += 64;
                seam::apply_snapshot(&mut world, &mut resources, 0, Vec::new(), leaves, Vec::new());
                let pairs: Vec<_> = resources.get_mut::<DespawnQueue>().unwrap().0.drain(..).collect();
                for (entity, _) in pairs {
                    world.despawn(entity).ok();
                }
            }
            total
        });
    });
}

/// Reconciliation replay: rebase onto the server position and re-apply
/// pending intents. 60 ≈ one second of unacked intents; 240 is the cap
/// (server stopped acking — the worst case).
fn bench_reconcile(c: &mut Criterion) {
    let mut group = c.benchmark_group("client/reconcile");
    for n in [60usize, 240] {
        let mut world = World::new();
        let mut resources = Resources::new();
        let pos = Vec3::new(3.0, 0.0, 3.0);
        let entity = world.spawn((Transform::new(pos), Player { speed: 6.0 }));
        let mut state = seam::state_for_bench(Some(1), true);
        seam::map_entity(&mut state, 1, entity);
        for seq in 1..=n as u32 {
            seam::push_pending(&mut state, seq, Vec2::X, DT);
        }
        resources.insert(state);
        // server_pos puts the replayed position exactly on the current one:
        // zero error → Trust → nothing moves, iterations stay stationary
        // (seq 0 acks nothing, so the pending queue never shrinks either).
        let server_pos = pos - movement_velocity(Vec2::X, 6.0) * (DT * n as f32);

        group.bench_function(format!("pending{n}"), |b| {
            b.iter(|| seam::reconcile_own(&mut world, &mut resources, entity, server_pos, 0));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_apply_states, bench_apply_enters, bench_reconcile);
criterion_main!(benches);
