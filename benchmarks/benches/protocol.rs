// postcard encode/decode — every snapshot is encoded once per client per
// 10 Hz tick (fresh Vec allocation each call), so this is the per-client
// serialization floor for capacity math.

use criterion::{criterion_group, criterion_main, Criterion};
use glam::{Vec2, Vec3};
use vordar_benches::Lcg;
use vordar_protocol::{decode, encode, ClientMsg, EntityPos, EntityState, ServerMsg, WirePos};

/// The steady-state worst frame under crowd throttling: a full 64-entry
/// states budget plus a few AOI enters/leaves.
fn snapshot_64() -> ServerMsg {
    let mut rng = Lcg::new(7);
    let mut pos = |i: u32| Vec3::new(rng.next_f32() * 80.0 - 40.0, 0.0, i as f32);
    ServerMsg::Snapshot {
        tick: 123_456,
        last_processed_seq: 7_890,
        enters: (0..8)
            .map(|i| EntityState { id: 1_000 + i, prefab: "enemy_sentinel".into(), pos: WirePos(pos(i)), hp: 40 })
            .collect(),
        leaves: (0..8).map(|i| 2_000 + i).collect(),
        states: (0..64).map(|i| EntityPos { id: 3_000 + i, pos: WirePos(pos(i)), hp: 40 }).collect(),
    }
}

fn bench_protocol(c: &mut Criterion) {
    let snapshot = snapshot_64();
    let snapshot_bytes = encode(&snapshot);
    eprintln!("snapshot_64 encoded size: {} B", snapshot_bytes.len());
    let intent = ClientMsg::MoveIntent { seq: 42, t_server_micros: 1_234_567, dir: Vec2::new(0.6, -0.8) };
    let intent_bytes = encode(&intent);

    let mut group = c.benchmark_group("protocol");
    group.bench_function("encode/snapshot_64", |b| b.iter(|| encode(&snapshot)));
    group.bench_function("decode/snapshot_64", |b| {
        b.iter(|| decode::<ServerMsg>(&snapshot_bytes).unwrap())
    });
    group.bench_function("encode/move_intent", |b| b.iter(|| encode(&intent)));
    group.bench_function("decode/move_intent", |b| {
        b.iter(|| decode::<ClientMsg>(&intent_bytes).unwrap())
    });
    group.finish();
}

criterion_group!(benches, bench_protocol);
criterion_main!(benches);
