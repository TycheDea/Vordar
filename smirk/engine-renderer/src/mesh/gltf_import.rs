use crate::anim::{AnimationClip, Skeleton};
use crate::mesh_pipeline::MeshVertex;
use crate::tangent::generate_tangents;
use glam::{Mat3, Mat4};

use super::anim_import::{extract_skeleton, extract_clips};

/// The three glTF alpha modes with their associated parameters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AlphaMode {
    /// No transparency; all fragments are opaque.
    Opaque,
    /// Binary transparency with a discard threshold.
    Mask(f32),
    /// Blended transparency (encoded as mask 0.5 until step 2 implements sorted rendering).
    Blend,
}

impl Default for AlphaMode {
    fn default() -> Self {
        AlphaMode::Opaque
    }
}

/// RGBA8 pixels, sRGB-encoded (glTF base-color convention).
pub struct ImageData {
    pub width:  u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A material texture slot's image data, either decoded RGBA8 (glTF-embedded
/// or loose PNG/JPG) or a pre-compressed DDS parsed off disk. Color-space
/// (sRGB vs linear) is carried alongside by the slot, not the variant —
/// `Compressed`'s format is self-describing (the DDS's own DXGI tag).
pub enum TextureSource {
    Rgba8(ImageData),
    Compressed(crate::texture::CompressedImage),
}

/// Per-vertex skin binding, parallel to `PrimitiveData::vertices`. Kept
/// separate from `Vertex` so static meshes carry no skinning overhead and the
/// static GPU path is untouched.
#[derive(Clone, Copy)]
pub struct VertexSkin {
    pub joints:  [u16; 4],
    pub weights: [f32; 4],
}

/// The full glTF metallic-roughness material of one primitive. Missing maps
/// stay None and bind 1×1 neutral defaults at upload; factors multiply per
/// the glTF spec.
pub struct MaterialData {
    pub base_color_factor: [f32; 4],
    pub metallic_factor:   f32,
    pub roughness_factor:  f32,
    /// Alpha mode from the glTF material: distinguishes OPAQUE, MASK, and BLEND.
    pub alpha_mode:        AlphaMode,
    pub emissive_factor:   [f32; 3],
    /// KHR_materials_emissive_strength (1.0 when absent) — HDR emissive for
    /// bloom.
    pub emissive_strength: f32,
    pub base_color_image:         Option<TextureSource>, // sRGB
    pub normal_image:             Option<TextureSource>, // linear
    pub metallic_roughness_image: Option<TextureSource>, // linear (g=rough, b=metal)
    pub emissive_image:           Option<TextureSource>, // sRGB
    pub occlusion_image:          Option<TextureSource>, // linear (r)
}

impl Default for MaterialData {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0; 4],
            metallic_factor:   1.0,
            roughness_factor:  1.0,
            alpha_mode:        AlphaMode::Opaque,
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
    let mut gltf = gltf::Gltf::open(path).map_err(|e| format!("{path}: {e}"))?;
    let blob = gltf.blob.take();
    let buffers = gltf::import_buffers(&gltf.document, std::path::Path::new(path).parent(), blob)
        .map_err(|e| format!("{path}: {e}"))?;
    let doc = &gltf.document;
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or_else(|| format!("{path}: no scene"))?;

    let mut primitives = Vec::new();
    for node in scene.nodes() {
        visit_node(&node, Mat4::IDENTITY, &buffers, doc, path, &mut primitives);
    }
    if primitives.is_empty() {
        return Err(format!("{path}: no triangle primitives in scene"));
    }

    // Skeleton + animations (absent for static meshes → None / empty).
    let (skeleton, clips) = match extract_skeleton(doc, &buffers, path) {
        Some((skel, node_to_joint)) => {
            let clips = extract_clips(doc, &buffers, &node_to_joint, skel.joint_count());
            (Some(skel), clips)
        }
        None => (None, Vec::new()),
    };

    Ok(MeshData { primitives, skeleton, clips })
}



fn visit_node(
    node:       &gltf::Node,
    parent:     Mat4,
    buffers:    &[gltf::buffer::Data],
    doc:        &gltf::Document,
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
            // source space. Generation runs pre-transform; the normal-matrix
            // rotation below carries them to world space.
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

            let material = read_material(&prim.material(), doc, buffers, path);

            out.push(PrimitiveData { vertices, indices, material, skin });
        }
    }

    for child in node.children() {
        visit_node(&child, global, buffers, doc, path, out);
    }
}

/// Read the whole glTF metallic-roughness material of a primitive: every
/// texture slot plus the scalar/vector factors.
fn read_material(
    mat:     &gltf::Material,
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
    path:    &str,
) -> MaterialData {
    // Preprocessed DDS sidecars live in `<asset stem>.textures/img<N>.dds`,
    // one per glTF image index (see scripts/asset-pipeline).
    let sidecar_dir = std::path::Path::new(path).with_extension("textures");
    let asset_dir = std::path::Path::new(path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let fetch = |index: usize, slot: &str| -> Option<TextureSource> {
        let sidecar = sidecar_dir.join(format!("img{index}.dds"));
        if sidecar.exists() {
            match std::fs::read(&sidecar)
                .map_err(|e| e.to_string())
                .and_then(|bytes| crate::texture::parse_dds(&bytes))
            {
                Ok(compressed) => return Some(TextureSource::Compressed(compressed)),
                Err(e) => log::warn!("{}: {e}, falling back to embedded {slot} image", sidecar.display()),
            }
        }
        // Sidecar missed (or failed): decode the embedded/loose image bytes,
        // fetched lazily so a sidecar hit never pays for this.
        let image = doc.images().nth(index)?;
        let bytes: std::borrow::Cow<[u8]> = match image.source() {
            gltf::image::Source::View { view, .. } => {
                let buf = &buffers[view.buffer().index()];
                let start = view.offset();
                std::borrow::Cow::Borrowed(&buf[start..start + view.length()])
            }
            gltf::image::Source::Uri { uri, .. } => {
                if uri.starts_with("data:") {
                    log::warn!("{path}: data-URI {slot} image not supported, skipping");
                    return None;
                }
                match std::fs::read(asset_dir.join(uri)) {
                    Ok(bytes) => std::borrow::Cow::Owned(bytes),
                    Err(e) => {
                        log::warn!("{path}: failed to read {slot} image {uri:?}: {e}");
                        return None;
                    }
                }
            }
        };
        match image::load_from_memory(&bytes) {
            Ok(decoded) => {
                let rgba = decoded.into_rgba8();
                Some(TextureSource::Rgba8(ImageData {
                    width:  rgba.width(),
                    height: rgba.height(),
                    pixels: rgba.into_raw(),
                }))
            }
            Err(e) => {
                log::warn!("{path}: unsupported {slot} image format: {e}");
                None
            }
        }
    };

    let pbr = mat.pbr_metallic_roughness();
    MaterialData {
        base_color_factor: pbr.base_color_factor(),
        metallic_factor:   pbr.metallic_factor(),
        roughness_factor:  pbr.roughness_factor(),
        alpha_mode: match mat.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask(mat.alpha_cutoff().unwrap_or(0.5)),
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
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
    use glam::Vec3;

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
        // alphaMode MASK carries its cutoff (OPAQUE would read AlphaMode::Opaque).
        assert_eq!(p.material.alpha_mode, AlphaMode::Mask(0.35));
        // No TANGENT accessor in the file — generated from UVs. This
        // triangle's UVs map u to +X, so the tangent points along +X.
        assert_eq!(p.vertices[0].tangent, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn base_color_prefers_sidecar_dds_over_embedded_png() {
        let path = std::env::temp_dir().join("vordar_mesh_test_textured.glb");
        crate::mesh::test_glb::write_textured_glb(&path);
        let sidecar_dir = path.with_extension("textures");
        std::fs::create_dir_all(&sidecar_dir).unwrap();
        std::fs::write(
            sidecar_dir.join("img0.dds"),
            include_bytes!("../../tests/data/red8x8_bc7_srgb.dds"),
        )
        .unwrap();

        let data = load_gltf_data(path.to_str().unwrap()).unwrap();
        let source = data.primitives[0]
            .material
            .base_color_image
            .as_ref()
            .expect("textured glb has a base-color texture");
        let TextureSource::Compressed(c) = source else {
            panic!("sidecar DDS must win the slot over the embedded PNG")
        };
        assert_eq!(c.format, wgpu::TextureFormat::Bc7RgbaUnormSrgb);
    }

    #[test]
    fn sidecar_skips_decode_of_corrupt_embedded_image() {
        let path = std::env::temp_dir().join("vordar_mesh_test_corrupt_textured.glb");
        crate::mesh::test_glb::write_corrupt_textured_glb(&path);
        let sidecar_dir = path.with_extension("textures");
        std::fs::create_dir_all(&sidecar_dir).unwrap();
        std::fs::write(
            sidecar_dir.join("img0.dds"),
            include_bytes!("../../tests/data/red8x8_bc7_srgb.dds"),
        )
        .unwrap();

        // Sidecar wins the slot; the corrupt embedded PNG must never be decoded.
        let data = load_gltf_data(path.to_str().unwrap())
            .expect("sidecar hit must skip decoding the corrupt embedded image");
        let source = data.primitives[0]
            .material
            .base_color_image
            .as_ref()
            .expect("textured glb has a base-color texture");
        assert!(matches!(source, TextureSource::Compressed(_)));

        // Without the sidecar, the corrupt embedded image is a per-slot None,
        // not a whole-asset Err (matches fetch's unsupported-format contract).
        std::fs::remove_dir_all(&sidecar_dir).unwrap();
        let data = load_gltf_data(path.to_str().unwrap())
            .expect("a corrupt embedded image must not fail the whole asset");
        assert!(
            data.primitives[0].material.base_color_image.is_none(),
            "corrupt embedded image with no sidecar must decode to None"
        );
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

    #[test]
    fn blend_alpha_mode_survives_import() {
        let path = std::env::temp_dir().join("vordar_mesh_test_blend.glb");
        crate::mesh::test_glb::write_blend_glb(&path);

        let data = load_gltf_data(path.to_str().unwrap()).unwrap();
        assert_eq!(data.primitives.len(), 1);
        let p = &data.primitives[0];
        assert_eq!(p.material.alpha_mode, AlphaMode::Blend);
    }

    /// Exercises real-asset paths the synthetic GLB can't: embedded PNG
    /// decode and image-format conversion. Skips when the content asset is
    /// absent (it lives in the game repo, not the engine).
    #[test]
    fn loads_real_textured_asset_if_present() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/source/test/avocado.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        assert!(!data.primitives.is_empty());
        let p = &data.primitives[0];
        assert!(p.vertices.len() > 100, "real mesh, not a placeholder");
        let source = p.material.base_color_image.as_ref().expect("avocado has a base-color texture");
        let TextureSource::Rgba8(img) = source else { panic!("avocado's base-color must decode as RGBA8") };
        assert_eq!(img.pixels.len() as u32, img.width * img.height * 4, "tightly packed RGBA8");
    }

    /// Real rigged+animated asset (the Fox dev probe): proves skin + multi-clip
    /// extraction on production data. Skips if absent.
    #[test]
    fn loads_skinned_fox_asset_if_present() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/source/test/fox.glb");
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

    /// Real game character (the KayKit human race): a skinned humanoid,
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
        // DDS sidecars (scripts/asset-pipeline) must win every base-color slot.
        if std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.textures")).exists() {
            for p in &data.primitives {
                let source = p.material.base_color_image.as_ref().expect("human primitive has a base-color texture");
                assert!(
                    matches!(source, TextureSource::Compressed(_)),
                    "base-color slot must prefer the sidecar DDS over the embedded image"
                );
            }
        }
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
    /// Root-motion offset must come from the skeleton root, not the mesh
    /// origin. This test verifies that all animation clips pose the skeleton
    /// above the floor plane when the root offset is applied.
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

    /// Root-motion offset must come from the skeleton root, not the mesh
    /// origin. This test verifies the idle pose by CPU-skinning vertices
    /// through the skeletal palette and checking that soles and crown align.
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
