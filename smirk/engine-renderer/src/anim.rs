// Skeletal animation — the pure runtime math, deliberately free of glTF and
// wgpu so it unit-tests without a GPU device (same discipline as mesh.rs's CPU
// stage). mesh.rs constructs `Skeleton` + `AnimationClip` from a glTF file;
// this module samples, blends, and skins them.
//
// Coordinate convention (standard linear-blend skinning): a skinned vertex
// stays in mesh-local space and is placed by the joint palette. For joint j:
//
//     global[j]      = global[parent[j]] * local[j]          (forward pass)
//     root joints:   global[j] = Skeleton::root * local[j]
//     jointMatrix[j] = global[j] * inverseBind[j]
//     skinnedPos     = Σ  weightᵢ · jointMatrix[jointᵢ] · pos
//
// `Skeleton::root` is the world transform of the non-joint nodes above the
// skeleton (the "Armature"/"Rig" node an exporter puts the bones under). It is
// folded into every root joint's global so an armature carrying a scale or
// offset — e.g. a character authored at 2 m tall, or grounded so its feet sit
// on the floor — loads correctly. Identity when the armature sits at origin.

use glam::{Mat4, Quat, Vec3};

/// A node's local transform (TRS). The animated unit — clips drive these, the
/// forward pass composes them into world/joint matrices.
#[derive(Clone, Copy, Debug)]
pub struct LocalTransform {
    pub translation: Vec3,
    pub rotation:    Quat,
    pub scale:       Vec3,
}

impl LocalTransform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation:    Quat::IDENTITY,
        scale:       Vec3::ONE,
    };

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// One bone. `parent` indexes into `Skeleton::joints` (None = a root). Joints
/// are stored in glTF skin order so vertex JOINTS_0 indices reference them
/// directly; the forward pass tolerates any ordering (see `joint_matrices`).
#[derive(Clone)]
pub struct Joint {
    pub parent:       Option<usize>,
    pub inverse_bind: Mat4,
    /// Bind-pose local transform, used for any channel a clip doesn't animate.
    pub rest:         LocalTransform,
    /// glTF node name, kept so attachment sockets can address a bone
    /// ("handslot.r") without knowing skin joint order.
    pub name:         String,
}

#[derive(Clone)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
    /// World transform of the non-joint ancestors above the bones (scale/offset
    /// baked onto the armature). Prefixed onto every root joint's global.
    pub root: Mat4,
}

impl Skeleton {
    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }
}

/// glTF keyframe interpolation. CUBICSPLINE is downsampled to Linear at load
/// time (see mesh.rs), so the runtime only needs these two.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interp {
    Linear,
    Step,
}

/// A keyframe track for one channel of one joint. `times` is ascending;
/// `values` is parallel to it.
#[derive(Clone)]
pub struct Track<T> {
    pub times:  Vec<f32>,
    pub values: Vec<T>,
    pub interp: Interp,
}

/// Which of a joint's channels a clip animates. A `None` channel falls back to
/// the joint's rest value at sample time.
#[derive(Clone, Default)]
pub struct JointTracks {
    pub translation: Option<Track<Vec3>>,
    pub rotation:    Option<Track<Quat>>,
    pub scale:       Option<Track<Vec3>>,
}

/// A named animation. `tracks` is indexed by joint (parallel to
/// `Skeleton::joints`); joints this clip never touches hold `JointTracks::default`.
#[derive(Clone)]
pub struct AnimationClip {
    pub name:     String,
    pub duration: f32,
    pub tracks:   Vec<JointTracks>,
}

// ── Sampling ────────────────────────────────────────────────────────────────

/// Locate the segment containing `t`: returns `(i, j, u)` where the value is
/// `lerp(values[i], values[j], u)`. Clamps to the endpoints (no extrapolation).
fn segment(times: &[f32], t: f32) -> (usize, usize, f32) {
    if times.len() <= 1 {
        return (0, 0, 0.0);
    }
    if t <= times[0] {
        return (0, 0, 0.0);
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return (last, last, 0.0);
    }
    // times ascending: first index strictly greater than t, minus one.
    let hi = times.partition_point(|&x| x <= t);
    let lo = hi - 1;
    let span = times[hi] - times[lo];
    let u = if span > 0.0 { (t - times[lo]) / span } else { 0.0 };
    (lo, hi, u)
}

fn sample_vec3(track: &Track<Vec3>, t: f32) -> Vec3 {
    let (i, j, u) = segment(&track.times, t);
    match track.interp {
        Interp::Step => track.values[i],
        Interp::Linear => track.values[i].lerp(track.values[j], u),
    }
}

fn sample_quat(track: &Track<Quat>, t: f32) -> Quat {
    let (i, j, u) = segment(&track.times, t);
    match track.interp {
        Interp::Step => track.values[i],
        Interp::Linear => track.values[i].slerp(track.values[j], u),
    }
}

/// Sample a clip at time `t` (already wrapped/clamped by the caller) into a
/// full local pose — one `LocalTransform` per joint, resting where the clip is
/// silent.
pub fn sample_pose(skeleton: &Skeleton, clip: &AnimationClip, t: f32) -> Vec<LocalTransform> {
    (0..skeleton.joints.len())
        .map(|jx| {
            let rest = skeleton.joints[jx].rest;
            let tracks = clip.tracks.get(jx);
            LocalTransform {
                translation: tracks
                    .and_then(|c| c.translation.as_ref())
                    .map(|tk| sample_vec3(tk, t))
                    .unwrap_or(rest.translation),
                rotation: tracks
                    .and_then(|c| c.rotation.as_ref())
                    .map(|tk| sample_quat(tk, t))
                    .unwrap_or(rest.rotation),
                scale: tracks
                    .and_then(|c| c.scale.as_ref())
                    .map(|tk| sample_vec3(tk, t))
                    .unwrap_or(rest.scale),
            }
        })
        .collect()
}

/// Crossfade two poses: `w = 0` → `a`, `w = 1` → `b`. Per-joint lerp on
/// translation/scale, slerp on rotation — the standard blend that keeps a
/// clip transition smooth instead of popping.
pub fn blend_poses(a: &[LocalTransform], b: &[LocalTransform], w: f32) -> Vec<LocalTransform> {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| LocalTransform {
            translation: x.translation.lerp(y.translation, w),
            rotation:    x.rotation.slerp(y.rotation, w),
            scale:       x.scale.lerp(y.scale, w),
        })
        .collect()
}

// ── Skinning ────────────────────────────────────────────────────────────────

/// Compose a local pose into each joint's global (armature-space) transform —
/// the bone's posed frame *before* the inverse bind. This is what attachment
/// sockets need: `socket_world = model · global[j]`. Tolerates joints stored
/// in any order (parent index may exceed the child's) via memoised resolution.
pub fn global_transforms(skeleton: &Skeleton, pose: &[LocalTransform]) -> Vec<Mat4> {
    let n = skeleton.joints.len();
    let mut globals = vec![Mat4::IDENTITY; n];
    let mut done = vec![false; n];
    for j in 0..n {
        resolve_global(j, skeleton, pose, &mut globals, &mut done);
    }
    globals
}

/// Compose a local pose into the joint palette uploaded to the GPU:
/// `jointMatrix[j] = global[j] * inverseBind[j]`.
pub fn joint_matrices(skeleton: &Skeleton, pose: &[LocalTransform]) -> Vec<Mat4> {
    let globals = global_transforms(skeleton, pose);
    globals
        .iter()
        .zip(&skeleton.joints)
        .map(|(g, j)| *g * j.inverse_bind)
        .collect()
}

fn resolve_global(
    j:       usize,
    skel:    &Skeleton,
    pose:    &[LocalTransform],
    globals: &mut [Mat4],
    done:    &mut [bool],
) {
    if done[j] {
        return;
    }
    let local = pose[j].matrix();
    globals[j] = match skel.joints[j].parent {
        Some(p) => {
            resolve_global(p, skel, pose, globals, done);
            globals[p] * local
        }
        None => skel.root * local,
    };
    done[j] = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_skeleton() -> Skeleton {
        // Two joints: root at origin, child offset +1 on X. Inverse binds are
        // the inverse of each joint's bind global (root bind = I, child bind =
        // translate(1,0,0)).
        Skeleton {
            root: Mat4::IDENTITY,
            joints: vec![
                Joint {
                    parent:       None,
                    inverse_bind: Mat4::IDENTITY,
                    rest:         LocalTransform::IDENTITY,
                    name:         "root".into(),
                },
                Joint {
                    parent:       Some(0),
                    inverse_bind: Mat4::from_translation(Vec3::new(-1.0, 0.0, 0.0)),
                    rest: LocalTransform {
                        translation: Vec3::new(1.0, 0.0, 0.0),
                        ..LocalTransform::IDENTITY
                    },
                    name:         "child".into(),
                },
            ],
        }
    }

    #[test]
    fn segment_clamps_and_interpolates() {
        let t = [0.0f32, 1.0, 2.0];
        assert_eq!(segment(&t, -5.0), (0, 0, 0.0));
        assert_eq!(segment(&t, 5.0), (2, 2, 0.0));
        let (i, j, u) = segment(&t, 1.5);
        assert_eq!((i, j), (1, 2));
        assert!((u - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bind_pose_yields_identity_joint_matrices() {
        let skel = chain_skeleton();
        let pose: Vec<_> = skel.joints.iter().map(|j| j.rest).collect();
        let mats = joint_matrices(&skel, &pose);
        // At bind pose every joint matrix is identity (global == bind == inv(inverse_bind)).
        for m in mats {
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-5), "bind pose must skin to identity: {m}");
        }
    }

    #[test]
    fn rotating_root_moves_child_joint() {
        let skel = chain_skeleton();
        // Rotate the root 90° about Z; child rest stays (1,0,0) local.
        let mut pose: Vec<_> = skel.joints.iter().map(|j| j.rest).collect();
        pose[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let mats = joint_matrices(&skel, &pose);
        // A point at the child's bind position (1,0,0) skinned by the child
        // joint matrix should swing to about (0,1,0).
        let skinned = mats[1].transform_point3(Vec3::new(1.0, 0.0, 0.0));
        assert!(skinned.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-5), "got {skinned}");
    }

    #[test]
    fn root_offset_scales_and_grounds_the_bind_pose() {
        // An armature carrying a uniform 0.5 scale + a -0.5 ground drop (feet to
        // floor) must transform the whole bind pose by exactly that: a point at
        // the child's bind position (1,0,0) lands at root · (1,0,0).
        let mut skel = chain_skeleton();
        skel.root = Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0))
            * Mat4::from_scale(Vec3::splat(0.5));
        let pose: Vec<_> = skel.joints.iter().map(|j| j.rest).collect();
        let mats = joint_matrices(&skel, &pose);
        let skinned = mats[1].transform_point3(Vec3::new(1.0, 0.0, 0.0));
        assert!(
            skinned.abs_diff_eq(Vec3::new(0.5, -0.5, 0.0), 1e-5),
            "root offset must scale+ground the bind pose: got {skinned}"
        );
    }

    #[test]
    fn global_transforms_compose_chain_through_root_offset() {
        // globals are the bones' armature-space frames (pre-inverse-bind):
        // with a 0.5-scale, -0.5-ground root and the child rotated 90° about Z
        // at the root, the child's frame origin sits at root · rot · (1,0,0).
        let mut skel = chain_skeleton();
        skel.root = Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0))
            * Mat4::from_scale(Vec3::splat(0.5));
        let mut pose: Vec<_> = skel.joints.iter().map(|j| j.rest).collect();
        pose[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let globals = global_transforms(&skel, &pose);
        let child_origin = globals[1].transform_point3(Vec3::ZERO);
        assert!(
            child_origin.abs_diff_eq(Vec3::new(0.0, 0.0, 0.0), 1e-5),
            "child bind (1,0,0) rotated to (0,1,0), scaled to (0,0.5,0), grounded to (0,0,0): got {child_origin}"
        );
        // And joint_matrices stays consistent: globals · inverse_bind.
        let mats = joint_matrices(&skel, &pose);
        assert!(mats[1].abs_diff_eq(globals[1] * skel.joints[1].inverse_bind, 1e-6));
    }

    #[test]
    fn sample_pose_uses_track_then_falls_back_to_rest() {
        let skel = chain_skeleton();
        let clip = AnimationClip {
            name:     "spin".into(),
            duration: 1.0,
            tracks:   vec![
                JointTracks {
                    rotation: Some(Track {
                        times:  vec![0.0, 1.0],
                        values: vec![Quat::IDENTITY, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)],
                        interp: Interp::Linear,
                    }),
                    ..Default::default()
                },
                JointTracks::default(), // child un-animated → rest
            ],
        };
        let pose = sample_pose(&skel, &clip, 1.0);
        assert!(pose[0].rotation.abs_diff_eq(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), 1e-5));
        // Child falls back to its rest translation.
        assert!(pose[1].translation.abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-6));
    }

    #[test]
    fn blend_is_endpoint_exact() {
        let a = vec![LocalTransform { translation: Vec3::ZERO, ..LocalTransform::IDENTITY }];
        let b = vec![LocalTransform { translation: Vec3::new(2.0, 0.0, 0.0), ..LocalTransform::IDENTITY }];
        assert!(blend_poses(&a, &b, 0.0)[0].translation.abs_diff_eq(Vec3::ZERO, 1e-6));
        assert!(blend_poses(&a, &b, 1.0)[0].translation.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-6));
        assert!(blend_poses(&a, &b, 0.5)[0].translation.abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-6));
    }
}
