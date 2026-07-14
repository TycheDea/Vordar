// vordar-client — the presentation half of the game: turns devices + camera
// into intent events, follows the simulation with the camera, and (networked)
// replicates server snapshots into the local world. Everything here may touch
// winit and the renderer; the shared simulation (vordar-game) never does.

pub mod body;
pub mod credentials;
pub mod ground;
pub mod locomotion;
pub mod net;
pub mod pose;
pub mod presentation;
pub mod react;
pub mod telegraph;
pub mod ui;
pub mod vfx;
pub mod weapons;
pub mod world_time;

use engine_app::app::App;
use engine_app::events::EventBus;
use engine_app::input::{KeyboardState, MouseState};
use engine_app::plugin::Plugin;
use engine_app::scheduler::{InterpolationAlpha, Phase, System, SystemOrder};
use engine_core::components::{PreviousTransform, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use glam::{Vec2, Vec3};
use hecs::Entity;
use vordar_game::Player;
use vordar_game::events::MoveIntent;
use winit::keyboard::KeyCode;

/// Camera orbit rate, radians per second.
const ORBIT_SPEED: f32 = 1.8;
/// Camera distance change per mouse-wheel line.
const ZOOM_STEP: f32 = 2.0;

/// One skill's client-side cooldown view. The server is the real authority;
/// this just avoids spamming casts that would be rejected anyway, and feeds
/// the action bar.
pub struct Cooldown {
    left: f32,
    total: f32,
}

impl Cooldown {
    pub fn new(total: f32) -> Self {
        Self { left: 0.0, total }
    }

    pub fn tick(&mut self, delta: f32) {
        self.left = (self.left - delta).max(0.0);
    }

    pub fn ready(&self) -> bool {
        self.left <= 0.0
    }

    /// Restart the cooldown (call when a cast is committed).
    pub fn fire(&mut self) {
        self.left = self.total;
    }

    /// Remaining fraction (1.0 just fired → 0.0 ready); None when ready.
    pub fn remaining_frac(&self) -> Option<f32> {
        (self.left > 0.0 && self.total > 0.0).then(|| self.left / self.total)
    }
}

/// Client-side cooldowns for the local class's abilities — one per action-bar
/// slot, in the class's authored order (slot 0 = LMB, 1 = Q, 2 = E). Rebuilt
/// when the local player's class becomes known or changes.
pub struct CastState {
    pub class: Option<String>,
    pub abilities: Vec<Cooldown>,
}

impl CastState {
    pub fn new() -> Self {
        Self { class: None, abilities: Vec::new() }
    }

    /// Rebuild the slots for `class` (cooldowns in seconds, slot order).
    /// No-op while the class is unchanged, so in-flight cooldowns survive.
    pub fn sync(&mut self, class: &str, cooldown_secs: &[f32]) {
        if self.class.as_deref() == Some(class) {
            return;
        }
        self.class = Some(class.to_owned());
        self.abilities = cooldown_secs.iter().map(|&s| Cooldown::new(s)).collect();
    }

    pub fn tick(&mut self, delta: f32) {
        for cooldown in &mut self.abilities {
            cooldown.tick(delta);
        }
    }

    pub fn ready(&self, slot: usize) -> bool {
        self.abilities.get(slot).map(|c| c.ready()).unwrap_or(false)
    }

    pub fn fire(&mut self, slot: usize) {
        if let Some(cooldown) = self.abilities.get_mut(slot) {
            cooldown.fire();
        }
    }
}

/// The local player's class id, once its entity exists — the net own-entity
/// when online, the first Player entity in the sandbox.
pub fn local_class(world: &World, resources: &Resources) -> Option<String> {
    let entity = crate::net::own_entity(resources)
        .or_else(|| world.query::<(Entity, &Player)>().iter().next().map(|(e, _)| e))?;
    world.get::<&vordar_game::class::ClassId>(entity).ok().map(|c| c.id.clone())
}

/// WASD against the current camera axes → desired world-XZ direction (≤ unit).
/// The single place keyboard state becomes movement input — used by the local
/// input system (sandbox) and the network send system (online play).
pub fn read_move_dir(resources: &Resources) -> Vec2 {
    let (forward, right) = engine_renderer::camera_movement_axes(resources);
    let kb = resources.get::<KeyboardState>().expect("KeyboardState not in resources");

    let mut dir = Vec3::ZERO;
    if kb.is_pressed(KeyCode::KeyW) { dir += forward; }
    if kb.is_pressed(KeyCode::KeyS) { dir -= forward; }
    if kb.is_pressed(KeyCode::KeyD) { dir += right;   }
    if kb.is_pressed(KeyCode::KeyA) { dir -= right;   }
    let dir = if dir.length_squared() > 0.0 { dir.normalize() } else { Vec3::ZERO };
    Vec2::new(dir.x, dir.z)
}

/// Arrow-key orbit + follow `target`, shared by the sandbox and networked
/// camera systems. Runs once per display frame (`delta` = frame time).
///   Arrow Left / Right — rotate around target (yaw)
///   Arrow Up   / Down  — raise / lower the viewing angle (pitch)
pub fn orbit_and_follow(target: Option<Vec3>, resources: &mut Resources, delta: f32) {
    let step = ORBIT_SPEED * delta;
    let (yaw, pitch) = {
        let kb = resources.get::<KeyboardState>().expect("KeyboardState not in resources");
        let yaw = if kb.is_pressed(KeyCode::ArrowLeft)  { -step }
                  else if kb.is_pressed(KeyCode::ArrowRight) { step }
                  else { 0.0 };
        let pitch = if kb.is_pressed(KeyCode::ArrowUp)   {  step }
                    else if kb.is_pressed(KeyCode::ArrowDown) { -step }
                    else { 0.0 };
        (yaw, pitch)
    };
    // Mouse wheel = dolly; this is the one wheel consumer per display frame.
    let wheel = resources.get_mut::<MouseState>().map(|m| m.take_wheel()).unwrap_or(0.0);
    if wheel != 0.0 {
        engine_renderer::zoom_camera(-wheel * ZOOM_STEP, resources);
    }
    engine_renderer::update_camera(target, yaw, pitch, resources);
}

/// The position the renderer will draw `entity` at this frame —
/// `lerp(PreviousTransform, Transform, alpha)`, the same formula as
/// RenderSyncSystem. The camera must follow THIS, not the raw Transform:
/// targeting the fixed-tick position while the entity renders interpolated
/// makes the followed entity vibrate on screen by one tick of movement.
pub fn render_position(world: &World, entity: Entity, resources: &Resources) -> Option<Vec3> {
    let transform = world.get::<&Transform>(entity).ok()?;
    let alpha = resources.get::<InterpolationAlpha>().map(|a| a.0).unwrap_or(1.0);
    Some(match world.get::<&PreviousTransform>(entity) {
        Ok(prev) => prev.position.lerp(transform.position, alpha),
        Err(_) => transform.position,
    })
}

/// Emits a MoveIntent for the local Player entity each Input tick (offline /
/// sandbox play, where this process runs the simulation itself).
pub struct PlayerInputSystem;

impl System for PlayerInputSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let dir = read_move_dir(resources);
        let player = world.query::<(Entity, &Player)>().iter().next().map(|(e, _)| e);
        if let Some(entity) = player {
            let bus = resources.get_mut::<EventBus>().expect("EventBus not in resources");
            bus.emit(MoveIntent { entity, dir });
        }
    }
}

/// Follows the local Player entity at its interpolated render position.
/// Runs Phase::RenderSync (once per display frame): the camera must move at
/// the same cadence as the rendered entities, or the followed player
/// oscillates on screen between fixed ticks.
pub struct CameraFollowSystem;

impl System for CameraFollowSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let player = world.query::<(Entity, &Player)>().iter().next().map(|(e, _)| e);
        let target = player.and_then(|e| render_position(world, e, resources));
        orbit_and_follow(target, resources, delta);
    }
}

/// Follows our own player (identified by the Welcome message) at its
/// interpolated render position. Runs Phase::RenderSync — see
/// CameraFollowSystem for why the camera must move at render cadence.
pub struct NetCameraFollowSystem;

impl System for NetCameraFollowSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let target = net::own_entity(resources).and_then(|e| render_position(world, e, resources));
        orbit_and_follow(target, resources, delta);
    }
}

/// Client-side presentation plugin for offline play: input → intents, camera
/// follow, zone dressing (floor + portals), minimap, left-click bolt.
pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        // ClearEventsSystem runs SystemOrder::First in Input, so Default lands after it.
        app.insert_resource(CastState::new())
            .insert_resource(presentation::CurrentZone("start".into()))
            .insert_resource(vordar_game::zones::load_zones("content/zones/zones.ron"))
            .insert_resource(vfx::ParticleSim::new())
            .add_system(PlayerInputSystem, Phase::Input,      SystemOrder::Default)
            .add_system(presentation::SandboxCastSystem, Phase::Input, SystemOrder::Default)
            .add_system(presentation::ZoneDressingSystem::new(), Phase::Update, SystemOrder::Default)
            .add_system(body::BodyComposeSystem, Phase::Update, SystemOrder::Default)
            .add_system(react::CorpseTtlSystem, Phase::Update, SystemOrder::Default)
            // Corpses must be cloned from dying entities BEFORE the flush removes them.
            .add_system(react::CorpseOnDeathSystem, Phase::DespawnFlush, SystemOrder::First)
            // Impact beats fire where despawning projectiles died (before the flush).
            .add_system(vfx::ImpactBurstSystem, Phase::DespawnFlush, SystemOrder::First)
            .add_system(pose::PoseAnimationSystem, Phase::RenderSync, SystemOrder::before::<engine_renderer::RenderSyncSystem>())
            // Facing + locomotion drive skinned meshes; both must run before the
            // mesh sync so rotation and clip selection are current this frame.
            // Hit reacts run before locomotion so a fresh flinch wins the frame.
            .add_system(react::HitReactSystem, Phase::RenderSync, SystemOrder::before::<locomotion::LocomotionSystem>())
            .add_system(locomotion::FacingSystem, Phase::RenderSync, SystemOrder::before::<engine_renderer::MeshRenderSyncSystem>())
            .add_system(locomotion::LocomotionSystem, Phase::RenderSync, SystemOrder::before::<engine_renderer::MeshRenderSyncSystem>())
            .add_system(vfx::VfxSystem::new(), Phase::RenderSync, SystemOrder::after::<engine_renderer::MeshRenderSyncSystem>())
            // Weapons glue to the freshly rebuilt hand sockets (same slot as VFX).
            .add_system(weapons::WeaponAttachSystem::default(), Phase::RenderSync, SystemOrder::after::<engine_renderer::MeshRenderSyncSystem>())
            .add_system(CameraFollowSystem, Phase::RenderSync, SystemOrder::First);
        ui::install(app);
    }
}
