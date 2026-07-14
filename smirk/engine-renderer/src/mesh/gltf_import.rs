use crate::anim::{AnimationClip, Interp, Joint, JointTracks, LocalTransform, Skeleton, Track};
use crate::mesh_pipeline::MeshVertex;
use crate::tangent::generate_tangents;
use glam::{Mat3, Mat4, Quat, Vec3};
use std::collections::HashMap;

/// RGBA8 pixels, sRGB-encoded (glTF base-color convention).
pub struct ImageData {
    pub width:  u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Per-vertex skin binding, parallel to `PrimitiveData::vertices`. Kept
/// separate from `Vertex` so static meshes carry no skinning overhead and the
/// static GPU path is untouched.
#[derive(Clone, Copy)]
pub struct VertexSkin {
    pub joints:  [u16; 4],
    pub weights: [f32; 4],
}

/// The full glTF metallic-roughness material of one primitive (VQ-A2/C2/C4).
/// Missing maps stay None and bind 1×1 neutral defaults at upload; factors
/// multiply per the glTF spec.
pub struct MaterialData {
    pub base_color_factor: [f32; 4],
    pub metallic_factor:   f32,
    pub roughness_factor:  f32,
    /// Fragment-discard threshold from glTF alphaMode: 0.0 for OPAQUE (never
    /// discards), the cutoff for MASK. BLEND is approximated as MASK at 0.5 —
    /// there is no sorted transparency pass.
    pub alpha_cutoff:      f32,
    pub emissive_factor:   [f32; 3],
    /// KHR_materials_emissive_strength (1.0 when absent) — HDR emissive for
    /// bloom (VQ-C3).
    pub emissive_strength: f32,
    pub base_color_image:         Option<ImageData>, // sRGB
    pub normal_image:             Option<ImageData>, // linear
    pub metallic_roughness_image: Option<ImageData>, // linear (g=rough, b=metal)
    pub emissive_image:           Option<ImageData>, // sRGB
    pub occlusion_image:          Option<ImageData>, // linear (r)
}

impl Default for MaterialData {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0; 4],
            metallic_factor:   1.0,
            roughness_factor:  1.0,
            alpha_cutoff:      0.0,
            emissive_factor:   [0.0; 3],
            emissive_strength: 1.0,
            base_color_image:         None,
            normal_image:             None,
            metallic_roughness_image: None,
            emissive_image:           None,
            occlusion_image:          None,
        }
    }
}

pub struct PrimitiveData {
    pub vertices: Vec<MeshVertex>,
    pub indices:  Vec<u32>,
    pub material: MaterialData,
    /// Present iff this primitive is skinned (node had a skin). When Some,
    /// `vertices` are in mesh-local space (node transform NOT baked) — the
    /// joint palette places them.
    pub skin: Option<Vec<VertexSkin>>,
}

pub struct MeshData {
    pub primitives: Vec<PrimitiveData>,
    /// The mesh's skeleton, if any primitive is skinned. One skin per file
    /// (our character assets); a second skin is ignored with a warning.
    pub skeleton: Option<Skeleton>,
    /// Named animation clips, indexed-by-joint tracks. Empty for static meshes.
    pub clips: Vec<AnimationClip>,
}

/// Parse a .glb/.gltf file into CPU-side mesh data. Node hierarchy transforms
/// are baked into the vertices (static meshes only — skinning comes later),
/// so one MeshData draws with a single model matrix per instance.
pub fn load_gltf_data(path: &str) -> Result<MeshData, String> {
    let (doc, buffers, images) =
        gltf::import(path).map_err(|e| format!("{path}: {e}"))?;
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or_else(|| format!("{path}: no scene"))?;

    let mut primitives = Vec::new();
    for node in scene.nodes() {
        visit_node(&node, Mat4::IDENTITY, &buffers, &images, path, &mut primitives);
    }
    if primitives.is_empty() {
        return Err(format!("{path}: no triangle primitives in scene"));
    }

    // Skeleton + animations (absent for static meshes → None / empty).
    let (skeleton, clips) = match extract_skeleton(&doc, &buffers, path) {
        Some((skel, node_to_joint)) => {
            let clips = extract_clips(&doc, &buffers, &node_to_joint, skel.joint_count());
            (Some(skel), clips)
        }
        None => (None, Vec::new()),
    };

    Ok(MeshData { primitives, skeleton, clips })
}

/// Build the skeleton from the file's first skin. Returns the skeleton plus a
/// `glTF node index → joint index` map used to route animation channels.
/// `None` for a static (skin-less) file.
fn extract_skeleton(
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
fn extract_clips(
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

fn visit_node(
    node:       &gltf::Node,
    parent:     Mat4,
    buffers:    &[gltf::buffer::Data],
    images:     &[gltf::image::Data],
    path:       &str,
    out:        &mut Vec<PrimitiveData>,
) {
    let global = parent * Mat4::from_cols_array_2d(&node.transform().matrix());

    if let Some(mesh) = node.mesh() {
        // Skinned primitives stay in mesh-local space (identity vertex
        // transform) — the joint palette does the placement. Static ones bake
        // the node hierarchy into their vertices as before.
        let skinned = node.skin().is_some();
        let vtx_xform  = if skinned { Mat4::IDENTITY } else { global };
        let normal_mat = if skinned { Mat3::IDENTITY } else { Mat3::from_mat4(global).inverse().transpose() };

        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                log::warn!("{path}: skipping non-triangle primitive ({:?})", prim.mode());
                continue;
            }
            let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(positions) = reader.read_positions() else {
                log::warn!("{path}: primitive without positions skipped");
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|n| n.collect())
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|t| t.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

            let indices: Vec<u32> = reader
                .read_indices()
                .map(|i| i.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());

            // Tangents: from the asset when present, otherwise generated in
            // source space (VQ-C4). Generation runs pre-transform; the
            // normal-matrix rotation below carries them to world space.
            let tangents: Vec<[f32; 4]> = reader
                .read_tangents()
                .map(|t| t.collect())
                .unwrap_or_else(|| generate_tangents(&positions, &normals, &uvs, &indices));

            let vertices: Vec<MeshVertex> = positions
                .iter()
                .zip(normals.iter())
                .zip(uvs.iter())
                .zip(tangents.iter())
                .map(|(((p, n), uv), t)| MeshVertex {
                    position: vtx_xform.transform_point3((*p).into()).to_array(),
                    normal:   (normal_mat * glam::Vec3::from(*n))
                        .normalize_or_zero()
                        .to_array(),
                    uv: *uv,
                    tangent: {
                        let txyz = (normal_mat * glam::Vec3::new(t[0], t[1], t[2]))
                            .normalize_or_zero();
                        [txyz.x, txyz.y, txyz.z, t[3]]
                    },
                })
                .collect();

            // Skin bindings (joints + weights), only for skinned primitives.
            let skin = if skinned {
                let joints = reader.read_joints(0).map(|j| j.into_u16().collect::<Vec<[u16; 4]>>());
                let weights = reader.read_weights(0).map(|w| w.into_f32().collect::<Vec<[f32; 4]>>());
                match (joints, weights) {
                    (Some(j), Some(w)) => Some(
                        j.iter()
                            .zip(w.iter())
                            .map(|(&joints, &weights)| VertexSkin { joints, weights })
                            .collect(),
                    ),
                    _ => {
                        log::warn!("{path}: skinned primitive missing JOINTS_0/WEIGHTS_0");
                        None
                    }
                }
            } else {
                None
            };

            let material = read_material(&prim.material(), images, path);

            out.push(PrimitiveData { vertices, indices, material, skin });
        }
    }

    for child in node.children() {
        visit_node(&child, global, buffers, images, path, out);
    }
}

/// Read the whole glTF metallic-roughness material of a primitive (VQ-A2):
/// every texture slot plus the scalar/vector factors.
fn read_material(
    mat:    &gltf::Material,
    images: &[gltf::image::Data],
    path:   &str,
) -> MaterialData {
    let fetch = |index: usize, slot: &str| -> Option<ImageData> {
        let img = images.get(index)?;
        let converted = to_rgba8(img);
        if converted.is_none() {
            log::warn!("{path}: unsupported {slot} image format {:?}", img.format);
        }
        converted
    };

    let pbr = mat.pbr_metallic_roughness();
    MaterialData {
        base_color_factor: pbr.base_color_factor(),
        metallic_factor:   pbr.metallic_factor(),
        roughness_factor:  pbr.roughness_factor(),
        alpha_cutoff: match mat.alpha_mode() {
            gltf::material::AlphaMode::Opaque => 0.0,
            gltf::material::AlphaMode::Mask => mat.alpha_cutoff().unwrap_or(0.5),
            gltf::material::AlphaMode::Blend => 0.5,
        },
        emissive_factor:   mat.emissive_factor(),
        emissive_strength: mat.emissive_strength().unwrap_or(1.0),
        base_color_image: pbr
            .base_color_texture()
            .and_then(|i| fetch(i.texture().source().index(), "base-color")),
        normal_image: mat
            .normal_texture()
            .and_then(|i| fetch(i.texture().source().index(), "normal")),
        metallic_roughness_image: pbr
            .metallic_roughness_texture()
            .and_then(|i| fetch(i.texture().source().index(), "metallic-roughness")),
        emissive_image: mat
            .emissive_texture()
            .and_then(|i| fetch(i.texture().source().index(), "emissive")),
        occlusion_image: mat
            .occlusion_texture()
            .and_then(|i| fetch(i.texture().source().index(), "occlusion")),
    }
}

/// Convert a decoded glTF image to tightly-packed RGBA8. 16-bit and float
/// formats are not supported (None).
fn to_rgba8(img: &gltf::image::Data) -> Option<ImageData> {
    use gltf::image::Format;
    let pixels = match img.format {
        Format::R8G8B8A8 => img.pixels.clone(),
        Format::R8G8B8 => img.pixels.chunks_exact(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        Format::R8 => img.pixels.iter()
            .flat_map(|&g| [g, g, g, 255])
            .collect(),
        Format::R8G8 => img.pixels.chunks_exact(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        _ => return None,
    };
    Some(ImageData { width: img.width, height: img.height, pixels })
}

/// Decode a PNG/JPG from disk into tightly-packed RGBA8 — the seam for
/// building procedural `MaterialData` (ground texture sets) outside glTF.
pub fn load_image_rgba(path: &str) -> Result<ImageData, String> {
    let img = image::open(path).map_err(|e| format!("{path}: {e}"))?.into_rgba8();
    Ok(ImageData {
        width:  img.width(),
        height: img.height(),
        pixels: img.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{joint_matrices, sample_pose};
    use engine_core::components::AnimationPlayer;

    #[test]
    fn loads_triangle_glb_with_baked_node_transform() {
        let path = std::env::temp_dir().join("vordar_mesh_test_triangle.glb");
        crate::mesh::test_glb::write_test_glb(&path);

        let data = load_gltf_data(path.to_str().unwrap()).unwrap();
        assert_eq!(data.primitives.len(), 1);
        let p = &data.primitives[0];
        assert_eq!(p.vertices.len(), 3);
        assert_eq!(p.indices, vec![0, 1, 2]);
        // Node translation (1,2,3) must be baked into positions.
        assert_eq!(p.vertices[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(p.vertices[1].position, [2.0, 2.0, 3.0]);
        // Normals survive (pure translation — unchanged).
        assert_eq!(p.vertices[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(p.vertices[2].uv, [0.0, 1.0]);
        // Solid-color material, no texture.
        assert_eq!(p.material.base_color_factor, [0.2, 0.4, 0.8, 1.0]);
        assert!(p.material.base_color_image.is_none());
        // alphaMode MASK carries its cutoff (OPAQUE would read 0.0).
        assert_eq!(p.material.alpha_cutoff, 0.35);
        // No TANGENT accessor in the file — generated from UVs. This
        // triangle's UVs map u to +X, so the tangent points along +X.
        assert_eq!(p.vertices[0].tangent, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(load_gltf_data("does/not/exist.glb").is_err());
    }

    #[test]
    fn loads_skinned_animated_glb() {
        let path = std::env::temp_dir().join("vordar_mesh_test_skinned.glb");
        crate::mesh::test_glb::write_skinned_glb(&path);
        let data = load_gltf_data(path.to_str().unwrap()).unwrap();

        // Skeleton: two joints, child parented to root.
        let skel = data.skeleton.as_ref().expect("skinned mesh has a skeleton");
        assert_eq!(skel.joint_count(), 2);
        assert_eq!(skel.joints[0].parent, None);
        assert_eq!(skel.joints[1].parent, Some(0));
        // Child inverse bind is translate(0,-1,0).
        assert!(skel.joints[1].inverse_bind
            .abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)), 1e-5));

        // Skin bindings present; vertices are NOT baked (skinned → mesh-local).
        let p = &data.primitives[0];
        let skin = p.skin.as_ref().expect("skinned primitive carries joints/weights");
        assert_eq!(skin.len(), 3);
        assert_eq!(skin[0].joints[0], 0);
        assert_eq!(skin[1].joints[0], 1);
        assert_eq!(p.vertices[1].position, [0.0, 1.0, 0.0], "skinned verts stay in local space");

        // Clip: one animation, ~1s, with a rotation track on the root joint.
        assert_eq!(data.clips.len(), 1);
        let clip = &data.clips[0];
        assert_eq!(clip.name, "Spin");
        assert!((clip.duration - 1.0).abs() < 1e-5);
        assert!(clip.tracks[0].rotation.is_some(), "root joint is animated");

        // End of clip: root rotated 90° about Z. A bind-space point at the
        // child (0,1,0), skinned by the child joint matrix, swings to (-1,0,0).
        let pose = sample_pose(skel, clip, clip.duration);
        let mats = joint_matrices(skel, &pose);
        let skinned = mats[1].transform_point3(Vec3::new(0.0, 1.0, 0.0));
        assert!(skinned.abs_diff_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-4), "got {skinned}");
    }

    /// Exercises real-asset paths the synthetic GLB can't: embedded PNG
    /// decode and image-format conversion. Skips when the content asset is
    /// absent (it lives in the game repo, not the engine).
    #[test]
    fn loads_real_textured_asset_if_present() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/avocado.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        assert!(!data.primitives.is_empty());
        let p = &data.primitives[0];
        assert!(p.vertices.len() > 100, "real mesh, not a placeholder");
        let img = p.material.base_color_image.as_ref().expect("avocado has a base-color texture");
        assert_eq!(img.pixels.len() as u32, img.width * img.height * 4, "tightly packed RGBA8");
    }

    /// Real rigged+animated asset (the Fox dev probe): proves skin + multi-clip
    /// extraction on production data. Skips if absent.
    #[test]
    fn loads_skinned_fox_asset_if_present() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/fox.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().expect("fox is skinned");
        assert!(skel.joint_count() > 10, "real skeleton");
        assert!(data.primitives[0].skin.is_some(), "vertices carry skin bindings");
        // Named locomotion clips the controller maps to idle/walk/run.
        let names: Vec<&str> = data.clips.iter().map(|c| c.name.as_str()).collect();
        for want in ["Survey", "Walk", "Run"] {
            assert!(names.contains(&want), "expected clip {want:?}, got {names:?}");
        }
        // Every clip poses to a full, finite joint palette.
        for clip in &data.clips {
            let pose = sample_pose(skel, clip, clip.duration * 0.5);
            let mats = joint_matrices(skel, &pose);
            assert_eq!(mats.len(), skel.joint_count());
            assert!(mats.iter().all(|m| m.is_finite()), "clip {} produced NaN", clip.name);
        }
    }

    /// Real game character (the KayKit human race): a Phase-C skinned humanoid,
    /// preprocessed so its armature carries a scale + ground offset. Proves the
    /// clip mapping the locomotion controller expects, and that the root offset
    /// baked onto the armature is picked up (feet grounded, character scaled).
    #[test]
    fn loads_human_character_asset_if_present() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().expect("human is skinned");
        assert!(skel.joint_count() > 20, "full humanoid skeleton");
        // Every primitive is skinned (body + face meshes).
        assert!(data.primitives.len() >= 2, "body plus face primitives");
        assert!(
            data.primitives.iter().all(|p| p.skin.is_some()),
            "all primitives carry skin bindings"
        );
        // Locomotion, the per-ability attack clips, hit react, and death
        // (the Mixamo clip library merged by mixamo_to_glb.py).
        let names: Vec<&str> = data.clips.iter().map(|c| c.name.as_str()).collect();
        for want in [
            "idle",
            "walk",
            "run",
            "attack_slash",
            "attack_heavy",
            "attack_cast",
            "hit",
            "death",
            "leap",
            "dodge",
        ] {
            assert!(names.contains(&want), "expected clip {want:?}, got {names:?}");
        }
        // The armature offset was captured: a non-identity root that scales down
        // (< 1) and drops the character so its feet sit on the floor (y < 0).
        assert!(!skel.root.abs_diff_eq(Mat4::IDENTITY, 1e-4), "armature offset captured");
        assert!(skel.root.w_axis.y < 0.0, "grounded below origin, got {}", skel.root.w_axis.y);
        assert!(skel.root.x_axis.x < 1.0 && skel.root.x_axis.x > 0.0, "scaled down");
        // Every clip poses to a full, finite palette.
        for clip in &data.clips {
            let pose = sample_pose(skel, clip, clip.duration * 0.5);
            let mats = joint_matrices(skel, &pose);
            assert!(mats.iter().all(|m| m.is_finite()), "clip {} produced NaN", clip.name);
        }
    }

    /// DIAGNOSTIC: does advancing an AnimationPlayer on the real human's walk
    /// clip actually move the skeleton? Catches a dead animation path (clips
    /// present but not driving joints, or time not advancing) — the CPU half of
    /// a "character renders but doesn't animate" bug.
    #[test]
    fn human_locomotion_clips_actually_animate() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skin = super::super::CpuSkin { skeleton: data.skeleton.unwrap(), clips: data.clips };
        for clip in ["idle", "walk", "run"] {
            let mut player = AnimationPlayer { clip: clip.into(), ..Default::default() };
            let (a, _) = super::super::pose_player(&mut player, &skin, 0.0);
            let (b, _) = super::super::pose_player(&mut player, &skin, 0.25);
            let moved = a.iter().zip(&b).any(|(x, y)| !x.abs_diff_eq(*y, 1e-4));
            assert!(moved, "clip {clip} must move the skeleton as time advances");
        }
    }

    /// DIAGNOSTIC (the "half under the field" report): with the armature's
    /// baked ground offset applied, no clip may pose any joint meaningfully
    /// below the floor plane (floor top = −0.5, joints sit above the sole).
    /// Catches clips whose ground reference disagrees with the bind pose —
    /// prime suspect for a character rendering half-sunk.
    #[test]
    fn human_clips_stay_above_the_floor() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        const FLOOR: f32 = -0.5;
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().unwrap();
        for clip in &data.clips {
            for i in 0..=4 {
                let t = clip.duration * i as f32 / 4.0;
                let pose = crate::anim::sample_pose(skel, clip, t);
                let globals = crate::anim::global_transforms(skel, &pose);
                let min_y = globals
                    .iter()
                    .map(|g| g.to_scale_rotation_translation().2.y)
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    min_y > FLOOR - 0.35,
                    "clip {:?} at t={t:.2}s poses a joint at y={min_y:.2} — well below the floor",
                    clip.name
                );
            }
        }
    }

    /// DIAGNOSTIC: CPU-skin the actual mesh vertices through the same palette
    /// the GPU gets (root · global · inverse_bind) — the grounded-joints probe
    /// above can't see an inverse-bind inconsistency, this can. The idle pose
    /// must put the soles on the floor (−0.5) and the crown near +1.24.
    #[test]
    fn human_skinned_vertices_stand_on_the_floor() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().unwrap();
        let clip = data.clips.iter().find(|c| c.name == "idle").unwrap();
        let pose = crate::anim::sample_pose(skel, clip, 0.0);
        let palette = crate::anim::joint_matrices(skel, &pose);

        let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
        for p in &data.primitives {
            let skin = p.skin.as_ref().expect("human primitives are skinned");
            for (v, s) in p.vertices.iter().zip(skin) {
                let pos = glam::Vec3::from_array(v.position);
                let mut out = glam::Vec3::ZERO;
                for (j, w) in s.joints.iter().zip(s.weights) {
                    if w > 0.0 {
                        out += palette[*j as usize].transform_point3(pos) * w;
                    }
                }
                min_y = min_y.min(out.y);
                max_y = max_y.max(out.y);
            }
        }
        println!("idle t=0 skinned vertices: y {min_y:.3} .. {max_y:.3}");
        assert!(
            (min_y - (-0.5)).abs() < 0.1,
            "soles must touch the floor: min_y = {min_y:.3}"
        );
        assert!(
            max_y > 0.9 && max_y < 1.5,
            "crown around head height: max_y = {max_y:.3}"
        );
    }

    /// The weapon-hand socket: the human skeleton keeps its bone names, the
    /// hand bone's global transform is finite, and it travels during the swing
    /// clip — the position a cast burst spawns from.
    #[test]
    fn human_hand_socket_exists_and_moves_during_swing() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().unwrap();
        let j = skel
            .joints
            .iter()
            .position(|jt| jt.name == "handslot.r")
            .expect("human skeleton keeps its bone names");
        let clip = data
            .clips
            .iter()
            .find(|c| c.name == "attack_slash")
            .unwrap();
        let at = |t: f32| {
            let pose = crate::anim::sample_pose(skel, clip, t);
            let g = crate::anim::global_transforms(skel, &pose);
            g[j].to_scale_rotation_translation().2
        };
        let (a, b) = (at(0.0), at(clip.duration * 0.5));
        assert!(a.is_finite() && b.is_finite(), "socket transforms must be finite");
        assert!(
            a.distance(b) > 0.05,
            "hand socket must travel during the swing: {a} -> {b}"
        );
    }
}
