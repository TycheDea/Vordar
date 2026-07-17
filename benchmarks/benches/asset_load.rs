// Asset loading baselines: CPU costs that will move off-frame with streaming.
//
// Measures the decode and processing costs of the heaviest shipped assets:
//   statue_vroid — glTF with embedded textures
//   human — skinned glTF with animation clips
//   zone_ground — texture decode + mesh generation for zone grounds
//
// These are the baseline costs before async load + GPU streaming work
// moves them off the critical path.

use criterion::{criterion_group, criterion_main, Criterion};
use engine_renderer::mesh::load_gltf_data;
use std::hint::black_box;
use vordar_client::ground;

fn asset_load_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("asset_load");
    group.sample_size(10);

    // `statue_vroid.glb` (11 MB, embedded textures, static bind pose)
    group.bench_function("first_sight/statue_vroid", |b| {
        b.iter(|| {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../content/models/statue_vroid.glb");
            match load_gltf_data(black_box(path)) {
                Ok(data) => black_box(data),
                Err(e) => panic!("load_gltf_data failed: {e}"),
            }
        })
    });

    // `human.glb` (9 MB, skinned with animation clips)
    group.bench_function("first_sight/human", |b| {
        b.iter(|| {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../content/models/human.glb");
            match load_gltf_data(black_box(path)) {
                Ok(data) => black_box(data),
                Err(e) => panic!("load_gltf_data failed: {e}"),
            }
        })
    });

    // Zone ground: load 3× 2k textures + mesh generation
    group.bench_function("zone_ground/decode_and_generate", |b| {
        b.iter(|| {
            let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../content/textures/ground/mud_leaves");
            let material = match ground::load_ground_material(black_box(dir)) {
                Ok(m) => m,
                Err(e) => panic!("load_ground_material failed: {e}"),
            };
            // Ground defaults: size = 400.0 (from vordar_game::zones::default_ground_size)
            // tile = 7.0 (from content/zones/zones.ron:21)
            let data = ground::generate_ground(black_box(400.0), black_box(7.0), black_box(material));
            black_box(data)
        })
    });

    group.finish();
}

criterion_group!(benches, asset_load_benches);
criterion_main!(benches);
