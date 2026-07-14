pub mod anim;
pub(crate) mod bloom;
pub mod camera;
pub mod dev_overlay;
pub(crate) mod facade;
pub(crate) mod frame;
pub(crate) mod ibl;
pub mod instance;
pub mod menu;
pub(crate) mod menu_actions;
pub mod mesh;
pub(crate) mod mesh_pipeline;
pub(crate) mod mipgen;
pub mod offscreen;
pub mod particle_pipeline;
pub(crate) mod post;
pub(crate) mod shadow;
pub mod tangent;
pub mod pipeline;
pub(crate) mod skinned_pipeline;
pub(crate) mod state;
pub mod texture;
pub mod ui_layers;

pub use dev_overlay::DevOverlaySystem;
pub use facade::{alloc_render_slot, alloc_shape_group_slots, camera_movement_axes, camera_yaw, clear_texture, create_checker_texture, free_render_slot, load_texture, register_procedural_mesh, screen_to_ground, set_camera_target, set_environment, set_exposure, set_fog, set_light, set_texture, unproject_to_ground, update_camera, zoom_camera, CameraConfig, TextureHandle};
pub use menu::{MenuState, MenuSystem};
pub use mesh::{MeshDrawList, MeshRenderSyncSystem, MeshStore, SkinnedDrawList, SocketConfig, SocketTransforms};
pub use mesh_pipeline::MeshVertex;
pub use particle_pipeline::{ParticleDrawList, ParticleInstance, ATLAS_GRID, MAX_PARTICLES};
pub use ui_layers::UiLayers;
use frame::RenderSystem;
pub(crate) use state::{RendererState, init, on_resize};

use engine_core::traits::Resources;
use engine_core::World;
use engine_app::app::App;
use engine_app::plugin::Plugin;
use engine_app::scheduler::{InterpolationAlpha, Phase, System, SystemOrder};
use engine_app::input::KeyboardState;
use engine_core::traits::DespawnQueue;
use engine_core::components::{PreviousTransform, RenderShape, RenderShapeType, ShapeGroup, Transform};
use glam::Mat4;
use winit::keyboard::KeyCode;
use crate::camera::CameraUniform;
use crate::instance::{InstancePool, InstanceSlot, ShapeGroupSlots, SdfInstance};

// ── Systems ───────────────────────────────────────────────────────────────────

/// Saves each entity's current position into PreviousTransform at the start of
/// every fixed step. Register in Phase::Update, SystemOrder::First so it runs
/// before any movement system mutates Transform.
pub struct SaveTransformSystem;

impl System for SaveTransformSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, _delta: f32) {
        for (transform, prev) in world.query::<(&Transform, &mut PreviousTransform)>().iter() {
            prev.position = transform.position;
        }
    }
}

pub struct RenderSyncSystem;

impl System for RenderSyncSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let alpha = resources.get::<InterpolationAlpha>().map(|a| a.0).unwrap_or(1.0);
        let pool = resources.get_mut::<InstancePool>()
            .expect("InstancePool not in resources");

        for (transform, prev, render_shape, slot) in
            world.query::<(&Transform, Option<&PreviousTransform>, &RenderShape, &InstanceSlot)>().iter()
        {
            // Lerp position if PreviousTransform is present; otherwise use current.
            let render_pos = match prev {
                Some(p) => p.position.lerp(transform.position, alpha),
                None    => transform.position,
            };
            let render_transform = Transform {
                position: render_pos,
                rotation: transform.rotation,
                scale:    transform.scale,
            };
            let (shape_type, shape_params) = shape_to_gpu(render_shape.shape);
            let new_inst = SdfInstance {
                model:       render_transform.to_model_matrix().to_cols_array_2d(),
                color:       render_shape.color.to_array(),
                shape_type,
                shape_params,
            };
            if bytemuck::bytes_of(&pool.slots[slot.0]) != bytemuck::bytes_of(&new_inst) {
                pool.slots[slot.0] = new_inst;
                pool.dirty[slot.0] = true;
            }
        }

        for (transform, prev, group, slots) in
            world.query::<(&Transform, Option<&PreviousTransform>, &ShapeGroup, &ShapeGroupSlots)>().iter()
        {
            let render_pos = match prev {
                Some(p) => p.position.lerp(transform.position, alpha),
                None    => transform.position,
            };
            let parent_model = Transform {
                position: render_pos,
                rotation: transform.rotation,
                scale:    transform.scale,
            }.to_model_matrix();

            for (sub, key) in group.shapes.iter().zip(slots.0.iter()) {
                let sub_model = parent_model
                    * Mat4::from_scale_rotation_translation(sub.scale, sub.rotation, sub.offset);
                let (shape_type, shape_params) = shape_to_gpu(sub.shape);
                let new_inst = SdfInstance {
                    model:       sub_model.to_cols_array_2d(),
                    color:       sub.color.to_array(),
                    shape_type,
                    shape_params,
                };
                if bytemuck::bytes_of(&pool.slots[*key]) != bytemuck::bytes_of(&new_inst) {
                    pool.slots[*key] = new_inst;
                    pool.dirty[*key] = true;
                }
            }
        }
    }
}

/// Cycles the camera projection mode (Perspective → Isometric → TopDown → …) on C press.
/// Register in Phase::PostUpdate, SystemOrder::First so it runs before CameraFollowSystem.
pub struct CycleCameraSystem {
    was_pressed: bool,
}

impl CycleCameraSystem {
    pub fn new() -> Self { Self { was_pressed: false } }
}

impl System for CycleCameraSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let pressed = resources
            .get::<KeyboardState>()
            .map(|kb| kb.is_pressed(KeyCode::KeyC))
            .unwrap_or(false);

        if pressed && !self.was_pressed {
            let state = resources.get_mut::<RendererState>()
                .expect("RendererState not in resources");
            state.camera.cycle_projection();
            let uniform = CameraUniform::from_camera(&state.camera);
            state.queue.write_buffer(&state.camera_buffer, 0, bytemuck::cast_slice(&[uniform]));
        }

        self.was_pressed = pressed;
    }
}

/// Frees render slots for entities queued for despawn — must run before DespawnFlushSystem.
/// Register via `register_render_cleanup(app)`.
pub struct RenderSlotDespawnSystem;

impl System for RenderSlotDespawnSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Collect entities without holding the DespawnQueue borrow into the loop.
        let entities: Vec<_> = resources
            .get::<DespawnQueue>()
            .map(|q| q.0.iter().map(|(e, _)| *e).collect())
            .unwrap_or_default();

        let pool = resources.get_mut::<InstancePool>()
            .expect("InstancePool not in resources");

        for entity in entities {
            if let Ok(slot) = world.get::<&InstanceSlot>(entity) {
                pool.free(slot.0);
            }
            if let Ok(slots) = world.get::<&ShapeGroupSlots>(entity) {
                for &key in &slots.0 { pool.free(key); }
            }
        }
    }
}

/// Allocates GPU instance slots for entities that have a RenderShape/ShapeGroup
/// but no slot yet — entities spawned from data (prefabs) need no renderer access
/// at spawn time. Runs in Phase::RenderSync, First, so freshly flushed entities
/// are visible the same frame. Steady-state cost is ~zero: the matched archetype
/// set is empty once every renderable entity holds its slot.
pub struct RenderSlotAttachSystem;

impl System for RenderSlotAttachSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Collect first — the query borrow must end before insert_one mutates the world.
        let singles: Vec<hecs::Entity> = world
            .query::<(hecs::Entity, &RenderShape)>().without::<&InstanceSlot>()
            .iter().map(|(e, _)| e).collect();
        let groups: Vec<(hecs::Entity, usize)> = world
            .query::<(hecs::Entity, &ShapeGroup)>().without::<&ShapeGroupSlots>()
            .iter().map(|(e, g)| (e, g.shapes.len())).collect();

        for entity in singles {
            let slot = alloc_render_slot(resources);
            let _ = world.insert_one(entity, slot);
        }
        for (entity, count) in groups {
            let slots = alloc_shape_group_slots(count, resources);
            let _ = world.insert_one(entity, slots);
        }
    }
}

/// Registers the full renderer: window/init callbacks and all render systems.
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.on_window_ready(init)
            .on_resize_fn(on_resize)
            // Save position snapshot before movement systems run.
            .add_system(SaveTransformSystem,       Phase::Update,       SystemOrder::First)
            // Free render slots before DespawnFlushSystem removes entities.
            .add_system(RenderSlotDespawnSystem,   Phase::DespawnFlush, SystemOrder::First)
            // Keyboard navigation for the pause menu.
            .add_system(MenuSystem,                Phase::PostUpdate,   SystemOrder::First)
            // C key cycles Perspective → Isometric → TopDown.
            .add_system(CycleCameraSystem::new(),  Phase::PostUpdate,   SystemOrder::First)
            // F3 toggles the dev stats overlay.
            .add_system(DevOverlaySystem::new(),   Phase::PostUpdate,   SystemOrder::First)
            // Attach slots to slotless renderables, then sync transforms to the GPU pool.
            .add_system(RenderSlotAttachSystem,    Phase::RenderSync,   SystemOrder::First)
            .add_system(RenderSyncSystem,          Phase::RenderSync,   SystemOrder::Default)
            .add_system(MeshRenderSyncSystem::new(), Phase::RenderSync, SystemOrder::Default)
            .add_system(RenderSystem::new(),       Phase::Render,       SystemOrder::Default);
    }
}

fn shape_to_gpu(shape: RenderShapeType) -> (u32, [f32; 4]) {
    match shape {
        RenderShapeType::Cube                         => (0, [0.0; 4]),
        RenderShapeType::Sphere                       => (1, [0.0; 4]),
        RenderShapeType::Diamond                      => (2, [0.0; 4]),
        RenderShapeType::RoundedBox { corner_radius } => (3, [corner_radius, 0.0, 0.0, 0.0]),
        RenderShapeType::Cylinder                     => (4, [0.0; 4]),
        RenderShapeType::Capsule                      => (5, [0.0; 4]),
        RenderShapeType::Custom { shape_type, params } => (shape_type, params),
    }
}

