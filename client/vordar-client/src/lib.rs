// vordar-client — the presentation half of the game: turns devices + camera
// into intent events, follows the simulation with the camera, and (networked)
// replicates server snapshots into the local world. Everything here may touch
// winit and the renderer; the shared simulation (vordar-game) never does.

pub mod net;
pub mod presentation;
pub mod ui;

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

/// Client-side cooldowns for the player's skills (action bar slots 1 + 2).
pub struct CastState {
    pub bolt: Cooldown,
    pub blast: Cooldown,
}

impl CastState {
    pub fn new() -> Self {
        let blast_secs = vordar_game::skills::skill("blast")
            .map(|s| s.cooldown_micros as f32 / 1e6)
            .unwrap_or(3.0);
        Self {
            bolt: Cooldown::new(vordar_game::skills::BOLT_COOLDOWN_SECS),
            blast: Cooldown::new(blast_secs),
        }
    }

    pub fn tick(&mut self, delta: f32) {
        self.bolt.tick(delta);
        self.blast.tick(delta);
    }
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

/// Client-side presentation plugin for offline play: input → intents, camera
/// follow, zone dressing (floor + portals), minimap, left-click bolt.
pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        // ClearEventsSystem runs SystemOrder::First in Input, so Default lands after it.
        app.insert_resource(CastState::new())
            .insert_resource(presentation::CurrentZone("start".into()))
            .insert_resource(vordar_game::zones::load_zones("content/zones/zones.ron"))
            .add_system(PlayerInputSystem, Phase::Input,      SystemOrder::Default)
            .add_system(presentation::SandboxCastSystem, Phase::Input, SystemOrder::Default)
            .add_system(presentation::ZoneDressingSystem::new(), Phase::Update, SystemOrder::Default)
            .add_system(CameraFollowSystem, Phase::RenderSync, SystemOrder::First);
        ui::install(app);
    }
}
