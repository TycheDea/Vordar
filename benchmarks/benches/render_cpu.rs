// Render-side CPU baselines: the per-frame costs that grow with the future
// enemy influx.
//
//   joint_palette — sample + globals + palette multiply for 40 skinned rigs
//                   of 64 joints (the stress-scene figure)
//   particle_fill — draw-list conversion at the 4096-particle cap
//
// Baselines to compare against when enemies land:
//   cargo bench -p vordar-benches --bench render_cpu -- --save-baseline pre-enemies

use criterion::{criterion_group, criterion_main, Criterion};
use engine_renderer::anim::{
    global_transforms, sample_pose, AnimationClip, Interp, Joint, JointTracks, LocalTransform,
    Skeleton, Track,
};
use engine_renderer::culling::{Aabb, Frustum, classify, Visibility};
use glam::{Mat4, Quat, Vec3};
use std::hint::black_box;
use vordar_client::vfx::{fill_draw_list, ParticleSim};
use vordar_game::vfx::ParticleBlend;

const RIGS: usize = 40;
const JOINTS: usize = 64;

/// A 64-joint chain with a keyframed rotation track on every joint —
/// worst-case sampling (every channel animated, deep hierarchy).
fn stress_skeleton() -> (Skeleton, AnimationClip) {
    let joints = (0..JOINTS)
        .map(|i| Joint {
            parent:       if i == 0 { None } else { Some(i - 1) },
            inverse_bind: Mat4::from_translation(Vec3::new(0.0, -(i as f32) * 0.1, 0.0)),
            rest: LocalTransform {
                translation: Vec3::new(0.0, 0.1, 0.0),
                rotation:    Quat::IDENTITY,
                scale:       Vec3::ONE,
            },
            name: format!("bone{i}"),
        })
        .collect();
    let skeleton = Skeleton { joints, root: Mat4::IDENTITY };

    let tracks = (0..JOINTS)
        .map(|i| JointTracks {
            rotation: Some(Track {
                times: (0..30).map(|k| k as f32 / 30.0).collect(),
                values: (0..30)
                    .map(|k| Quat::from_rotation_z((k + i) as f32 * 0.02))
                    .collect(),
                interp: Interp::Linear,
            }),
            ..Default::default()
        })
        .collect();
    let clip = AnimationClip { name: "stress".into(), duration: 1.0, tracks };
    (skeleton, clip)
}

fn joint_palette(c: &mut Criterion) {
    let (skeleton, clip) = stress_skeleton();
    c.bench_function("joint_palette_40x64", |b| {
        b.iter(|| {
            let mut total = Mat4::ZERO;
            for rig in 0..RIGS {
                let t = rig as f32 * 0.021;
                let pose = sample_pose(&skeleton, &clip, black_box(t));
                let globals = global_transforms(&skeleton, &pose);
                for (global, joint) in globals.iter().zip(&skeleton.joints) {
                    total += *global * joint.inverse_bind;
                }
            }
            black_box(total)
        })
    });
}

fn particle_fill(c: &mut Criterion) {
    let mut sim = ParticleSim::new();
    sim.burst(Vec3::ZERO, Vec3::ONE, 4096, 4.0, 0.1);
    // Give half the pool the alpha blend so the partition path is exercised.
    for (i, p) in sim.particles.iter_mut().enumerate() {
        if i % 2 == 0 {
            p.blend = ParticleBlend::Alpha;
        }
    }
    let eye = Vec3::new(0.0, 5.0, 15.0);
    let mut out = Vec::new();
    c.bench_function("particle_fill_4096", |b| {
        b.iter(|| {
            let additive = fill_draw_list(black_box(&sim.particles), eye, &mut out);
            black_box((additive, out.len()))
        })
    });
}

fn frustum_classify(c: &mut Criterion) {
    // Build perspective frustum (default orbit geometry): eye ≈ (24.0, 22.7, 24.0)
    let eye = Vec3::new(24.0, 22.7, 24.0);
    let perspective_vp = Mat4::perspective_rh(45f32.to_radians(), 16.0 / 9.0, 0.1, 200.0)
        * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
    let camera_frustum = Frustum::from_view_proj(perspective_vp);

    // Build orthographic frustum (shadow light): sun_dir = (-1, 2, -1).normalized()
    let sun_dir = Vec3::new(-1.0, 2.0, -1.0).normalize();
    let ortho_vp = Mat4::orthographic_rh(-160.0, 160.0, -160.0, 160.0, 0.0, 400.0)
        * Mat4::look_at_rh(sun_dir * 200.0, Vec3::ZERO, Vec3::Y);
    let shadow_frustum = Frustum::from_view_proj(ortho_vp);

    // 552 unit AABBs scattered deterministically over ±160-unit square
    // (40 rigs + 512 statics, index-hash positions)
    let aabbs: Vec<(Aabb, Mat4)> = (0..552)
        .map(|i: u32| {
            // Deterministic hash-based position: use index to spread across ±160 square
            let hash = (i.wrapping_mul(73856093) ^ (i.wrapping_mul(19349663))) as f32;
            let normalized_hash = (hash.abs() % 1.0) * 2.0 - 1.0; // [-1, 1]
            let x = normalized_hash * 160.0;
            let hash2 = (i.wrapping_mul(83492791)).wrapping_add(i << 16) as f32;
            let normalized_hash2 = (hash2.abs() % 1.0) * 2.0 - 1.0;
            let z = normalized_hash2 * 160.0;
            let y = 0.0;

            let local_aabb = Aabb { min: Vec3::splat(-0.5), max: Vec3::splat(0.5) };
            let transform = Mat4::from_translation(Vec3::new(x, y, z));
            (local_aabb, transform)
        })
        .collect();

    c.bench_function("frustum_classify_552", |b| {
        b.iter(|| {
            let mut both_count = 0;
            let mut cam_count = 0;
            let mut shadow_count = 0;

            for (local_aabb, transform) in black_box(&aabbs) {
                let world_aabb = local_aabb.transformed(transform);
                if let Some(vis) = classify(&world_aabb, &camera_frustum, &shadow_frustum) {
                    match vis {
                        Visibility::Both => both_count += 1,
                        Visibility::CamOnly => cam_count += 1,
                        Visibility::ShadowOnly => shadow_count += 1,
                    }
                }
            }

            black_box((both_count, cam_count, shadow_count))
        })
    });
}

criterion_group!(benches, joint_palette, particle_fill, frustum_classify);
criterion_main!(benches);
