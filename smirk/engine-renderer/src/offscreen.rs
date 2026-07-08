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

/// Read a 4-bytes-per-pixel `SceneTarget` back to CPU memory, rows unpadded
/// (`width * height * 4` bytes, row-major). Blocks until the copy completes.
pub fn read_rgba8(gpu: &HeadlessGpu, target: &SceneTarget) -> Vec<u8> {
    const ROW_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT; // 256
    let unpadded = target.width * 4;
    let padded   = unpadded.div_ceil(ROW_ALIGN) * ROW_ALIGN;

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Readback Buffer"),
        size:               (padded * target.height) as u64,
        usage:              wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Readback Encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture:   &target.color,
            mip_level: 0,
            origin:    wgpu::Origin3d::ZERO,
            aspect:    wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(padded),
                rows_per_image: Some(target.height),
            },
        },
        wgpu::Extent3d { width: target.width, height: target.height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(std::iter::once(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("readback map failed"));
    gpu.device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
        .expect("device poll failed");

    let mapped = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((unpadded * target.height) as usize);
    for row in 0..target.height {
        let start = (row * padded) as usize;
        pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}
