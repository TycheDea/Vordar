pub mod anim;
pub(crate) mod bloom;
pub mod camera;
pub mod dev_overlay;
pub(crate) mod facade;
pub(crate) mod frame;
pub(crate) mod gpu_timer;
pub(crate) mod ibl;
pub mod instance;
pub(crate) mod instance_sync;
pub mod menu;
pub(crate) mod menu_actions;
pub mod mesh;
pub(crate) mod mesh_pipeline;
pub(crate) mod mipgen;
#[cfg(feature = "offscreen")]
pub mod offscreen;
pub mod particle_pipeline;
pub(crate) mod post;
pub(crate) mod shadow;
pub(crate) mod sky;
pub mod tangent;
pub mod sdf_pipeline;
pub(crate) mod skinned_pipeline;
pub(crate) mod state;
pub mod texture;
pub mod ui_layers;

pub use dev_overlay::DevOverlaySystem;
pub use facade::{alloc_render_slot, alloc_shape_group_slots, camera_movement_axes, camera_yaw, clear_texture, create_checker_texture, free_render_slot, load_texture, register_procedural_mesh, screen_to_ground, set_camera_target, set_environment, set_exposure, set_fog, set_light, set_texture, unproject_to_ground, update_camera, zoom_camera, CameraConfig, TextureHandle};
pub use instance_sync::RenderSyncSystem;
pub use menu::{MenuState, MenuSystem};
pub use mesh::{MeshDrawList, MeshRenderSyncSystem, MeshStore, SkinnedDrawList, SocketConfig, SocketTransforms};
pub use mesh_pipeline::MeshVertex;
pub use particle_pipeline::{ParticleDrawList, ParticleInstance, ATLAS_GRID, MAX_PARTICLES};
pub use ui_layers::UiLayers;
use frame::RenderSystem;
pub(crate) use state::{RendererState, init, on_resize};

use engine_app::app::App;
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, SystemOrder};
use crate::instance_sync::{SaveTransformSystem, RenderSlotDespawnSystem, RenderSlotAttachSystem};
use crate::camera::CycleCameraSystem;

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
            .add_system(CycleCameraSystem,          Phase::PostUpdate,   SystemOrder::First)
            // F3 toggles the dev stats overlay.
            .add_system(DevOverlaySystem,           Phase::PostUpdate,   SystemOrder::First)
            // Attach slots to slotless renderables, then sync transforms to the GPU pool.
            .add_system(RenderSlotAttachSystem,    Phase::RenderSync,   SystemOrder::First)
            .add_system(RenderSyncSystem,          Phase::RenderSync,   SystemOrder::Default)
            .add_system(MeshRenderSyncSystem::new(), Phase::RenderSync, SystemOrder::Default)
            .add_system(RenderSystem::new(),       Phase::Render,       SystemOrder::Default);
    }
}

