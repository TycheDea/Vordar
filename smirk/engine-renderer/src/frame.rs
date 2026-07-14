//! RenderSystem — the per-frame graph: shadow → main (SDF/mesh/skinned/
//! sky) → particles → bloom/tonemap → egui → present.

use crate::dev_overlay;
use crate::instance::{InstancePool, SdfInstance, INSTANCE_SIZE};
use crate::menu::{draw_menu, MenuAction, MenuState};
use crate::mesh::{MeshDrawList, MeshStore, SkinnedDrawList};
use crate::particle_pipeline;
use crate::pipeline::INDICES;
use crate::shadow;
use crate::state::{RendererState, MAX_MESH_INSTANCES};
use crate::ui_layers::UiLayers;
use crate::ParticleDrawList;
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;
use std::sync::Arc;
use winit::window::Window;

pub(crate) struct RenderSystem {
    gpu_buf:      Vec<SdfInstance>,
    dirty_ranges: Vec<(u64, usize)>,
    /// Deferred actions collected from egui during the last frame.
    pending_menu: Vec<MenuAction>,
    /// Frames spent above 80% of the particle cap (throttles the warning).
    particle_warn: u32,
    /// GPU frame timing (dev overlay): sampled sparsely, last value cached.
    frame_index: u64,
    last_gpu_ms: Option<f32>,
}

/// Sample the GPU frame time once every N frames while the overlay is open
/// (each sample costs a blocking map — dev-only).
const GPU_TIMING_INTERVAL: u64 = 30;

impl RenderSystem {
    pub fn new() -> Self {
        Self {
            gpu_buf: Vec::new(),
            dirty_ranges: Vec::new(),
            pending_menu: Vec::new(),
            particle_warn: 0,
            frame_index: 0,
            last_gpu_ms: None,
        }
    }
}

impl System for RenderSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        // ── Apply deferred menu actions from the last frame ───────────────────
        crate::menu_actions::apply_menu_actions(std::mem::take(&mut self.pending_menu), resources);

        // ── Collect dirty ranges ──────────────────────────────────────────────
        self.gpu_buf.clear();
        self.dirty_ranges.clear();
        let slot_count = {
            let pool = resources.get::<InstancePool>().expect("InstancePool not in resources");
            let mut i = 0;
            while i < pool.slots.len() {
                if pool.dirty[i] {
                    let start = i;
                    while i < pool.slots.len() && pool.dirty[i] { i += 1; }
                    self.dirty_ranges.push((start as u64 * INSTANCE_SIZE as u64, i - start));
                    self.gpu_buf.extend_from_slice(&pool.slots[start..i]);
                } else {
                    i += 1;
                }
            }
            pool.slots.len()
        };
        {
            let pool = resources.get_mut::<InstancePool>().expect("InstancePool not in resources");
            pool.dirty.iter_mut().for_each(|d| *d = false);
        }

        // ── Snapshot lightweight state for egui draw (before mut borrow) ─────
        self.frame_index += 1;
        let overlay_open = resources
            .get::<engine_app::dev_stats::DevStats>()
            .map(|s| s.open)
            .unwrap_or(false);
        // Publish last frame's GPU time before the lines snapshot below.
        if overlay_open {
            if let (Some(ms), Some(stats)) =
                (self.last_gpu_ms, resources.get_mut::<engine_app::dev_stats::DevStats>())
            {
                stats.set("gpu", format!("{ms:.2} ms"));
            }
        }
        let sample_gpu = overlay_open && self.frame_index % GPU_TIMING_INTERVAL == 0;

        let window       = resources.get::<Arc<Window>>().cloned();
        let menu_snap    = resources.get::<MenuState>().cloned();
        let dev_lines    = resources.get::<engine_app::dev_stats::DevStats>()
            .filter(|s| s.open)
            .map(|s| s.display_lines());
        let monitor_fps  = window.as_ref()
            .and_then(|w| w.current_monitor())
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| (mhz / 1000).max(30));

        // ── Egui frame: engine UI + game-registered UiLayers ─────────────────
        // Runs against Arc-clone handles BEFORE the RendererState mut borrow,
        // so game layers get read access to Resources.
        let egui_frame = if let Some(ref w) = window {
            let (egui_ctx, egui_winit) = {
                let s = resources.get::<RendererState>()
                    .expect("RendererState not in resources");
                (s.egui_ctx.clone(), s.egui_winit.clone())
            };
            let raw_input = egui_winit.lock().unwrap().take_egui_input(w);
            let mut menu_actions: Vec<MenuAction> = Vec::new();

            // begin_pass/end_pass, NOT run_ui: run_ui wraps the frame in a
            // full-screen background Ui and allocates it as a central panel,
            // which makes egui claim the whole viewport — egui-winit then
            // consumes every unrelated click and wheel event (game input
            // died unless another button was already held).
            egui_ctx.begin_pass(raw_input);
            if let Some(ref lines) = dev_lines {
                dev_overlay::draw_dev_overlay(&egui_ctx, lines);
            }
            if let Some(m) = menu_snap.as_ref() {
                if m.open {
                    draw_menu(&egui_ctx, m, monitor_fps, &mut menu_actions);
                }
            }
            // Game UI layers (minimap, action bar, ...). Taken out so the
            // callbacks can read Resources while the registry is borrowed.
            let mut layers = resources.get_mut::<UiLayers>()
                .map(std::mem::take)
                .unwrap_or_default();
            for layer in layers.layers.iter_mut() {
                layer(&egui_ctx, resources);
            }
            if let Some(slot) = resources.get_mut::<UiLayers>() {
                *slot = layers;
            }
            let full_output = egui_ctx.end_pass();

            // Handle platform output (clipboard, cursor, etc.)
            egui_winit.lock().unwrap()
                .handle_platform_output(w, full_output.platform_output.clone());

            let ppp = full_output.pixels_per_point;
            let prims = egui_ctx.tessellate(full_output.shapes.clone(), ppp);
            self.pending_menu = menu_actions;
            Some((full_output, prims, ppp))
        } else {
            None
        };

        // Mesh draw lists + store, taken out so they outlive the RendererState
        // borrow below (returned at the end of the frame).
        let mesh_list     = resources.get_mut::<MeshDrawList>().map(std::mem::take);
        let skinned_list  = resources.get_mut::<SkinnedDrawList>().map(std::mem::take);
        let mesh_store    = resources.get_mut::<MeshStore>().map(std::mem::take);
        let particle_list = resources.get_mut::<ParticleDrawList>().map(std::mem::take);

        // Cap guardrail (VQ-F2): meter + throttled warning past 80%.
        {
            let count = particle_list.as_ref().map(|l| l.instances.len()).unwrap_or(0);
            if let Some(stats) = resources.get_mut::<engine_app::dev_stats::DevStats>() {
                stats.set("particles", format!("{count}/{}", particle_pipeline::MAX_PARTICLES));
            }
            if count * 10 > particle_pipeline::MAX_PARTICLES * 8 {
                self.particle_warn += 1;
                if self.particle_warn % 300 == 1 {
                    log::warn!(
                        "live particles at {count}/{} (>80% of the engine cap)",
                        particle_pipeline::MAX_PARTICLES
                    );
                }
            } else {
                self.particle_warn = 0;
            }
        }

        // ── All GPU work inside one mutable borrow of RendererState ───────────
        let state = resources.get_mut::<RendererState>()
            .expect("RendererState not in resources");

        let mut buf_pos = 0usize;
        for &(offset, count) in &self.dirty_ranges {
            let data = bytemuck::cast_slice(&self.gpu_buf[buf_pos..buf_pos + count]);
            state.queue.write_buffer(&state.instance_buffer, offset, data);
            buf_pos += count;
        }

        if let Some(list) = mesh_list.as_ref().filter(|l| !l.instances.is_empty()) {
            let n = list.instances.len().min(MAX_MESH_INSTANCES);
            state.queue.write_buffer(
                &state.mesh_instance_buffer, 0,
                bytemuck::cast_slice(&list.instances[..n]),
            );
        }

        // Skinned instances + joint palette.
        if let Some(list) = skinned_list.as_ref().filter(|l| !l.instances.is_empty()) {
            state.queue.write_buffer(
                &state.skinned_instance_buffer, 0,
                bytemuck::cast_slice(&list.instances),
            );
            if !list.joints.is_empty() {
                state.queue.write_buffer(
                    &state.joint_buffer, 0,
                    bytemuck::cast_slice(&list.joints),
                );
            }
        }

        // Particles.
        let particle_count = particle_list
            .as_ref()
            .map(|l| l.instances.len().min(particle_pipeline::MAX_PARTICLES))
            .unwrap_or(0);
        if particle_count > 0 {
            let list = particle_list.as_ref().expect("count > 0");
            state.queue.write_buffer(
                &state.particle_instance_buffer, 0,
                bytemuck::cast_slice(&list.instances[..particle_count]),
            );
        }

        // wgpu 29: get_current_texture() returns CurrentSurfaceTexture enum
        let surface_texture = match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                state.resize(state.config.width, state.config.height);
                restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
                return;
            }
        };

        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // ── Egui GPU upload ───────────────────────────────────────────────────
        let (egui_output, egui_primitives, egui_screen) = if let Some((full_output, prims, ppp)) = egui_frame {
            let sd = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [state.config.width, state.config.height],
                pixels_per_point: ppp,
            };
            // Upload new egui textures
            for (id, delta) in &full_output.textures_delta.set {
                state.egui_renderer.update_texture(&state.device, &state.queue, *id, delta);
            }
            (Some(full_output), Some(prims), Some(sd))
        } else {
            (None, None, None)
        };

        // ── Build command encoder ─────────────────────────────────────────────
        let mut encoder = state.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("Render Encoder") }
        );

        // Shadow pre-pass (VQ-D3): fit the sun's ortho volume around the
        // camera target (texel-snapped) and render depth-only variants of
        // every opaque draw. Particles don't cast.
        {
            let light_vp = shadow::fit_light_vp(state.camera.target, state.light_dir);
            state.queue.write_buffer(
                &state.light_vp_buffer, 0,
                bytemuck::cast_slice(&light_vp.to_cols_array()),
            );

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shadow Pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &state.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: if sample_gpu {
                    state.gpu_timer.as_ref().map(|t| t.begin_writes())
                } else {
                    None
                },
                ..Default::default()
            });

            // SDF primitives.
            pass.set_pipeline(&state.shadow_pipelines.sdf);
            pass.set_bind_group(0, &state.shadow_bind_group, &[]);
            pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, state.instance_buffer.slice(..));
            pass.set_index_buffer(state.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..INDICES.len() as u32, 0, 0..slot_count as u32);

            // Static meshes.
            if let (Some(list), Some(store)) = (mesh_list.as_ref(), mesh_store.as_ref()) {
                if !list.instances.is_empty() {
                    pass.set_pipeline(&state.shadow_pipelines.mesh);
                    pass.set_vertex_buffer(1, state.mesh_instance_buffer.slice(..));
                    for &(mesh_idx, first, count) in &list.ranges {
                        if first as usize >= MAX_MESH_INSTANCES { break; }
                        let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                        let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                        for prim in &gpu_mesh.primitives {
                            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                        }
                    }
                }
            }

            // Skinned meshes (re-binds the shared joint palette).
            if let (Some(list), Some(store)) = (skinned_list.as_ref(), mesh_store.as_ref()) {
                if !list.instances.is_empty() {
                    pass.set_pipeline(&state.shadow_pipelines.skinned);
                    pass.set_bind_group(1, &state.joint_bind_group, &[]);
                    pass.set_vertex_buffer(1, state.skinned_instance_buffer.slice(..));
                    for &(mesh_idx, first, count) in &list.ranges {
                        let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                        for prim in &gpu_mesh.primitives {
                            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                        }
                    }
                }
            }
        }

        // Main 3D pass — MSAA HDR opaque + sky. Color/depth stay live for the
        // particle pass, which resolves at its end (VQ-D1/D4).
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &state.hdr.msaa_view,
                    resolve_target: None,
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &state.hdr.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let tex_bg = &state.texture_store[state.active_texture_idx].1;
            pass.set_pipeline(&state.pipeline);
            pass.set_bind_group(0, &state.camera_bind_group, &[]);
            pass.set_bind_group(1, tex_bg, &[]);
            pass.set_bind_group(2, &state.environment.bind_group, &[]);
            pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, state.instance_buffer.slice(..));
            pass.set_index_buffer(state.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..INDICES.len() as u32, 0, 0..slot_count as u32);

            // Mesh pass — same render pass and camera bind group, real geometry.
            // Ranges are sorted by first-instance, so overflow past the buffer
            // cap ends the loop rather than wrapping.
            if let (Some(list), Some(store)) = (mesh_list.as_ref(), mesh_store.as_ref()) {
                if !list.instances.is_empty() {
                    pass.set_pipeline(&state.mesh_pipeline);
                    pass.set_bind_group(2, &state.environment.bind_group, &[]);
                    pass.set_vertex_buffer(1, state.mesh_instance_buffer.slice(..));
                    for &(mesh_idx, first, count) in &list.ranges {
                        if first as usize >= MAX_MESH_INSTANCES { break; }
                        let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                        let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                        for prim in &gpu_mesh.primitives {
                            pass.set_bind_group(1, &prim.material_bind_group, &[]);
                            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                        }
                    }
                }
            }

            // Skinned mesh pass — same camera bind group, plus the joint
            // palette (group 2). Instances index their own joint block via the
            // joint_base instance attribute, so one draw per mesh still works.
            if let (Some(list), Some(store)) = (skinned_list.as_ref(), mesh_store.as_ref()) {
                if !list.instances.is_empty() {
                    pass.set_pipeline(&state.skinned_pipeline);
                    pass.set_bind_group(0, &state.camera_bind_group, &[]);
                    pass.set_bind_group(2, &state.joint_bind_group, &[]);
                    pass.set_bind_group(3, &state.environment.bind_group, &[]);
                    pass.set_vertex_buffer(1, state.skinned_instance_buffer.slice(..));
                    for &(mesh_idx, first, count) in &list.ranges {
                        let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                        for prim in &gpu_mesh.primitives {
                            pass.set_bind_group(1, &prim.material_bind_group, &[]);
                            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                        }
                    }
                }
            }

            // Sky pass — the IBL cubemap as background, pinned to the far
            // plane behind everything opaque (VQ-D2).
            pass.set_pipeline(&state.sky_pipeline);
            pass.set_bind_group(0, &state.camera_bind_group, &[]);
            pass.set_bind_group(1, &state.environment.sky_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Particle pass (VQ-E3): depth read-only so the shader can sample the
        // scene depth for the soft fade; additive first, then premultiplied
        // alpha; the MSAA resolve happens at the end of this pass.
        {
            let additive_count = particle_list
                .as_ref()
                .map(|l| l.additive_count.min(particle_count))
                .unwrap_or(0);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Particle Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &state.hdr.msaa_view,
                    resolve_target: Some(&state.hdr.resolve_view),
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard, // resolve keeps the frame
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view:        &state.hdr.depth_view,
                    depth_ops:   None, // read-only: tested by particles, sampled for softness
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if particle_count > 0 {
                pass.set_bind_group(0, &state.camera_bind_group, &[]);
                pass.set_bind_group(1, &state.particle_fx_bind_group, &[]);
                pass.set_vertex_buffer(0, state.particle_instance_buffer.slice(..));
                if additive_count > 0 {
                    pass.set_pipeline(&state.particle_additive);
                    pass.draw(0..4, 0..additive_count as u32);
                }
                if particle_count > additive_count {
                    pass.set_pipeline(&state.particle_alpha);
                    pass.draw(0..4, additive_count as u32..particle_count as u32);
                }
            }
        }

        // Bloom chain from the HDR resolve, then tonemap (ACES + exposure +
        // bloom composite) onto the swapchain.
        state.bloom.encode(&mut encoder);
        state.tonemap.encode(
            &mut encoder,
            &view,
            if sample_gpu {
                state.gpu_timer.as_ref().map(|t| t.end_writes())
            } else {
                None
            },
        );

        // Egui overlay pass (Load existing pixels — don't clear the 3D scene)
        if let (Some(prims), Some(sd)) = (egui_primitives.as_ref(), egui_screen.as_ref()) {
            state.egui_renderer.update_buffers(
                &state.device, &state.queue, &mut encoder, prims, sd,
            );
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &view,
                    resolve_target: None,
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Load, // preserve 3D scene below
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            let mut rpass_static = rpass.forget_lifetime();
            state.egui_renderer.render(&mut rpass_static, prims, sd);
        }

        // Free textures, submit, present
        if let Some(ref fo) = egui_output {
            for id in &fo.textures_delta.free {
                state.egui_renderer.free_texture(id);
            }
        }
        if sample_gpu {
            if let Some(timer) = state.gpu_timer.as_ref() {
                timer.resolve(&mut encoder);
            }
        }
        state.queue.submit(std::iter::once(encoder.finish()));
        if sample_gpu {
            if let Some(timer) = state.gpu_timer.as_ref() {
                self.last_gpu_ms = timer.read_blocking(&state.device).or(self.last_gpu_ms);
            }
        }
        surface_texture.present();

        restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
    }
}

/// Return the taken mesh draw lists/store to Resources — called on every exit
/// path of RenderSystem::run so loaded meshes survive skipped frames.
fn restore_mesh_resources(
    resources: &mut Resources,
    list:      Option<MeshDrawList>,
    skinned:   Option<SkinnedDrawList>,
    store:     Option<MeshStore>,
    particles: Option<ParticleDrawList>,
) {
    if let Some(l) = list      { resources.insert(l); }
    if let Some(s) = skinned   { resources.insert(s); }
    if let Some(s) = store     { resources.insert(s); }
    if let Some(p) = particles { resources.insert(p); }
}
