//! RenderSystem — the per-frame graph: shadow → main (SDF/mesh/skinned/
//! sky/transparent) → particles → bloom/tonemap → egui → present.

use crate::dev_overlay;
use crate::gpu_timer::{GpuPass, GpuPassTimings};
use crate::instance::{InstancePool, SdfInstance, INSTANCE_SIZE};
use crate::menu::{draw_menu, MenuAction, MenuState};
use crate::mesh::{MeshDrawList, MeshStore, SkinnedDrawList};
use crate::particle_pipeline;
use crate::sdf_pipeline::INDICES;
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
    last_gpu: Option<GpuPassTimings>,
    /// Sorted back-to-front blend draws for this frame's transparent pass,
    /// rebuilt each frame by `collect_transparent_draws`.
    transparent_draws: Vec<TransparentDraw>,
    /// Contiguous runs of in-use SDF slots for this frame, collected after dirty-range scan.
    sdf_runs: Vec<(u32, u32)>,
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
            last_gpu: None,
            transparent_draws: Vec::new(),
            sdf_runs: Vec::new(),
        }
    }

    /// Gather dirty instance-pool ranges into `gpu_buf`/`dirty_ranges` and
    /// clear the pool's dirty flags; returns the pool's total slot count.
    fn collect_dirty_ranges(&mut self, resources: &mut Resources) -> usize {
        self.gpu_buf.clear();
        self.dirty_ranges.clear();
        let slot_count = {
            let pool = resources.expect::<InstancePool>();
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
            let pool = resources.expect_mut::<InstancePool>();
            pool.dirty.iter_mut().for_each(|d| *d = false);
        }
        slot_count
    }

    /// Build this frame's egui output (engine UI + game `UiLayers`) against
    /// Arc-clone handles, before the `RendererState` mut borrow — so game
    /// layers get read access to `Resources`. `None` if there is no window.
    fn build_egui_frame(
        &mut self,
        resources: &mut Resources,
    ) -> Option<(egui::FullOutput, Vec<egui::ClippedPrimitive>, f32)> {
        let window = resources.get::<Arc<Window>>().cloned()?;
        let menu_snap = resources.get::<MenuState>().cloned();
        let dev_lines = resources.get::<engine_app::dev_stats::DevStats>()
            .filter(|s| s.open)
            .map(|s| s.display_lines());
        let monitor_fps = window.current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| (mhz / 1000).max(30));

        let (egui_ctx, egui_winit) = {
            let s = resources.expect::<RendererState>();
            (s.egui_ctx.clone(), s.egui_winit.clone())
        };
        let raw_input = egui_winit.lock().unwrap().take_egui_input(&window);
        let mut menu_actions: Vec<MenuAction> = Vec::new();
        if menu_snap.as_ref().is_some_and(|m| m.quit_requested) {
            menu_actions.push(MenuAction::Quit);
            if let Some(m) = resources.get_mut::<MenuState>() { m.quit_requested = false; }
        }

        // begin_pass/end_pass, NOT run_ui: run_ui wraps the frame in a
        // full-screen background Ui and allocates it as a central panel,
        // which makes egui claim the whole viewport — egui-winit then
        // consumes every unrelated click and wheel event (game input
        // died unless another button was already held).
        egui_ctx.begin_pass(raw_input);
        if let Some(ref lines) = dev_lines {
            dev_overlay::draw_dev_overlay(&egui_ctx, lines);
        }
        if let Some(m) = menu_snap.as_ref()
            && m.open {
                draw_menu(&egui_ctx, m, monitor_fps, &mut menu_actions);
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
            .handle_platform_output(&window, full_output.platform_output.clone());

        let ppp = full_output.pixels_per_point;
        let prims = egui_ctx.tessellate(full_output.shapes.clone(), ppp);
        self.pending_menu = menu_actions;
        Some((full_output, prims, ppp))
    }
}

impl System for RenderSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        // ── Bake + swap the environment if its background decode arrived ──────
        if let Some(state) = resources.get_mut::<RendererState>() {
            state.poll_pending_environment();
        }

        // ── Apply deferred menu actions from the last frame ───────────────────
        crate::menu_actions::apply_menu_actions(std::mem::take(&mut self.pending_menu), resources);

        // ── Collect dirty ranges ──────────────────────────────────────────────
        let _slot_count = self.collect_dirty_ranges(resources);

        // ── Collect SDF in-use runs ───────────────────────────────────────────
        {
            let pool = resources.expect::<InstancePool>();
            pool.used_runs(&mut self.sdf_runs);
        }

        // ── Snapshot lightweight state for egui draw (before mut borrow) ─────
        self.frame_index += 1;
        let overlay_open = resources
            .get::<engine_app::dev_stats::DevStats>()
            .map(|s| s.open)
            .unwrap_or(false);
        // Publish last frame's per-pass GPU times before the lines snapshot below.
        if overlay_open
            && let (Some(t), Some(stats)) =
                (self.last_gpu, resources.get_mut::<engine_app::dev_stats::DevStats>())
            {
                stats.set("gpu shadow",        format!("{:.2} ms", t.shadow));
                stats.set("gpu main",          format!("{:.2} ms", t.main));
                stats.set("gpu particles",     format!("{:.2} ms", t.particles));
                stats.set("gpu bloom+tonemap", format!("{:.2} ms", t.bloom_tonemap));
                stats.set("gpu egui",          format!("{:.2} ms", t.egui));
            }
        let sample_gpu = overlay_open && self.frame_index.is_multiple_of(GPU_TIMING_INTERVAL);

        // ── Egui frame: engine UI + game-registered UiLayers ─────────────────
        let egui_frame = self.build_egui_frame(resources);

        // Mesh draw lists + store, taken out so they outlive the RendererState
        // borrow below (returned at the end of the frame).
        let mesh_list     = resources.get_mut::<MeshDrawList>().map(std::mem::take);
        let skinned_list  = resources.get_mut::<SkinnedDrawList>().map(std::mem::take);
        let mesh_store    = resources.get_mut::<MeshStore>().map(std::mem::take);
        let particle_list = resources.get_mut::<ParticleDrawList>().map(std::mem::take);

        // Cap guardrail: meter + throttled warning past 80%.
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
        let state = resources.expect_mut::<RendererState>();

        let particle_count = upload_gpu_buffers(
            state, &self.dirty_ranges, &self.gpu_buf,
            mesh_list.as_ref(), skinned_list.as_ref(), particle_list.as_ref(),
        );

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

        record_shadow_pass(
            state, &mut encoder, &self.sdf_runs, sample_gpu,
            mesh_list.as_ref(), skinned_list.as_ref(), mesh_store.as_ref(),
        );

        if let Some(store) = mesh_store.as_ref() {
            collect_transparent_draws(
                store, mesh_list.as_ref(), skinned_list.as_ref(),
                state.camera.eye(), &mut self.transparent_draws,
            );
        } else {
            self.transparent_draws.clear();
        }

        if state.ssao_enabled {
            record_depth_prepass(
                state, &mut encoder, &self.sdf_runs,
                mesh_list.as_ref(), skinned_list.as_ref(), mesh_store.as_ref(),
            );
            record_ssao(state, &mut encoder);
        }

        record_main_pass(
            state, &mut encoder, &self.sdf_runs, sample_gpu,
            mesh_list.as_ref(), skinned_list.as_ref(), mesh_store.as_ref(),
            &self.transparent_draws,
        );

        record_particle_pass(state, &mut encoder, sample_gpu, particle_list.as_ref(), particle_count);

        // Bloom chain from the HDR resolve, then tonemap (ACES + exposure +
        // bloom composite) onto the swapchain.
        state.bloom.encode(&mut encoder, if sample_gpu { state.gpu_timer.as_ref() } else { None });
        state.tonemap.encode(
            &mut encoder,
            &view,
            if sample_gpu {
                state.gpu_timer.as_ref().map(|t| t.pass_writes(GpuPass::Tonemap))
            } else {
                None
            },
        );

        record_egui_overlay_pass(
            state, &mut encoder, &view,
            egui_primitives.as_ref(), egui_screen.as_ref(), sample_gpu,
        );

        // Free textures, submit, present
        if let Some(ref fo) = egui_output {
            for id in &fo.textures_delta.free {
                state.egui_renderer.free_texture(id);
            }
        }
        if sample_gpu
            && let Some(timer) = state.gpu_timer.as_ref() {
                timer.resolve(&mut encoder);
            }
        state.queue.submit(std::iter::once(encoder.finish()));
        if sample_gpu
            && let Some(timer) = state.gpu_timer.as_ref() {
                self.last_gpu = timer.read_blocking(&state.device).or(self.last_gpu);
            }
        surface_texture.present();

        restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
    }
}

/// Upload this frame's dirty ranges plus mesh/skinned/particle instance data
/// to their GPU buffers; returns the clamped particle instance count.
fn upload_gpu_buffers(
    state:         &RendererState,
    dirty_ranges:  &[(u64, usize)],
    gpu_buf:       &[SdfInstance],
    mesh_list:     Option<&MeshDrawList>,
    skinned_list:  Option<&SkinnedDrawList>,
    particle_list: Option<&ParticleDrawList>,
) -> usize {
    let mut buf_pos = 0usize;
    for &(offset, count) in dirty_ranges {
        let data = bytemuck::cast_slice(&gpu_buf[buf_pos..buf_pos + count]);
        state.queue.write_buffer(&state.instance_buffer, offset, data);
        buf_pos += count;
    }

    if let Some(list) = mesh_list.filter(|l| !l.instances.is_empty()) {
        let n = list.instances.len().min(MAX_MESH_INSTANCES);
        state.queue.write_buffer(
            &state.mesh_instance_buffer, 0,
            bytemuck::cast_slice(&list.instances[..n]),
        );
    }

    // Skinned instances + joint palette.
    if let Some(list) = skinned_list.filter(|l| !l.instances.is_empty()) {
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
        .map(|l| l.instances.len().min(particle_pipeline::MAX_PARTICLES))
        .unwrap_or(0);
    if particle_count > 0 {
        let list = particle_list.expect("count > 0");
        state.queue.write_buffer(
            &state.particle_instance_buffer, 0,
            bytemuck::cast_slice(&list.instances[..particle_count]),
        );
    }
    particle_count
}

/// Shadow pre-pass: fit the sun's ortho volume around the camera target
/// (texel-snapped) and render depth-only variants of every opaque draw into
/// each of `CASCADE_COUNT`'s layers. Particles don't cast.
fn record_shadow_pass(
    state:        &RendererState,
    encoder:      &mut wgpu::CommandEncoder,
    sdf_runs:     &[(u32, u32)],
    sample_gpu:   bool,
    mesh_list:    Option<&MeshDrawList>,
    skinned_list: Option<&SkinnedDrawList>,
    mesh_store:   Option<&MeshStore>,
) {
    let cascades = shadow::fit_cascades(state.camera.target, state.light_dir);
    shadow::write_cascade_uniforms(&state.queue, &state.light_vp_buffer, &state.shadow_cast_buffer, &cascades);

    for (cascade, view) in state.shadow_cascade_views.iter().enumerate() {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shadow Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: if sample_gpu {
                state.gpu_timer.as_ref().map(|t| t.pass_writes(GpuPass::Shadow))
            } else {
                None
            },
            ..Default::default()
        });

        let offset = shadow::cast_offset(cascade as u32);

        // SDF primitives.
        pass.set_pipeline(&state.shadow_pipelines.sdf);
        pass.set_bind_group(0, &state.shadow_bind_group, &[offset]);
        pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, state.instance_buffer.slice(..));
        pass.set_index_buffer(state.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        for &(first, count) in sdf_runs {
            pass.draw_indexed(0..INDICES.len() as u32, 0, first..first + count);
        }

        // Static meshes: opaque first (one pipeline for the whole pass), then
        // MASK primitives on the fragment-discard pipeline (material group 1)
        // so a cutout region doesn't cast a solid-quad shadow.
        if let (Some(list), Some(store)) = (mesh_list, mesh_store)
            && !list.instances.is_empty() {
                pass.set_pipeline(&state.shadow_pipelines.mesh);
                pass.set_vertex_buffer(1, state.mesh_instance_buffer.slice(..));
                for &(mesh_idx, first, count) in &list.shadow_ranges {
                    if first as usize >= MAX_MESH_INSTANCES { break; }
                    let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                    let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                    for prim in &gpu_mesh.primitives {
                        if prim.blend || prim.masked { continue; }
                        pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                        pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                    }
                }
                pass.set_pipeline(&state.shadow_pipelines.mesh_masked);
                for &(mesh_idx, first, count) in &list.shadow_ranges {
                    if first as usize >= MAX_MESH_INSTANCES { break; }
                    let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                    let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                    for prim in &gpu_mesh.primitives {
                        if !prim.masked { continue; }
                        pass.set_bind_group(1, &prim.material_bind_group, &[]);
                        pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                        pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                    }
                }
            }

        // Skinned meshes (re-binds the shared joint palette); opaque then
        // MASK, same reasoning as the static loop above (material group 2 —
        // group 1 is the joint palette here).
        if let (Some(list), Some(store)) = (skinned_list, mesh_store)
            && !list.instances.is_empty() {
                pass.set_pipeline(&state.shadow_pipelines.skinned);
                pass.set_bind_group(1, &state.joint_bind_group, &[]);
                pass.set_vertex_buffer(1, state.skinned_instance_buffer.slice(..));
                for &(mesh_idx, first, count) in &list.shadow_ranges {
                    let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                    for prim in &gpu_mesh.primitives {
                        if prim.blend || prim.masked { continue; }
                        pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                        pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                    }
                }
                pass.set_pipeline(&state.shadow_pipelines.skinned_masked);
                for &(mesh_idx, first, count) in &list.shadow_ranges {
                    let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                    for prim in &gpu_mesh.primitives {
                        if !prim.masked { continue; }
                        pass.set_bind_group(2, &prim.material_bind_group, &[]);
                        pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                        pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                    }
                }
            }
    }
}

/// Depth-only prepass — opaque SDF/mesh/skinned geometry from the main
/// camera's viewpoint into `state.ssao_targets`' full-res depth, the same
/// visibility set `record_main_pass` draws (`list.ranges`, not the shadow
/// pass's light-frustum `shadow_ranges`). Feeds `record_ssao`.
fn record_depth_prepass(
    state:        &RendererState,
    encoder:      &mut wgpu::CommandEncoder,
    sdf_runs:     &[(u32, u32)],
    mesh_list:    Option<&MeshDrawList>,
    skinned_list: Option<&SkinnedDrawList>,
    mesh_store:   Option<&MeshStore>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Depth Prepass"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &state.ssao_targets.prepass_depth_view,
            depth_ops: Some(wgpu::Operations {
                load:  wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        ..Default::default()
    });

    pass.set_pipeline(&state.depth_prepass_pipelines.sdf);
    pass.set_bind_group(0, &state.camera_bind_group, &[]);
    pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
    pass.set_vertex_buffer(1, state.instance_buffer.slice(..));
    pass.set_index_buffer(state.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
    for &(first, count) in sdf_runs {
        pass.draw_indexed(0..INDICES.len() as u32, 0, first..first + count);
    }

    if let (Some(list), Some(store)) = (mesh_list, mesh_store)
        && !list.instances.is_empty() {
            pass.set_pipeline(&state.depth_prepass_pipelines.mesh);
            pass.set_vertex_buffer(1, state.mesh_instance_buffer.slice(..));
            for &(mesh_idx, first, count) in &list.ranges {
                if first as usize >= MAX_MESH_INSTANCES { break; }
                let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                for prim in &gpu_mesh.primitives {
                    if prim.blend || prim.masked { continue; }
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                }
            }
            // MASK primitives: fragment-discard pipeline (material group 1) so
            // a cutout region isn't written into the SSAO prepass depth.
            pass.set_pipeline(&state.depth_prepass_pipelines.mesh_masked);
            for &(mesh_idx, first, count) in &list.ranges {
                if first as usize >= MAX_MESH_INSTANCES { break; }
                let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                for prim in &gpu_mesh.primitives {
                    if !prim.masked { continue; }
                    pass.set_bind_group(1, &prim.material_bind_group, &[]);
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                }
            }
        }

    if let (Some(list), Some(store)) = (skinned_list, mesh_store)
        && !list.instances.is_empty() {
            pass.set_pipeline(&state.depth_prepass_pipelines.skinned);
            pass.set_bind_group(1, &state.joint_bind_group, &[]);
            pass.set_vertex_buffer(1, state.skinned_instance_buffer.slice(..));
            for &(mesh_idx, first, count) in &list.ranges {
                let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                for prim in &gpu_mesh.primitives {
                    if prim.blend || prim.masked { continue; }
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                }
            }
            pass.set_pipeline(&state.depth_prepass_pipelines.skinned_masked);
            for &(mesh_idx, first, count) in &list.ranges {
                let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                for prim in &gpu_mesh.primitives {
                    if !prim.masked { continue; }
                    pass.set_bind_group(2, &prim.material_bind_group, &[]);
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                }
            }
        }
}

/// GTAO from the depth prepass — prefilter, main, and denoise compute passes
/// into `state.ssao_targets`.
fn record_ssao(state: &RendererState, encoder: &mut wgpu::CommandEncoder) {
    state.gtao.encode(encoder, &state.camera_bind_group);
}

/// Main 3D pass — MSAA HDR opaque + sky. Color/depth stay live for the
/// particle pass, which resolves at its end.
#[allow(clippy::too_many_arguments)]
fn record_main_pass(
    state:              &RendererState,
    encoder:            &mut wgpu::CommandEncoder,
    sdf_runs:           &[(u32, u32)],
    sample_gpu:         bool,
    mesh_list:          Option<&MeshDrawList>,
    skinned_list:       Option<&SkinnedDrawList>,
    mesh_store:         Option<&MeshStore>,
    transparent_draws:  &[TransparentDraw],
) {
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
        timestamp_writes: if sample_gpu {
            state.gpu_timer.as_ref().map(|t| t.pass_writes(GpuPass::Main))
        } else {
            None
        },
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
    for &(first, count) in sdf_runs {
        pass.draw_indexed(0..INDICES.len() as u32, 0, first..first + count);
    }

    // Mesh pass — same render pass and camera bind group, real geometry.
    // Ranges are sorted by first-instance, so overflow past the buffer
    // cap ends the loop rather than wrapping.
    if let (Some(list), Some(store)) = (mesh_list, mesh_store)
        && !list.instances.is_empty() {
            pass.set_pipeline(&state.mesh_pipeline);
            pass.set_bind_group(2, &state.environment.bind_group, &[]);
            pass.set_bind_group(3, &state.detail_bind_group, &[]);
            pass.set_vertex_buffer(1, state.mesh_instance_buffer.slice(..));
            for &(mesh_idx, first, count) in &list.ranges {
                if first as usize >= MAX_MESH_INSTANCES { break; }
                let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                for prim in &gpu_mesh.primitives {
                    if prim.blend { continue; }
                    pass.set_bind_group(1, &prim.material_bind_group, &[]);
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                }
            }
        }

    // Skinned mesh pass — same camera bind group, plus the joint
    // palette (group 2). Instances index their own joint block via the
    // joint_base instance attribute, so one draw per mesh still works.
    if let (Some(list), Some(store)) = (skinned_list, mesh_store)
        && !list.instances.is_empty() {
            pass.set_pipeline(&state.skinned_pipeline);
            pass.set_bind_group(0, &state.camera_bind_group, &[]);
            pass.set_bind_group(2, &state.joint_bind_group, &[]);
            pass.set_bind_group(3, &state.environment.bind_group, &[]);
            pass.set_vertex_buffer(1, state.skinned_instance_buffer.slice(..));
            for &(mesh_idx, first, count) in &list.ranges {
                let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                for prim in &gpu_mesh.primitives {
                    if prim.blend { continue; }
                    pass.set_bind_group(1, &prim.material_bind_group, &[]);
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                }
            }
        }

    // Sky pass — the IBL cubemap as background, pinned to the far
    // plane behind everything opaque.
    pass.set_pipeline(&state.sky_pipeline);
    pass.set_bind_group(0, &state.camera_bind_group, &[]);
    pass.set_bind_group(1, &state.environment.sky_bind_group, &[]);
    pass.draw(0..3, 0..1);

    // Transparent pass — the sorted back-to-front sequence `collect_transparent_draws`
    // built, spanning both static and skinned instances through their premultiplied
    // pipelines, drawn last so the sky and every opaque draw are already resolved.
    if let Some(store) = mesh_store {
        let mut current_skinned: Option<bool> = None;
        for draw in transparent_draws {
            if current_skinned != Some(draw.skinned) {
                current_skinned = Some(draw.skinned);
                if draw.skinned {
                    pass.set_pipeline(&state.skinned_transparent_pipeline);
                    pass.set_bind_group(0, &state.camera_bind_group, &[]);
                    pass.set_bind_group(2, &state.joint_bind_group, &[]);
                    pass.set_bind_group(3, &state.environment.bind_group, &[]);
                    pass.set_vertex_buffer(1, state.skinned_instance_buffer.slice(..));
                } else {
                    pass.set_pipeline(&state.mesh_transparent_pipeline);
                    pass.set_bind_group(0, &state.camera_bind_group, &[]);
                    pass.set_bind_group(2, &state.environment.bind_group, &[]);
                    pass.set_bind_group(3, &state.detail_bind_group, &[]);
                    pass.set_vertex_buffer(1, state.mesh_instance_buffer.slice(..));
                }
            }
            let Some(gpu_mesh) = store.meshes.get(draw.mesh_idx) else { continue };
            let Some(prim) = gpu_mesh.primitives.get(draw.prim_idx) else { continue };
            pass.set_bind_group(1, &prim.material_bind_group, &[]);
            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..prim.index_count, 0, draw.instance..draw.instance + 1);
        }
    }
}

/// One transparent-primitive instance queued for the sorted replay after the
/// sky draw: which primitive, which instance slot, and its eye-distance for
/// back-to-front ordering.
pub(crate) struct TransparentDraw {
    skinned:  bool,
    mesh_idx: usize,
    prim_idx: usize,
    instance: u32,
    depth_sq: f32,
}

/// Gather every `blend` primitive instance from the static and skinned draw
/// lists into `out`, sorted back-to-front by squared distance from `eye` —
/// the single sequence `record_main_pass` replays after the sky. Applies the
/// same `first >= MAX_MESH_INSTANCES` clamp as the opaque static loop;
/// skinned ranges need no extra cap since sync already enforces
/// `MAX_SKINNED_INSTANCES`.
fn collect_transparent_draws(
    store:        &MeshStore,
    mesh_list:    Option<&MeshDrawList>,
    skinned_list: Option<&SkinnedDrawList>,
    eye:          glam::Vec3,
    out:          &mut Vec<TransparentDraw>,
) {
    out.clear();

    if let Some(list) = mesh_list {
        for &(mesh_idx, first, count) in &list.ranges {
            if first as usize >= MAX_MESH_INSTANCES { break; }
            let count = count.min(MAX_MESH_INSTANCES as u32 - first);
            let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
            for (prim_idx, prim) in gpu_mesh.primitives.iter().enumerate() {
                if !prim.blend { continue; }
                for instance in first..first + count {
                    let model = glam::Mat4::from_cols_array_2d(&list.instances[instance as usize].model);
                    let depth_sq = eye.distance_squared(model.transform_point3(prim.centroid()));
                    out.push(TransparentDraw { skinned: false, mesh_idx, prim_idx, instance, depth_sq });
                }
            }
        }
    }

    if let Some(list) = skinned_list {
        for &(mesh_idx, first, count) in &list.ranges {
            let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
            for (prim_idx, prim) in gpu_mesh.primitives.iter().enumerate() {
                if !prim.blend { continue; }
                for instance in first..first + count {
                    let model = glam::Mat4::from_cols_array_2d(&list.instances[instance as usize].model);
                    let depth_sq = eye.distance_squared(model.transform_point3(prim.centroid()));
                    out.push(TransparentDraw { skinned: true, mesh_idx, prim_idx, instance, depth_sq });
                }
            }
        }
    }

    out.sort_by(|a, b| b.depth_sq.total_cmp(&a.depth_sq));
}

/// Particle pass: depth read-only so the shader can sample the scene depth
/// for the soft fade; additive first, then premultiplied alpha; the MSAA
/// resolve happens at the end of this pass.
fn record_particle_pass(
    state:          &RendererState,
    encoder:        &mut wgpu::CommandEncoder,
    sample_gpu:     bool,
    particle_list:  Option<&ParticleDrawList>,
    particle_count: usize,
) {
    let additive_count = particle_list
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
        timestamp_writes: if sample_gpu {
            state.gpu_timer.as_ref().map(|t| t.pass_writes(GpuPass::Particles))
        } else {
            None
        },
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

/// Egui overlay pass (Load existing pixels — don't clear the 3D scene).
fn record_egui_overlay_pass(
    state:           &mut RendererState,
    encoder:         &mut wgpu::CommandEncoder,
    view:            &wgpu::TextureView,
    egui_primitives: Option<&Vec<egui::ClippedPrimitive>>,
    egui_screen:     Option<&egui_wgpu::ScreenDescriptor>,
    sample_gpu:      bool,
) {
    if let (Some(prims), Some(sd)) = (egui_primitives, egui_screen) {
        state.egui_renderer.update_buffers(
            &state.device, &state.queue, encoder, prims, sd,
        );
        let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Load, // preserve 3D scene below
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: if sample_gpu {
                state.gpu_timer.as_ref().map(|t| t.pass_writes(GpuPass::Egui))
            } else {
                None
            },
            ..Default::default()
        });
        let mut rpass_static = rpass.forget_lifetime();
        state.egui_renderer.render(&mut rpass_static, prims, sd);
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

#[cfg(all(test, feature = "offscreen"))]
mod tests {
    use super::*;
    use crate::anim::{AnimationClip, Joint, JointTracks, LocalTransform, Skeleton};
    use crate::mesh::{AlphaMode, MaterialData, MeshData, PrimitiveData};
    use crate::mesh_pipeline::{self, MeshInstance, MeshVertex};
    use crate::mipgen::MipGenerator;
    use crate::offscreen::HeadlessGpu;
    use crate::skinned_pipeline::SkinnedMeshInstance;
    use glam::{Mat4, Vec3};

    fn tri_vertex(x: f32, y: f32) -> MeshVertex {
        MeshVertex {
            position: [x, y, 0.0],
            normal:   [0.0, 0.0, 1.0],
            uv:       [x, y],
            tangent:  [1.0, 0.0, 0.0, 1.0],
        }
    }

    fn triangle_primitive(blend: bool) -> PrimitiveData {
        PrimitiveData {
            vertices: vec![tri_vertex(0.0, 0.0), tri_vertex(1.0, 0.0), tri_vertex(0.0, 1.0)],
            indices:  vec![0, 1, 2],
            material: MaterialData {
                alpha_mode: if blend { AlphaMode::Blend } else { AlphaMode::Opaque },
                ..Default::default()
            },
            skin: None,
        }
    }

    /// The `stub_skin` construction pattern from sync.rs:315-331, inlined —
    /// this module needs only the `Skeleton` half (`MeshData::skeleton`),
    /// not the `CpuSkin` sync.rs builds around it.
    fn stub_skeleton(joint_count: usize) -> Skeleton {
        let joints = (0..joint_count)
            .map(|i| Joint {
                parent:       if i == 0 { None } else { Some(i - 1) },
                inverse_bind: Mat4::IDENTITY,
                rest:         LocalTransform::IDENTITY,
                name:         format!("bone{i}"),
            })
            .collect();
        Skeleton { joints, root: Mat4::IDENTITY }
    }

    fn translated(z: f32) -> [[f32; 4]; 4] {
        Mat4::from_translation(Vec3::new(0.0, 0.0, z)).to_cols_array_2d()
    }

    #[test]
    fn collector_skips_opaque_and_sorts_back_to_front_across_static_and_skinned() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — collect_transparent_draws test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();

        // mesh0: opaque prim0 + blend prim1 ("glass+solid").
        let mesh0 = store.register(
            &gpu.device, &gpu.queue, &layout, &mipgen, "glass+solid",
            MeshData {
                primitives: vec![triangle_primitive(false), triangle_primitive(true)],
                skeleton:   None,
                clips:      vec![],
            },
        );
        // mesh1: one blend prim ("glass2").
        let mesh1 = store.register(
            &gpu.device, &gpu.queue, &layout, &mipgen, "glass2",
            MeshData {
                primitives: vec![triangle_primitive(true)],
                skeleton:   None,
                clips:      vec![],
            },
        );
        // mesh2: skinned, one blend prim, 1-joint stub skeleton.
        let mesh2 = store.register(
            &gpu.device, &gpu.queue, &layout, &mipgen, "skinned-glass",
            MeshData {
                primitives: vec![triangle_primitive(true)],
                skeleton:   Some(stub_skeleton(1)),
                clips:      vec![AnimationClip {
                    name:     "clip_a".into(),
                    duration: 1.0,
                    tracks:   vec![JointTracks::default(); 1],
                }],
            },
        );

        let mesh_list = MeshDrawList {
            instances: vec![
                MeshInstance { model: translated(0.0), tint: [1.0; 4] },
                MeshInstance { model: translated(-10.0), tint: [1.0; 4] },
            ],
            ranges: vec![(mesh0, 0, 1), (mesh1, 1, 1)],
            shadow_ranges: vec![],
        };
        let skinned_list = SkinnedDrawList {
            instances: vec![SkinnedMeshInstance {
                model:      translated(-5.0),
                tint:       [1.0; 4],
                joint_base: 0,
                _pad:       [0; 3],
            }],
            joints: vec![],
            ranges: vec![(mesh2, 0, 1)],
            shadow_ranges: vec![],
        };

        let eye = Vec3::new(0.0, 0.0, 10.0);
        let mut out = Vec::new();
        collect_transparent_draws(&store, Some(&mesh_list), Some(&skinned_list), eye, &mut out);

        assert_eq!(out.len(), 3, "opaque prim0 must never appear");
        assert!(
            !out.iter().any(|d| d.mesh_idx == mesh0 && d.prim_idx == 0),
            "opaque primitive leaked into transparent draws"
        );

        // Descending depth: z=-10 (mesh1) first, z=-5 skinned (mesh2) second, z=0 (mesh0) last.
        assert_eq!((out[0].mesh_idx, out[0].prim_idx, out[0].instance, out[0].skinned), (mesh1, 0, 1, false));
        assert_eq!((out[1].mesh_idx, out[1].prim_idx, out[1].instance, out[1].skinned), (mesh2, 0, 0, true));
        assert_eq!((out[2].mesh_idx, out[2].prim_idx, out[2].instance, out[2].skinned), (mesh0, 1, 0, false));
        assert!(out[0].depth_sq > out[1].depth_sq, "mesh1 must sort before the skinned instance");
        assert!(out[1].depth_sq > out[2].depth_sq, "the skinned instance must sort before mesh0");
    }

    #[test]
    fn collector_breaks_on_mesh_range_past_instance_cap() {
        let Some(gpu) = HeadlessGpu::new() else {
            eprintln!("SKIP: no GPU adapter available — collect_transparent_draws cap test needs one");
            return;
        };
        let layout = mesh_pipeline::create_material_bind_group_layout(&gpu.device);
        let mipgen = MipGenerator::new(&gpu.device);
        let mut store = MeshStore::default();
        let mesh0 = store.register(
            &gpu.device, &gpu.queue, &layout, &mipgen, "glass-only",
            MeshData { primitives: vec![triangle_primitive(true)], skeleton: None, clips: vec![] },
        );

        let mesh_list = MeshDrawList {
            instances: vec![MeshInstance { model: translated(0.0), tint: [1.0; 4] }],
            ranges:    vec![(mesh0, 0, 1), (mesh0, MAX_MESH_INSTANCES as u32, 1)],
            shadow_ranges: vec![],
        };

        let eye = Vec3::new(0.0, 0.0, 10.0);
        let mut out = Vec::new();
        collect_transparent_draws(&store, Some(&mesh_list), None, eye, &mut out);

        assert_eq!(out.len(), 1, "the range starting at MAX_MESH_INSTANCES must contribute nothing");
    }
}
