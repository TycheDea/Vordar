// Render-side CPU baselines (Phase 8, VQ-F1): the per-frame costs that grow
// with the future enemy influx.
//
//   joint_palette — sample + globals + palette multiply for 40 skinned rigs
//                   of 64 joints (the VQ-F1 stress-scene figure)
//   particle_fill — draw-list conversion at the 4096-particle cap
//
// Baselines to compare against when enemies land:
//   cargo bench -p vordar-benches --bench render_cpu -- --save-baseline pre-enemies

use criterion::{criterion_group, criterion_main, Criterion};
use engine_renderer::anim::{
    global_transforms, sample_pose, AnimationClip, Interp, Joint, JointTracks, LocalTransform,
    Skeleton, Track,
};
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
    let mut out = Vec::new();
    c.bench_function("particle_fill_4096", |b| {
        b.iter(|| {
            let additive = fill_draw_list(black_box(&sim.particles), &mut out);
            black_box((additive, out.len()))
        })
    });
}

criterion_group!(benches, joint_palette, particle_fill);
criterion_main!(benches);
