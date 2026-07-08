// Headless offscreen render harness — VQ-G1.
//
// Lets integration tests exercise the real scene pipelines (same WGSL, same
// pipeline factories as RendererState) without a window or swapchain: render
// into an owned color target, read the pixels back, assert analytically
// (coverage %, darker-than, monotonic — never exact pixel values).
//
// Pre-stages the Phase-2 HDR retarget: `SceneTarget` is the "Main Pass renders
// into a texture I own" abstraction; Phase 2 points RendererState's main pass
// at one of these (Rgba16Float) instead of the swapchain view.
//
// Device requirements are deliberately minimal (no TEXTURE_COMPRESSION_BC —
// fallback adapters lack it), so harness assets must be RGBA8/procedural.

use crate::camera::{self, Camera};
use crate::instance::SdfInstance;
use crate::mesh::{self, MeshData};
use crate::mesh_pipeline::{self, MeshInstance};
use crate::mipgen::MipGenerator;
use crate::pipeline::{self, INDICES};
use crate::texture;
use wgpu::util::DeviceExt;

/// A GPU device with no surface attached. `None` when the machine has no
/// usable adapter (headless CI) — callers skip their test cleanly.
pub struct HeadlessGpu {
    pub device: wgpu::Device,
    pub queue:  wgpu::Queue,
}

impl HeadlessGpu {
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     None,
                force_fallback_adapter: false,
            },
        )).ok()?;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default())
        ).ok()?;
        Some(Self { device, queue })
    }
}

/// An offscreen render target: color (readback-capable) + depth. The size and
/// format the Main Pass renders into, decoupled from any swapchain.
pub struct SceneTarget {
    pub color:      wgpu::Texture,
    pub color_view: wgpu::TextureView,
    pub depth_view: wgpu::TextureView,
    pub width:      u32,
    pub height:     u32,
    pub format:     wgpu::TextureFormat,
}

impl SceneTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("Offscreen Scene Target"),
            size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format,
            usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats:    &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let (_, depth_view) = texture::create_depth_texture(device, width, height);
        Self { color, color_view, depth_view, width, height, format }
    }
}

/// Render SDF-instance geometry through the real scene pipeline (shader.wgsl)
/// into `target`, with the default orbit camera looking at the origin and the
/// default sun. This is the same draw the Main Pass makes for primitives.
pub fn render_sdf_scene(gpu: &HeadlessGpu, target: &SceneTarget, instances: &[SdfInstance], clear: wgpu::Color) {
    let device = &gpu.device;

    let camera = Camera::new(target.width as f32 / target.height as f32);
    let (_cam_buf, _light_buf, camera_bgl, camera_bind_group) =
        camera::create_gpu_resources(device, &camera);

    let texture_bgl = pipeline::create_texture_bind_group_layout(device);
    let white       = texture::create_default_white(device, &gpu.queue);
    let white_bg    = texture::create_bind_group(device, &texture_bgl, &white);

    let render_pipeline = pipeline::create_pipeline(device, target.format, &camera_bgl, &texture_bgl);
    let vertex_buffer   = pipeline::create_vertex_buffer(device);
    let index_buffer    = pipeline::create_index_buffer(device);
    let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:    Some("Offscreen Instance Buffer"),
        contents: bytemuck::cast_slice(instances),
        usage:    wgpu::BufferUsages::VERTEX,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Offscreen Encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Offscreen Main Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           &target.color_view,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if !instances.is_empty() {
            pass.set_pipeline(&render_pipeline);
            pass.set_bind_group(0, &camera_bind_group, &[]);
            pass.set_bind_group(1, &white_bg, &[]);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, instance_buffer.slice(..));
            pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..INDICES.len() as u32, 0, 0..instances.len() as u32);
        }
    }
    gpu.queue.submit(std::iter::once(encoder.finish()));
}

/// Render static glTF mesh data through the real mesh pipeline
/// (mesh_shader.wgsl, full PBR material bind groups, mipped textures) into
/// `target` — one instance at the origin, white tint, default camera + sun.
/// Skinned data is rejected (the harness drives the static path).
pub fn render_mesh_scene(gpu: &HeadlessGpu, target: &SceneTarget, data: MeshData, clear: wgpu::Color) {
    assert!(data.skeleton.is_none(), "render_mesh_scene drives the static mesh path only");
    let device = &gpu.device;

    let camera = Camera::new(target.width as f32 / target.height as f32);
    let (_cam_buf, _light_buf, camera_bgl, camera_bind_group) =
        camera::create_gpu_resources(device, &camera);

    let material_bgl = mesh_pipeline::create_material_bind_group_layout(device);
    let mipgen       = MipGenerator::new(device);
    let gpu_mesh     = mesh::upload_mesh(device, &gpu.queue, &material_bgl, &mipgen, data);
    let render_pipeline =
        mesh_pipeline::create_mesh_pipeline(device, target.format, &camera_bgl, &material_bgl);

    let instance = MeshInstance {
        model: glam::Mat4::IDENTITY.to_cols_array_2d(),
        tint:  [1.0; 4],
    };
    let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:    Some("Offscreen Mesh Instance Buffer"),
        contents: bytemuck::cast_slice(&[instance]),
        usage:    wgpu::BufferUsages::VERTEX,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Offscreen Mesh Encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Offscreen Mesh Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           &target.color_view,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&render_pipeline);
        pass.set_bind_group(0, &camera_bind_group, &[]);
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        for prim in &gpu_mesh.primitives {
            pass.set_bind_group(1, &prim.material_bind_group, &[]);
            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..prim.index_count, 0, 0..1);
        }
    }
    gpu.queue.submit(std::iter::once(encoder.finish()));
}

/// Upload RGBA8 pixels and build the full mip chain through the real blit
/// generator (VQ-C1). Exposed so tests can assert on downsampled levels.
pub fn create_mipped_rgba8(
    gpu:    &HeadlessGpu,
    width:  u32,
    height: u32,
    pixels: &[u8],
    srgb:   bool,
) -> wgpu::Texture {
    let mipgen = MipGenerator::new(&gpu.device);
    texture::create_rgba_texture_mipped(&gpu.device, &gpu.queue, &mipgen, width, height, pixels, srgb)
        .texture
}

/// Read one mip level of a 4-bytes-per-pixel texture back to CPU memory, rows
/// unpadded (row-major). Blocks until the copy completes.
pub fn read_texture_mip(gpu: &HeadlessGpu, texture: &wgpu::Texture, mip: u32) -> Vec<u8> {
    const ROW_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT; // 256
    let width  = (texture.width() >> mip).max(1);
    let height = (texture.height() >> mip).max(1);
    let unpadded = width * 4;
    let padded   = unpadded.div_ceil(ROW_ALIGN) * ROW_ALIGN;

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Readback Buffer"),
        size:               (padded * height) as u64,
        usage:              wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Readback Encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin:    wgpu::Origin3d::ZERO,
            aspect:    wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(std::iter::once(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("readback map failed"));
    gpu.device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
        .expect("device poll failed");

    let mapped = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((unpadded * height) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}

/// Read a 4-bytes-per-pixel `SceneTarget` back to CPU memory, rows unpadded
/// (`width * height * 4` bytes, row-major). Blocks until the copy completes.
pub fn read_rgba8(gpu: &HeadlessGpu, target: &SceneTarget) -> Vec<u8> {
    read_texture_mip(gpu, &target.color, 0)
}
