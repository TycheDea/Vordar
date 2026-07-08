// Billboard particle pass (VQ-E3) — textured, atlas-driven, soft, in two
// blend variants:
//   additive (One+One)           — energy: glows, sparks, trails
//   premultiplied alpha           — media: smoke, dust
// Particles draw in their own pass after the opaque scene so the scene depth
// can be sampled for depth-fade ("soft particles"); the quad is expanded from
// the camera basis, or velocity-aligned when the instance carries stretch.

use std::mem::size_of;
use wgpu::VertexFormat::{Float32x4, Uint32x4};
use wgpu::{
    BindGroupLayout, Device, Queue, RenderPipeline, TextureFormat, VertexAttribute,
    VertexBufferLayout, VertexStepMode,
};

/// One particle on the GPU. `color` rgb is already faded (premultiplied) by
/// the CPU sim; `a` feeds the alpha-blend variant. `stretch` xyz is the world
/// velocity, w the stretch factor (0 = round billboard).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleInstance {
    pub position: [f32; 3], // offset  0 — world-space center
    pub size:     f32,      // offset 12 — half-extent of the quad
    pub color:    [f32; 4], // offset 16
    pub stretch:  [f32; 4], // offset 32 — velocity xyz, stretch factor w
    pub cell:     u32,      // offset 48 — atlas cell (4×4 grid)
    pub _pad:     [u32; 3], // offset 52
}                           // total: 64 bytes

pub const MAX_PARTICLES: usize = 4096;
pub(crate) const PARTICLE_INSTANCE_SIZE: usize = size_of::<ParticleInstance>(); // 64

/// Atlas grid dimension (cells per side).
pub const ATLAS_GRID: u32 = 4;

/// Per-pass resources beyond the camera: atlas, scene depth (for the soft
/// fade), and the fade params.
pub(crate) fn create_particle_fx_bind_group_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label:   Some("Particle FX BGL"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled:   false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            }, // atlas
            wgpu::BindGroupLayoutEntry {
                binding:    1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count:      None,
            },
            wgpu::BindGroupLayoutEntry {
                binding:    2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled:   true,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type:    wgpu::TextureSampleType::Depth,
                },
                count: None,
            }, // scene depth (MSAA)
            wgpu::BindGroupLayoutEntry {
                binding:    3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            }, // params: viewport size + fade range
        ],
    })
}

/// Both blend variants share the shader and layouts.
pub(crate) fn create_particle_pipelines(
    device:         &Device,
    surface_format: TextureFormat,
    camera_bgl:     &BindGroupLayout,
    fx_bgl:         &BindGroupLayout,
) -> (RenderPipeline, RenderPipeline) {
    let shader = device.create_shader_module(wgpu::include_wgsl!("particle_shader.wgsl"));

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:              Some("Particle Pipeline Layout"),
        bind_group_layouts: &[Some(camera_bgl), Some(fx_bgl)],
        immediate_size:     0,
    });

    let instance_attributes = [
        VertexAttribute { offset: 0,  shader_location: 0, format: Float32x4 }, // position + size
        VertexAttribute { offset: 16, shader_location: 1, format: Float32x4 }, // color
        VertexAttribute { offset: 32, shader_location: 2, format: Float32x4 }, // stretch
        VertexAttribute { offset: 48, shader_location: 3, format: Uint32x4 },  // cell + pad
    ];
    let instance_buffer_layout = VertexBufferLayout {
        array_stride: PARTICLE_INSTANCE_SIZE as u64,
        step_mode:    VertexStepMode::Instance,
        attributes:   &instance_attributes,
    };

    let make = |label: &str, blend: wgpu::BlendState| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:      &shader,
                entry_point: Some("vtx_main"),
                buffers:     std::slice::from_ref(&instance_buffer_layout),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format:              TextureFormat::Depth32Float,
                depth_write_enabled: Some(false), // read-only pass
                depth_compare:       Some(wgpu::CompareFunction::Less),
                stencil:             Default::default(),
                bias:                Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: crate::post::SCENE_SAMPLES, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: Some("frag_main"),
                targets:     &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend:  Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache:          None,
        })
    };

    let additive = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation:  wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation:  wgpu::BlendOperation::Add,
        },
    };
    // Shader outputs premultiplied rgb.
    let premultiplied = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation:  wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation:  wgpu::BlendOperation::Add,
        },
    };
    (
        make("Particle Pipeline (additive)", additive),
        make("Particle Pipeline (alpha)", premultiplied),
    )
}

// ── Procedural atlas ─────────────────────────────────────────────────────────

/// Grayscale particle atlas pixels (VQ-E3), `size`×`size` RGBA8, ATLAS_GRID²
/// cells: 0 = soft glow, 1 = hard-core glow, 2 = horizontal streak,
/// 3 = smoke puff; remaining cells repeat soft glow variants. Pure CPU —
/// unit-tested; runtime-tinted so it is style-agnostic.
pub(crate) fn atlas_pixels(size: u32) -> Vec<u8> {
    let cell_px = size / ATLAS_GRID;
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    // Tiny deterministic noise for the smoke cell.
    let noise = |x: f32, y: f32| -> f32 {
        let h = (x * 127.1 + y * 311.7).sin() * 43758.5453;
        h.fract().abs()
    };
    for py in 0..size {
        for px in 0..size {
            let cell = (py / cell_px) * ATLAS_GRID + (px / cell_px);
            // Cell-local coords in [-1, 1].
            let cx = ((px % cell_px) as f32 + 0.5) / cell_px as f32 * 2.0 - 1.0;
            let cy = ((py % cell_px) as f32 + 0.5) / cell_px as f32 * 2.0 - 1.0;
            let r = (cx * cx + cy * cy).sqrt();
            let v: f32 = match cell {
                // 0: soft gaussian glow.
                0 => (-r * r * 4.0).exp(),
                // 1: hot core + wide halo.
                1 => (-r * r * 18.0).exp() + 0.35 * (-r * r * 3.0).exp(),
                // 2: streak — tight in y, long in x.
                2 => (-cy * cy * 24.0).exp() * (-cx * cx * 2.2).exp(),
                // 3: smoke puff — radial falloff broken by noise.
                3 => {
                    let n = 0.6 + 0.4 * noise((cx * 3.0).floor(), (cy * 3.0).floor());
                    ((1.0 - r).max(0.0) * n).powf(1.4)
                }
                // Variants: reuse the soft glow at falling intensities.
                c => (-r * r * 4.0).exp() * (1.0 - (c % 4) as f32 * 0.18),
            };
            let b = (v.clamp(0.0, 1.0) * 255.0) as u8;
            let i = ((py * size + px) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&[b, b, b, b]);
        }
    }
    pixels
}

/// Upload the atlas (linear RGBA8, no mips — cells are sampled near native size).
pub(crate) fn create_particle_atlas(device: &Device, queue: &Queue) -> crate::texture::ColorTexture {
    const SIZE: u32 = 512;
    let pixels = atlas_pixels(SIZE);
    crate::texture::create_rgba_texture(device, queue, SIZE, SIZE, &pixels, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_instance_is_tightly_packed() {
        assert_eq!(PARTICLE_INSTANCE_SIZE, 64);
        assert_eq!(std::mem::offset_of!(ParticleInstance, color), 16);
        assert_eq!(std::mem::offset_of!(ParticleInstance, stretch), 32);
        assert_eq!(std::mem::offset_of!(ParticleInstance, cell), 48);
    }

    #[test]
    fn atlas_cells_are_bright_centered_and_dark_cornered() {
        const S: u32 = 128; // 32px cells
        let px = atlas_pixels(S);
        let cell_px = S / ATLAS_GRID;
        let value = |cell_x: u32, cell_y: u32, lx: u32, ly: u32| {
            let x = cell_x * cell_px + lx;
            let y = cell_y * cell_px + ly;
            px[((y * S + x) * 4) as usize]
        };
        for cell in 0..4u32 {
            let (cx, cy) = (cell % ATLAS_GRID, cell / ATLAS_GRID);
            let center = value(cx, cy, cell_px / 2, cell_px / 2);
            let corner = value(cx, cy, 1, 1);
            assert!(center > 100, "cell {cell} center bright, got {center}");
            assert!(corner < center / 2, "cell {cell} corner dark: {corner} vs {center}");
        }
    }

    #[test]
    fn streak_cell_is_elongated_along_x() {
        const S: u32 = 128;
        let px = atlas_pixels(S);
        let cell_px = S / ATLAS_GRID;
        // Cell 2 sits at grid (2, 0).
        let value = |lx: u32, ly: u32| px[(((0 * cell_px + ly) * S) + 2 * cell_px + lx) as usize * 4];
        let mid = cell_px / 2;
        let along_x = value(mid + cell_px / 4, mid);
        let along_y = value(mid, mid + cell_px / 4);
        assert!(
            along_x > along_y * 3,
            "streak spreads along x: x={along_x} y={along_y}"
        );
    }
}
