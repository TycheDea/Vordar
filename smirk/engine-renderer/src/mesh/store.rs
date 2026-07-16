use super::gltf_import::{ImageData, MeshData, VertexSkin};
use crate::anim::{AnimationClip, Skeleton};
use crate::mesh_pipeline::MaterialUniform;
use crate::mipgen::MipGenerator;
use crate::skinned_pipeline::SkinnedVertex;
use crate::texture::{self, ColorTexture};
use std::collections::HashMap;
use std::sync::mpsc;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{BindGroup, BindGroupLayout, Buffer, BufferUsages, Device, Queue};

pub(crate) struct GpuPrimitive {
    pub(crate) vertex_buffer: Buffer,
    pub(crate) index_buffer:  Buffer,
    pub(crate) index_count:   u32,
    // Textures + factor uniform kept alive alongside their bind group.
    pub(crate) _textures:          Vec<ColorTexture>,
    pub(crate) _material_buffer:   Buffer,
    pub(crate) material_bind_group: BindGroup,
    pub(crate) blend:   bool,
    pub(crate) centroid: glam::Vec3,
}

/// CPU-side animation data kept next to a skinned GpuMesh so sampling needs no
/// GPU access. Present iff the mesh is skinned.
pub(crate) struct CpuSkin {
    pub(crate) skeleton: Skeleton,
    pub(crate) clips:    Vec<AnimationClip>,
}

pub(crate) struct GpuMesh {
    pub(crate) primitives: Vec<GpuPrimitive>,
    /// Some => primitives' vertex buffers hold `SkinnedVertex` and the mesh
    /// draws with the skinned pipeline; None => static-geometry path.
    pub(crate) skin: Option<CpuSkin>,
}

/// One material texture slot: the image (sRGB or linear, mipped) when the
/// asset has one, else a 1×1 neutral default so the bind group is complete.
fn slot_texture(
    device:  &Device,
    queue:   &Queue,
    mipgen:  &MipGenerator,
    image:   &Option<ImageData>,
    srgb:    bool,
    neutral: [u8; 4],
) -> ColorTexture {
    match image {
        Some(img) => texture::create_rgba_texture_mipped(
            device, queue, mipgen, img.width, img.height, &img.pixels, srgb,
        ),
        None => texture::create_rgba_texture(device, queue, 1, 1, &neutral, false),
    }
}

pub(crate) fn upload_mesh(
    device: &Device,
    queue:  &Queue,
    layout: &BindGroupLayout,
    mipgen: &MipGenerator,
    data:   MeshData,
) -> GpuMesh {
    let skinned = data.skeleton.is_some();
    let primitives = data.primitives.iter().map(|p| {
        // Skinned meshes upload SkinnedVertex (adds joints/weights); static
        // meshes upload MeshVertex directly.
        let vertex_buffer = if skinned {
            let verts: Vec<SkinnedVertex> = p.vertices.iter().enumerate().map(|(i, v)| {
                let sk = p.skin.as_ref().map(|s| s[i]).unwrap_or(VertexSkin {
                    joints:  [0, 0, 0, 0],
                    weights: [1.0, 0.0, 0.0, 0.0],
                });
                SkinnedVertex {
                    position: v.position,
                    normal:   v.normal,
                    uv:       v.uv,
                    tangent:  v.tangent,
                    joints:   sk.joints,
                    weights:  sk.weights,
                }
            }).collect();
            device.create_buffer_init(&BufferInitDescriptor {
                label:    Some("Skinned Vertex Buffer"),
                contents: bytemuck::cast_slice(&verts),
                usage:    BufferUsages::VERTEX,
            })
        } else {
            device.create_buffer_init(&BufferInitDescriptor {
                label:    Some("Mesh Vertex Buffer"),
                contents: bytemuck::cast_slice(&p.vertices),
                usage:    BufferUsages::VERTEX,
            })
        };
        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label:    Some("Mesh Index Buffer"),
            contents: bytemuck::cast_slice(&p.indices),
            usage:    BufferUsages::INDEX,
        });

        // The five material textures: sRGB for color-like slots, linear for
        // data-like slots; 1×1 neutral defaults where absent.
        let m = &p.material;
        let albedo   = slot_texture(device, queue, mipgen, &m.base_color_image, true, [255; 4]);
        let normal   = slot_texture(device, queue, mipgen, &m.normal_image, false, [128, 128, 255, 255]);
        let mr       = slot_texture(device, queue, mipgen, &m.metallic_roughness_image, false, [255; 4]);
        let emissive = slot_texture(device, queue, mipgen, &m.emissive_image, true, [255; 4]);
        let ao       = slot_texture(device, queue, mipgen, &m.occlusion_image, false, [255; 4]);

        let (cutoff, blend_w) = match m.alpha_mode {
            super::gltf_import::AlphaMode::Opaque => (0.0, 0.0),
            super::gltf_import::AlphaMode::Mask(c) => (c, 0.0),
            super::gltf_import::AlphaMode::Blend => (0.0, 1.0),
        };
        let blend = m.alpha_mode == super::gltf_import::AlphaMode::Blend;

        let centroid = if p.vertices.is_empty() {
            glam::Vec3::ZERO
        } else {
            let mut min = glam::Vec3::from(p.vertices[0].position);
            let mut max = min;
            for v in &p.vertices {
                let pos = glam::Vec3::from(v.position);
                min = min.min(pos);
                max = max.max(pos);
            }
            (min + max) / 2.0
        };

        let uniform = MaterialUniform {
            base_color: m.base_color_factor,
            emissive: [
                m.emissive_factor[0] * m.emissive_strength,
                m.emissive_factor[1] * m.emissive_strength,
                m.emissive_factor[2] * m.emissive_strength,
                0.0,
            ],
            mr: [m.metallic_factor, m.roughness_factor, cutoff, blend_w],
        };
        let material_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label:    Some("Material Uniform"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage:    BufferUsages::UNIFORM,
        });

        let material_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("Material Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&albedo.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&albedo.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&normal.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&mr.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&emissive.view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&ao.view) },
                wgpu::BindGroupEntry { binding: 6, resource: material_buffer.as_entire_binding() },
            ],
        });

        GpuPrimitive {
            vertex_buffer,
            index_buffer,
            index_count: p.indices.len() as u32,
            _textures: vec![albedo, normal, mr, emissive, ao],
            _material_buffer: material_buffer,
            material_bind_group,
            blend,
            centroid,
        }
    }).collect();

    let skin = data.skeleton.map(|skeleton| CpuSkin { skeleton, clips: data.clips });
    GpuMesh { primitives, skin }
}

/// A path's streaming state. `Pending` while a detached decode thread is in
/// flight; `Failed` once the error is logged, so a bad path isn't retried
/// every frame; `Loaded` once `integrate` has uploaded the result.
#[derive(Debug, PartialEq)]
pub(crate) enum MeshEntry {
    Loaded(usize),
    Pending,
    Failed,
}

/// Loaded meshes keyed by asset path, streamed in on first sight: a miss
/// spawns a background decode and returns nothing to draw until a later
/// `integrate` call uploads the completed `MeshData`.
pub struct MeshStore {
    by_path:           HashMap<String, MeshEntry>,
    pub(crate) meshes: Vec<GpuMesh>,
    results_tx: mpsc::Sender<(String, Result<MeshData, String>)>,
    results_rx: mpsc::Receiver<(String, Result<MeshData, String>)>,
}

impl Default for MeshStore {
    fn default() -> Self {
        let (results_tx, results_rx) = mpsc::channel();
        Self { by_path: HashMap::new(), meshes: Vec::new(), results_tx, results_rx }
    }
}

impl MeshStore {
    /// Upload procedurally-built mesh data under a synthetic key (e.g.
    /// "zone-ground:start"). Re-registering a key uploads fresh data and
    /// replaces the existing slot in place — zone rebuilds replace their
    /// ground without growing `meshes` (indices must stay stable, so nothing
    /// can ever be removed from it).
    pub(crate) fn register(
        &mut self,
        device: &Device,
        queue:  &Queue,
        layout: &BindGroupLayout,
        mipgen: &MipGenerator,
        key:    &str,
        data:   MeshData,
    ) -> usize {
        let mesh = upload_mesh(device, queue, layout, mipgen, data);
        if let Some(&MeshEntry::Loaded(idx)) = self.by_path.get(key) {
            self.meshes[idx] = mesh;
            return idx;
        }
        let idx = self.meshes.len();
        self.meshes.push(mesh);
        self.by_path.insert(key.to_owned(), MeshEntry::Loaded(idx));
        idx
    }

    /// Enqueue a background job producing `MeshData` under `key`; a no-op
    /// when the key already exists in any state (Loaded/Pending/Failed) —
    /// ground data is deterministic per key, so zone re-entry costs nothing.
    /// Uploading, budgeting, and failure logging all ride the same
    /// `integrate` path as `get_or_request`'s streamed glTF loads.
    pub(crate) fn request_job(
        &mut self,
        key:  &str,
        job:  impl FnOnce() -> Result<MeshData, String> + Send + 'static,
    ) {
        if self.by_path.contains_key(key) {
            return;
        }
        self.by_path.insert(key.to_owned(), MeshEntry::Pending);
        let tx = self.results_tx.clone();
        let owned = key.to_owned();
        std::thread::spawn(move || {
            let result = job();
            // App shutdown may have dropped the receiver; nothing to do.
            let _ = tx.send((owned, result));
        });
    }

    /// Look up a loaded mesh by path. A miss marks the path `Pending` and
    /// spawns a detached thread decoding it in the background; `Pending` and
    /// `Failed` both resolve to `None` — the caller renders nothing for the
    /// entity until a later `integrate` call uploads the completed decode.
    pub(crate) fn get_or_request(&mut self, path: &str) -> Option<usize> {
        match self.by_path.get(path) {
            Some(MeshEntry::Loaded(idx)) => Some(*idx),
            Some(MeshEntry::Pending) | Some(MeshEntry::Failed) => None,
            None => {
                self.by_path.insert(path.to_owned(), MeshEntry::Pending);
                let tx = self.results_tx.clone();
                let owned = path.to_owned();
                std::thread::spawn(move || {
                    let result = super::gltf_import::load_gltf_data(&owned);
                    // App shutdown may have dropped the receiver; nothing to do.
                    let _ = tx.send((owned, result));
                });
                None
            }
        }
    }

    /// Drain up to `budget` completed background decodes: a successful one is
    /// uploaded and marked `Loaded`; a failed one is logged once and marked
    /// `Failed`. A result whose entry is no longer `Pending` (a `register`
    /// call on the same path raced it) is dropped. Returns the number of
    /// results drained, which may be less than `budget` if none more had
    /// arrived yet.
    pub(crate) fn integrate(
        &mut self,
        device: &Device,
        queue:  &Queue,
        layout: &BindGroupLayout,
        mipgen: &MipGenerator,
        budget: usize,
    ) -> usize {
        let mut drained = 0;
        for _ in 0..budget {
            let (path, result) = match self.results_rx.try_recv() {
                Ok(msg) => msg,
                Err(_) => break,
            };
            drained += 1;
            if self.by_path.get(&path) != Some(&MeshEntry::Pending) {
                continue;
            }
            match result {
                Ok(data) => {
                    let idx = self.meshes.len();
                    self.meshes.push(upload_mesh(device, queue, layout, mipgen, data));
                    self.by_path.insert(path, MeshEntry::Loaded(idx));
                }
                Err(e) => {
                    log::error!("mesh load failed: {e}");
                    self.by_path.insert(path, MeshEntry::Failed);
                }
            }
        }
        drained
    }

    /// Count of paths mid-decode — the map holds tens of entries, so a linear
    /// scan per frame (dev overlay) is cheap.
    pub(crate) fn pending_count(&self) -> usize {
        self.by_path.values().filter(|e| matches!(e, MeshEntry::Pending)).count()
    }

    /// Sum of every loaded mesh's material textures' resident GPU bytes —
    /// feeds the dev overlay's "tex mem (assets)" line (VQ-C5's budget).
    pub(crate) fn texture_memory_bytes(&self) -> u64 {
        self.meshes
            .iter()
            .flat_map(|m| &m.primitives)
            .flat_map(|p| &p._textures)
            .map(|t| t.bytes)
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn entry_state(&self, path: &str) -> Option<&MeshEntry> {
        self.by_path.get(path)
    }
}

pub(crate) const MESH_UPLOADS_PER_FRAME: usize = 1;

#[cfg(all(test, feature = "offscreen"))]
mod tests {
    use super::*;
    use crate::mesh::gltf_import::{AlphaMode, ImageData, MaterialData, PrimitiveData};
    use crate::mesh_pipeline::{self, MeshVertex};
    use crate::offscreen::HeadlessGpu;

    fn triangle_mesh_data() -> MeshData {
        let vertex = |x: f32, y: f32| MeshVertex {
            position: [x, y, 0.0],
            normal:   [0.0, 0.0, 1.0],
            uv:       [x, y],
            tangent:  [1.0, 0.0, 0.0, 1.0],
        };
        MeshData {
            primitives: vec![PrimitiveData {
                vertices: vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(0.0, 1.0)],
                indices:  vec![0, 1, 2],
                material: Default::default(),
                skin:     None,
            }],
            skeleton: None,
            clips:    vec![],
        }
    }

    /// Re-registering the same key must replace the GpuMesh in place — not
    /// grow `meshes` — so indices stay stable and the replaced buffers/
    /// textures drop instead of leaking (finding: re-registration leak).
    #[test]
    fn register_same_key_replaces_in_place() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — MeshStore::register test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        let idx1 = store.register(
            &gpu.device, &gpu.queue, &layout, &mipgen, "zone-ground:start", triangle_mesh_data(),
        );
        let idx2 = store.register(
            &gpu.device, &gpu.queue, &layout, &mipgen, "zone-ground:start", triangle_mesh_data(),
        );

        assert_eq!(idx1, idx2, "re-registering the same key must return the same index");
        assert_eq!(store.meshes.len(), 1, "re-registering must not grow the mesh vec");
    }

    /// First sight of a path must not block: `get_or_request` returns `None`
    /// immediately while the decode runs on a detached thread, and a later
    /// `integrate` call uploads the result once it arrives.
    #[test]
    fn first_sight_streams_in_background() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — MeshStore streaming test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        let path = std::env::temp_dir().join("vordar_store_test_first_sight.glb");
        crate::mesh::test_glb::write_test_glb(&path);
        let path = path.to_str().unwrap();

        assert_eq!(store.get_or_request(path), None, "first sight must not block on decode");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let idx = loop {
            store.integrate(&gpu.device, &gpu.queue, &layout, &mipgen, 1);
            if let Some(idx) = store.get_or_request(path) {
                break idx;
            }
            assert!(std::time::Instant::now() < deadline, "streamed load did not complete within 5s");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        assert_eq!(store.meshes.len(), 1);
        assert_eq!(store.get_or_request(path), Some(idx), "a loaded path resolves immediately");
    }

    /// A failed decode must be cached `Failed`, not retried on every
    /// subsequent `get_or_request` call.
    #[test]
    fn failed_load_is_cached_not_retried() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — MeshStore streaming test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        assert_eq!(store.get_or_request("does/not/exist.glb"), None);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let drained = store.integrate(&gpu.device, &gpu.queue, &layout, &mipgen, 1);
            if drained > 0 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "failed load did not resolve within 5s");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(store.get_or_request("does/not/exist.glb"), None);
        assert_eq!(store.entry_state("does/not/exist.glb"), Some(&MeshEntry::Failed));
    }

    /// `integrate`'s budget bounds how many completed decodes it uploads per
    /// call — two distinct pending assets never both land in one call.
    #[test]
    fn budget_bounds_integrations_per_call() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — MeshStore streaming test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        let path_a = std::env::temp_dir().join("vordar_store_test_budget_a.glb");
        let path_b = std::env::temp_dir().join("vordar_store_test_budget_b.glb");
        crate::mesh::test_glb::write_test_glb(&path_a);
        crate::mesh::test_glb::write_skinned_glb(&path_b);
        let path_a = path_a.to_str().unwrap();
        let path_b = path_b.to_str().unwrap();

        store.get_or_request(path_a);
        store.get_or_request(path_b);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while store.meshes.len() < 2 {
            let drained = store.integrate(&gpu.device, &gpu.queue, &layout, &mipgen, 1);
            assert!(drained <= 1, "budget of 1 must bound integrations per call");
            assert!(std::time::Instant::now() < deadline, "both loads did not complete within 5s");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(store.meshes.len(), 2);
    }

    /// A background job's key is deterministic per zone, so a second
    /// `request_job` call on the same key (zone re-entry) must not re-run
    /// the job — only the first call's closure ever executes.
    #[test]
    fn request_job_runs_once_per_key() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — MeshStore::request_job test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let key = "zone-ground:test";

        for _ in 0..2 {
            let calls = calls.clone();
            store.request_job(key, move || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(triangle_mesh_data())
            });
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let idx = loop {
            store.integrate(&gpu.device, &gpu.queue, &layout, &mipgen, 1);
            if let Some(idx) = store.get_or_request(key) {
                break idx;
            }
            assert!(std::time::Instant::now() < deadline, "job did not complete within 5s");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst), 1,
            "second request_job call on the same key must be a no-op"
        );
        assert_eq!(store.meshes.len(), 1);
        assert_eq!(store.get_or_request(key), Some(idx));
    }

    #[test]
    fn upload_records_blend_flag_and_centroid() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — GpuPrimitive metadata test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);

        let vertex = |x: f32, y: f32| MeshVertex {
            position: [x, y, 0.0],
            normal:   [0.0, 0.0, 1.0],
            uv:       [x, y],
            tangent:  [1.0, 0.0, 0.0, 1.0],
        };

        let data = MeshData {
            primitives: vec![
                PrimitiveData {
                    vertices: vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(0.0, 1.0)],
                    indices:  vec![0, 1, 2],
                    material: Default::default(),
                    skin:     None,
                },
                PrimitiveData {
                    vertices: vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(0.0, 1.0)],
                    indices:  vec![0, 1, 2],
                    material: MaterialData {
                        alpha_mode: AlphaMode::Blend,
                        ..Default::default()
                    },
                    skin:     None,
                },
            ],
            skeleton: None,
            clips:    vec![],
        };

        let gpu_mesh = upload_mesh(&gpu.device, &gpu.queue, &layout, &mipgen, data);

        assert_eq!(gpu_mesh.primitives.len(), 2);
        assert!(!gpu_mesh.primitives[0].blend, "default material should not be blend");
        assert!(gpu_mesh.primitives[1].blend, "alpha_mode::Blend should set blend flag");

        let centroid = glam::Vec3::new(0.5, 0.5, 0.0);
        assert!(gpu_mesh.primitives[0].centroid.abs_diff_eq(centroid, 1e-5), "prim0 centroid");
        assert!(gpu_mesh.primitives[1].centroid.abs_diff_eq(centroid, 1e-5), "prim1 centroid");
    }

    /// `texture_memory_bytes` sums every registered primitive's live material
    /// textures: an 8×8 mipped base-color image (4 mips: 64+16+4+1 texels ×
    /// 4 B/texel RGBA8 = 340 B) plus the four maps this fixture leaves unset
    /// (normal/metallic-roughness/emissive/occlusion), each a 1×1 neutral
    /// default at 4 B. Total: 340 + 4×4 = 356.
    #[test]
    fn texture_memory_bytes_sums_live_material_textures() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — MeshStore::texture_memory_bytes test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        let mut data = triangle_mesh_data();
        data.primitives[0].material.base_color_image = Some(ImageData {
            width:  8,
            height: 8,
            pixels: vec![255u8; 8 * 8 * 4],
        });

        store.register(&gpu.device, &gpu.queue, &layout, &mipgen, "tex-mem-test", data);

        assert_eq!(store.texture_memory_bytes(), 356);
    }

    /// Content-gated: streams the real statue asset (11 MB, embedded
    /// textures) and times the single `integrate` call that performs its
    /// upload — the residual main-thread cost once decode has moved
    /// off-frame, recorded in BASELINE.md.
    #[test]
    fn statue_streams_and_uploads_within_budget() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/statue_vroid.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — MeshStore streaming test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        assert_eq!(store.get_or_request(path), None);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let upload_time = loop {
            let start = std::time::Instant::now();
            let drained = store.integrate(&gpu.device, &gpu.queue, &layout, &mipgen, 1);
            let elapsed = start.elapsed();
            if drained > 0 {
                break elapsed;
            }
            assert!(std::time::Instant::now() < deadline, "statue decode did not complete within 60s");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        assert_eq!(store.get_or_request(path), Some(0));
        println!("statue_vroid.glb single-integrate upload cost: {upload_time:?}");
    }

    /// Content-gated: streams the statue + human assets (the heaviest
    /// shipped pair) through the real `get_or_request`/`integrate` path and
    /// prints resident texture memory, recorded in BASELINE.md. Uses
    /// `MeshStore`'s `pub(crate)` streaming API, so this must live inside
    /// the engine crate rather than an integration test.
    #[test]
    fn statue_and_human_texture_memory_measurement() {
        let statue_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/statue_vroid.glb");
        let human_path  = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(statue_path).exists() || !std::path::Path::new(human_path).exists() {
            return;
        }
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — MeshStore streaming test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        store.get_or_request(statue_path);
        store.get_or_request(human_path);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while store.meshes.len() < 2 {
            store.integrate(&gpu.device, &gpu.queue, &layout, &mipgen, 1);
            assert!(std::time::Instant::now() < deadline, "statue+human decode did not complete within 60s");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let bytes = store.texture_memory_bytes();
        println!(
            "statue_vroid.glb + human.glb resident texture memory: {} MB ({bytes} B)",
            bytes / (1024 * 1024)
        );
    }
}
