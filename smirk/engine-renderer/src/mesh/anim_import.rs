use crate::anim::{AnimationClip, Interp, Joint, JointTracks, LocalTransform, Skeleton, Track};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;

/// Build the skeleton from the file's first skin. Returns the skeleton plus a
/// `glTF node index → joint index` map used to route animation channels.
/// `None` for a static (skin-less) file.
pub(crate) fn extract_skeleton(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
    path:    &str,
) -> Option<(Skeleton, HashMap<usize, usize>)> {
    let skin = doc.skins().next()?;
    if doc.skins().count() > 1 {
        log::warn!("{path}: multiple skins — using the first");
    }

    let joint_nodes: Vec<gltf::Node> = skin.joints().collect();
    let node_to_joint: HashMap<usize, usize> = joint_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.index(), i))
        .collect();

    let reader = skin.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
    let ibms: Vec<Mat4> = reader
        .read_inverse_bind_matrices()
        .map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect())
        .unwrap_or_else(|| vec![Mat4::IDENTITY; joint_nodes.len()]);

    // child node index → parent node index, over every node in the document.
    let mut parent_of: HashMap<usize, usize> = HashMap::new();
    for node in doc.nodes() {
        for child in node.children() {
            parent_of.insert(child.index(), node.index());
        }
    }

    let joints: Vec<Joint> = joint_nodes
        .iter()
        .enumerate()
        .map(|(i, jn)| {
            // Parent = nearest ancestor that is itself a joint (skips a
            // non-joint "Armature" root above the bones).
            let mut cur = parent_of.get(&jn.index()).copied();
            let parent = loop {
                match cur {
                    Some(pn) => match node_to_joint.get(&pn) {
                        Some(&pj) => break Some(pj),
                        None => cur = parent_of.get(&pn).copied(),
                    },
                    None => break None,
                }
            };
            let (t, r, s) = jn.transform().decomposed();
            Joint {
                parent,
                inverse_bind: ibms.get(i).copied().unwrap_or(Mat4::IDENTITY),
                rest: LocalTransform {
                    translation: Vec3::from(t),
                    rotation:    Quat::from_array(r),
                    scale:       Vec3::from(s),
                },
                name: jn.name().unwrap_or_default().to_string(),
            }
        })
        .collect();

    // Root offset: the world transform of the non-joint nodes the bones hang
    // under (an exporter's "Armature"/"Rig" node, which may carry a scale or a
    // ground offset). Fold it into every root joint's global so an armature
    // authored at 2 m or grounded to the floor loads correctly. Taken from the
    // first root joint's ancestor chain (a single armature is the norm).
    let all_nodes: Vec<gltf::Node> = doc.nodes().collect();
    let root = joints
        .iter()
        .zip(joint_nodes.iter())
        .find(|(j, _)| j.parent.is_none())
        .map(|(_, top)| {
            let mut chain = Vec::new(); // immediate parent → … → scene root
            let mut cur = parent_of.get(&top.index()).copied();
            while let Some(n) = cur {
                chain.push(n);
                cur = parent_of.get(&n).copied();
            }
            chain.iter().rev().fold(Mat4::IDENTITY, |acc, &n| {
                acc * Mat4::from_cols_array_2d(&all_nodes[n].transform().matrix())
            })
        })
        .unwrap_or(Mat4::IDENTITY);

    Some((Skeleton { joints, root }, node_to_joint))
}

/// Read every animation into per-joint keyframe tracks. Channels targeting
/// non-joint nodes are ignored (we only skin the skeleton).
pub(crate) fn extract_clips(
    doc:           &gltf::Document,
    buffers:       &[gltf::buffer::Data],
    node_to_joint: &HashMap<usize, usize>,
    joint_count:   usize,
) -> Vec<AnimationClip> {
    use gltf::animation::util::ReadOutputs;
    use gltf::animation::Interpolation;

    doc.animations()
        .enumerate()
        .map(|(ai, anim)| {
            let mut tracks = vec![JointTracks::default(); joint_count];
            let mut duration = 0.0f32;

            for channel in anim.channels() {
                let node_idx = channel.target().node().index();
                let Some(&jx) = node_to_joint.get(&node_idx) else { continue };

                let raw_interp = channel.sampler().interpolation();
                let interp = match raw_interp {
                    Interpolation::Step => Interp::Step,
                    Interpolation::Linear | Interpolation::CubicSpline => Interp::Linear,
                };
                let is_cubic = raw_interp == Interpolation::CubicSpline;

                let reader = channel.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let times: Vec<f32> = match reader.read_inputs() {
                    Some(i) => i.collect(),
                    None => continue,
                };
                if let Some(&last) = times.last() {
                    duration = duration.max(last);
                }

                match reader.read_outputs() {
                    Some(ReadOutputs::Translations(it)) => {
                        let vals = keyframe_values(it.map(Vec3::from).collect(), times.len(), is_cubic);
                        tracks[jx].translation = Some(Track { times, values: vals, interp });
                    }
                    Some(ReadOutputs::Rotations(it)) => {
                        let vals = keyframe_values(
                            it.into_f32().map(Quat::from_array).collect(),
                            times.len(),
                            is_cubic,
                        );
                        tracks[jx].rotation = Some(Track { times, values: vals, interp });
                    }
                    Some(ReadOutputs::Scales(it)) => {
                        let vals = keyframe_values(it.map(Vec3::from).collect(), times.len(), is_cubic);
                        tracks[jx].scale = Some(Track { times, values: vals, interp });
                    }
                    _ => {} // morph-target weights: not skinned
                }
            }

            AnimationClip {
                name: anim.name().map(str::to_owned).unwrap_or_else(|| format!("anim{ai}")),
                duration,
                tracks,
            }
        })
        .collect()
}

/// CUBICSPLINE outputs store (in-tangent, value, out-tangent) per keyframe.
/// Downsampled to Linear, only the middle value survives.
fn keyframe_values<T: Copy>(vals: Vec<T>, times_len: usize, is_cubic: bool) -> Vec<T> {
    if is_cubic && vals.len() == 3 * times_len {
        (0..times_len).map(|i| vals[3 * i + 1]).collect()
    } else {
        vals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{joint_matrices, sample_pose};

    #[test]
    fn loads_skinned_animated_glb() {
        let path = std::env::temp_dir().join("vordar_anim_test_skinned.glb");
        crate::mesh::test_glb::write_skinned_glb(&path);
        let (doc, buffers, _images) =
            gltf::import(path.to_str().unwrap()).expect("glb imports");
        let data = extract_skeleton(&doc, &buffers, path.to_str().unwrap());

        // Skeleton: two joints, child parented to root.
        let (skel, node_to_joint) = data.expect("skinned mesh has a skeleton");
        assert_eq!(skel.joint_count(), 2);
        assert_eq!(skel.joints[0].parent, None);
        assert_eq!(skel.joints[1].parent, Some(0));
        // Child inverse bind is translate(0,-1,0).
        assert!(skel.joints[1].inverse_bind
            .abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)), 1e-5));

        // Clips: one animation, ~1s, with a rotation track on the root joint.
        let clips = extract_clips(&doc, &buffers, &node_to_joint, skel.joint_count());
        assert_eq!(clips.len(), 1);
        let clip = &clips[0];
        assert_eq!(clip.name, "Spin");
        assert!((clip.duration - 1.0).abs() < 1e-5);
        assert!(clip.tracks[0].rotation.is_some(), "root joint is animated");

        // End of clip: root rotated 90° about Z. A bind-space point at the
        // child (0,1,0), skinned by the child joint matrix, swings to (-1,0,0).
        let pose = sample_pose(&skel, clip, clip.duration);
        let mats = joint_matrices(&skel, &pose);
        let skinned = mats[1].transform_point3(Vec3::new(0.0, 1.0, 0.0));
        assert!(skinned.abs_diff_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-4), "got {skinned}");
    }
}
