// Networked-client plugin — replicates the server's world into this one.
//
// Phase 2 model: remote entities are server-driven, interpolated between
// snapshot positions (NetLerp). Our OWN player is predicted: each Input tick
// we send the intent AND emit it locally, so the shared vordar-game movement
// systems apply it immediately. Snapshots then reconcile: rebase onto the
// server's authoritative position and replay the intents the server hasn't
// processed yet (`last_processed_seq`). Both phases run Fixed(60), so one
// sent intent maps 1:1 to one local integration step.

use crate::{orbit_and_follow, read_move_dir};
use engine_app::app::App;
use engine_app::events::EventBus;
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_app::time::Time;
use engine_core::components::{Health, RenderShape, Transform};
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use engine_net::{ClientEvent, NetClient};
use glam::{Vec2, Vec3};
use hecs::Entity;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::Duration;
use vordar_game::events::MoveIntent;
use vordar_game::motion::MovementSystem;
use vordar_game::player::{movement_velocity, PlayerMovementSystem};
use vordar_game::Player;
use vordar_game::world::{active_event, day_night_light, WorldEventsDef};
use vordar_protocol::{decode, encode, ClientMsg, EntityPos, EntityState, ServerMsg, PROTOCOL_VERSION, SNAPSHOT_HZ};

/// Reconciliation error below this is ignored — local prediction is trusted
/// outright. Tick-phase jitter between client and server lives in this band
/// (~1–2 sim ticks of movement); correcting it every snapshot reads as shaking.
const TRUST_DISTANCE: f32 = 0.3;
/// Mispredictions larger than this snap to the reconciled position (forced
/// corrections, teleports); between TRUST and SNAP the error is folded into
/// the predicted position gradually by NetCorrectionSystem.
const SNAP_DISTANCE: f32 = 1.0;
/// Half-life of an outstanding correction — time until half the error has
/// been folded in. Short enough to converge in a few hundred ms, long enough
/// that the per-tick nudge stays below normal movement speed.
const CORRECTION_HALF_LIFE: f32 = 0.15;
/// Safety bound on unacknowledged intents (~4 s at 60 Hz). Hitting it means
/// the server stopped acking; predicting further is pointless.
const MAX_PENDING_INTENTS: usize = 240;

pub struct NetClientPlugin {
    pub server_addr: SocketAddr,
    /// Predict own movement locally; off reproduces the Phase 1 server-driven
    /// feel (one round-trip of input latency) for comparison.
    pub predict: bool,
    /// Artificial round-trip latency added by engine-net (testing knob).
    pub simulated_rtt: Duration,
    /// Character name sent as the first message after connect — identity
    /// without auth during development (accounts land later).
    pub user: String,
}

impl Plugin for NetClientPlugin {
    fn build(&self, app: &mut App) {
        let client = NetClient::connect_with_latency(self.server_addr, PROTOCOL_VERSION, self.simulated_rtt)
            .unwrap_or_else(|e| panic!("failed to start network client: {e}"));
        app.insert_resource(NetClientState {
            client,
            user: self.user.clone(),
            own_id: None,
            entities: HashMap::new(),
            seq: 0,
            predict: self.predict,
            pending: VecDeque::new(),
            correction: Vec3::ZERO,
            simulated_rtt: self.simulated_rtt,
        })
        .add_system(NetReceiveSystem, Phase::Input, SystemOrder::Default)
        .add_system(NetSendInputSystem, Phase::Input, SystemOrder::after::<NetReceiveSystem>())
        .add_system(AbilityCastSystem::new(), Phase::Input, SystemOrder::after::<NetSendInputSystem>())
        .insert_resource(WorldTime { offset_micros: 0, synced: false })
        .insert_resource(crate::CastState::new())
        .insert_resource(crate::presentation::CurrentZone("start".into()))
        .insert_resource(vordar_game::zones::load_zones("content/zones/zones.ron"))
        .insert_resource(crate::vfx::ParticleSim::new())
        .add_system(NetLerpSystem, Phase::Update, SystemOrder::First)
        .add_system(crate::presentation::ZoneDressingSystem::new(), Phase::Update, SystemOrder::Default)
        .add_system(crate::body::BodyComposeSystem, Phase::Update, SystemOrder::Default)
        .add_system(crate::react::CorpseTtlSystem, Phase::Update, SystemOrder::Default)
        .add_system(crate::react::CorpseOnDeathSystem, Phase::DespawnFlush, SystemOrder::First)
        .add_system(crate::pose::PoseAnimationSystem, Phase::RenderSync, SystemOrder::before::<engine_renderer::RenderSyncSystem>())
        // Facing + locomotion drive skinned meshes — same registration as the
        // sandbox plugin; remote entities animate from NetMotion (snapshot
        // deltas), the predicted own player from its real sim Velocity.
        // Hit reacts watch replicated hp (protocol v8) for flinches + sparks.
        .add_system(crate::react::HitReactSystem, Phase::RenderSync, SystemOrder::before::<crate::locomotion::LocomotionSystem>())
        .add_system(crate::locomotion::FacingSystem, Phase::RenderSync, SystemOrder::before::<engine_renderer::MeshRenderSyncSystem>())
        .add_system(crate::locomotion::LocomotionSystem, Phase::RenderSync, SystemOrder::before::<engine_renderer::MeshRenderSyncSystem>())
        .add_system(crate::vfx::VfxSystem::new(), Phase::RenderSync, SystemOrder::after::<engine_renderer::MeshRenderSyncSystem>())
        // Impact beats fire where despawning projectiles died (before the flush).
        .add_system(crate::vfx::ImpactBurstSystem, Phase::DespawnFlush, SystemOrder::First)
        .add_system(NetCameraFollowSystem, Phase::RenderSync, SystemOrder::First)
        .add_system(TelegraphFillSystem, Phase::RenderSync, SystemOrder::First)
        .add_system(DayNightSystem, Phase::RenderSync, SystemOrder::First);
        crate::ui::install(app);
        if self.predict {
            // The shared simulation moves our own player. Remote players stay
            // NetLerp-driven: no intents are emitted for them, so the shared
            // systems hold their velocity at zero. LeapSystem mirrors the
            // server's dash override so an Onslaught moves the own view
            // immediately instead of waiting a round-trip.
            app.add_system(PlayerMovementSystem, Phase::Update, SystemOrder::First)
                .add_system(vordar_game::combat::leap::LeapSystem, Phase::Update, SystemOrder::Default)
                .add_system(MovementSystem, Phase::Update, SystemOrder::Last)
                .add_system(NetCorrectionSystem, Phase::Update, SystemOrder::Last);
        }
    }
}

/// An intent sent to the server but not yet covered by `last_processed_seq`.
/// Replayed on top of each snapshot of our own player.
struct PendingIntent {
    seq: u32,
    dir: Vec2,
    dt: f32,
}

pub struct NetClientState {
    client: NetClient,
    user: String,
    own_id: Option<u64>,
    /// server entity id → local entity
    entities: HashMap<u64, Entity>,
    seq: u32,
    predict: bool,
    pending: VecDeque<PendingIntent>,
    /// Outstanding reconciliation error, folded into the predicted position a
    /// little each Update tick by NetCorrectionSystem.
    correction: Vec3,
    /// Kept so a zone Redirect reconnects with the same latency knob.
    simulated_rtt: Duration,
}

impl NetClientState {
    fn own_entity(&self) -> Option<Entity> {
        self.own_id.and_then(|id| self.entities.get(&id).copied())
    }
}

/// The locally-controlled entity when playing online; None offline (the
/// sandbox finds its player by component instead).
pub(crate) fn own_entity(resources: &Resources) -> Option<Entity> {
    resources.get::<NetClientState>().and_then(|s| s.own_entity())
}

/// Interpolation between the last two snapshot positions (component on every
/// replicated entity except a predicted own player; drives Transform).
struct NetLerp {
    from: Vec3,
    to: Vec3,
    t: f32,
}

pub struct NetReceiveSystem;

impl System for NetReceiveSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let events = resources.get_mut::<NetClientState>().unwrap().client.poll();

        for event in events {
            match event {
                ClientEvent::Connected => {
                    // Identity first: the server spawns us and sends Welcome
                    // only after Login (loads the character's saved state).
                    let state = resources.get_mut::<NetClientState>().unwrap();
                    let name = state.user.clone();
                    state.client.send(encode(&ClientMsg::Login { name: name.clone() }));
                    log::info!("connected to server, logging in as '{name}'");
                }
                ClientEvent::Disconnected => log::warn!("disconnected from server"),
                ClientEvent::Message(data) => match decode::<ServerMsg>(&data) {
                    Some(ServerMsg::Welcome { player_id }) => {
                        log::info!("welcome: our player id is {player_id}");
                        let state = resources.get_mut::<NetClientState>().unwrap();
                        state.own_id = Some(player_id);
                        // A re-Welcome means death + respawn: the pending
                        // intents and correction belong to the old body.
                        state.pending.clear();
                        state.correction = Vec3::ZERO;
                    }
                    Some(ServerMsg::Snapshot { last_processed_seq, enters, leaves, states, .. }) => {
                        apply_snapshot(world, resources, last_processed_seq, enters, leaves, states);
                    }
                    Some(ServerMsg::MechanicScheduled {
                        telegraph_prefab, pos, radius, resolve_at_micros, duration_micros, ..
                    }) => {
                        spawn_telegraph(world, resources, &telegraph_prefab, pos, radius, resolve_at_micros, duration_micros);
                    }
                    Some(ServerMsg::HitResult { mechanic, hits }) => {
                        log::info!("mechanic {mechanic} hit {} entities", hits.len());
                    }
                    Some(ServerMsg::WorldClock { world_micros, at_server_micros }) => {
                        let wt = resources.get_mut::<WorldTime>().unwrap();
                        wt.offset_micros = world_micros as i64 - at_server_micros as i64;
                        wt.synced = true;
                    }
                    Some(ServerMsg::EntityDied { id, pos }) => {
                        handle_entity_died(world, resources, id, pos);
                    }
                    Some(ServerMsg::Redirect { zone, addr }) => {
                        // Zone transfer: WE close the old connection (dropping
                        // the NetClient) and start fresh at the new address.
                        // Remaining drained events belong to the old session.
                        handle_redirect(world, resources, &zone, addr);
                        break;
                    }
                    None => log::warn!("undecodable server message ({} bytes)", data.len()),
                },
            }
        }

        // Publish the synced clock for anything that schedules in server time.
        let offset = resources.get::<NetClientState>().unwrap().client.server_offset_micros();
        if let Some(offset) = offset {
            resources.get_mut::<Time>().unwrap().server_offset_micros = offset;
        }
    }
}

/// Tear down the old zone's replicated world and reconnect to the new one.
/// The fresh connection's Connected event re-triggers Login; the server
/// spawns us at the position the transfer (or login routing) persisted.
fn handle_redirect(world: &mut World, resources: &mut Resources, zone: &str, addr: SocketAddr) {
    log::info!("redirected to zone '{zone}' at {addr}");

    // Everything replicated belongs to the old zone, as do telegraph visuals
    // (their mechanics resolve in a world we just left).
    let telegraphs: Vec<Entity> = world.query::<(Entity, &TelegraphVisual)>().iter().map(|(e, _)| e).collect();
    let replicated: Vec<Entity> = resources.get::<NetClientState>().unwrap().entities.values().copied().collect();
    {
        let queue = resources.get_mut::<DespawnQueue>().unwrap();
        for entity in replicated.into_iter().chain(telegraphs) {
            queue.push(entity, None);
        }
    }

    let state = resources.get_mut::<NetClientState>().unwrap();
    state.entities.clear();
    state.own_id = None;
    state.pending.clear();
    state.correction = Vec3::ZERO;
    // Fresh connection, fresh validation stream (per-connection on the server).
    state.seq = 0;
    // Dropping the old NetClient closes the QUIC connection — the server sees
    // a normal Disconnected (which finds no PlayerConn and does nothing).
    state.client = NetClient::connect_with_latency(addr, PROTOCOL_VERSION, state.simulated_rtt)
        .unwrap_or_else(|e| panic!("failed to connect to zone '{zone}' at {addr}: {e}"));
    resources.get_mut::<WorldTime>().unwrap().synced = false;
    // ZoneDressingSystem rebuilds the floor/portals for the new zone.
    if let Some(current) = resources.get_mut::<crate::presentation::CurrentZone>() {
        current.0 = zone.to_owned();
    }
}

/// The server's death signal (v8): burst + cosmetic corpse for the dying
/// entity. Snapshots stop mentioning it the same tick, so its local entity is
/// despawned here too instead of waiting for the AOI leave. Our own death is
/// burst-only — the server re-Welcomes us into a respawned entity.
fn handle_entity_died(world: &mut World, resources: &mut Resources, id: u64, pos: Vec3) {
    let (entity, own) = {
        let state = resources.get_mut::<NetClientState>().unwrap();
        (state.entities.remove(&id), state.own_id == Some(id))
    };
    // Death burst at the server-authoritative position.
    let color = entity
        .and_then(|e| world.get::<&vordar_game::class::ClassId>(e).ok().map(|c| c.id.clone()))
        .map(|class| crate::vfx::class_tint(resources, &class))
        .unwrap_or(glam::Vec3::ONE);
    if let Some(sim) = resources.get_mut::<crate::vfx::ParticleSim>() {
        sim.burst(
            pos + Vec3::Y,
            color,
            crate::vfx::DEATH_COUNT,
            crate::vfx::DEATH_SPEED,
            crate::vfx::DEATH_SIZE,
        );
    }
    if own {
        return; // respawn arrives via re-Welcome; keep our entity
    }
    if let Some(entity) = entity {
        // Corpse for mesh characters, then remove the live entity.
        let corpse = {
            let transform = world.get::<&Transform>(entity).map(|t| Transform::clone(&t));
            let mesh = world
                .get::<&engine_core::components::RenderMesh>(entity)
                .map(|m| engine_core::components::RenderMesh::clone(&m));
            let clips = world
                .get::<&crate::locomotion::LocomotionClips>(entity)
                .map(|c| c.death.clone());
            match (transform, mesh, clips) {
                (Ok(t), Ok(m), Ok(death)) if !death.is_empty() => Some((t, m, death)),
                _ => None,
            }
        };
        if let Some((transform, mesh, death)) = corpse {
            crate::react::spawn_corpse(world, transform, mesh, &death);
        }
        resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
    }
}

fn apply_snapshot(
    world: &mut World,
    resources: &mut Resources,
    last_processed_seq: u32,
    enters: Vec<EntityState>,
    leaves: Vec<u64>,
    states: Vec<EntityPos>,
) {
    // Take the map instead of cloning it — nothing below reads it through
    // NetClientState, and it is written back at the end of this function.
    let (mut known, own_id, predict) = {
        let state = resources.get_mut::<NetClientState>().unwrap();
        (std::mem::take(&mut state.entities), state.own_id, state.predict)
    };

    // Enters first, so this snapshot's states can address the new entities.
    for enter in enters {
        if known.contains_key(&enter.id) {
            continue;
        }
        let is_own_predicted = predict && own_id == Some(enter.id);
        match spawn_prefab(&enter.prefab, enter.pos, &mut SpawnContext { world, resources }) {
            Ok(entity) => {
                // A predicted own player is moved by the simulation, not the lerp.
                if !is_own_predicted {
                    let _ = world.insert_one(entity, NetLerp { from: enter.pos, to: enter.pos, t: 1.0 });
                }
                // Seed replicated health (v8) so the hit-react watcher starts
                // from the server's value, not the prefab's.
                if let Ok(mut health) = world.get::<&mut Health>(entity) {
                    health.current = enter.hp;
                }
                known.insert(enter.id, entity);
            }
            Err(e) => log::error!("replicated spawn '{}' failed: {e}", enter.prefab),
        }
    }

    // Entities that left our AOI (or despawned on the server).
    for id in leaves {
        if let Some(entity) = known.remove(&id) {
            resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
        }
    }

    // Own-player state is handled by reconciliation, which needs &mut World —
    // pull it out before the view below borrows the world.
    let own_state = match (predict, own_id) {
        (true, Some(own)) => states.iter().find(|s| s.id == own).map(|s| (own, s.pos)),
        _ => None,
    };

    // Replicated health (v8) — every state, own player included: the client
    // never simulates its own damage, so the snapshot is the only source.
    {
        let mut hp_q = world.query::<&mut Health>();
        let mut hp_view = hp_q.view();
        for state in &states {
            let Some(&entity) = known.get(&state.id) else { continue };
            if let Some(health) = hp_view.get_mut(entity) {
                health.current = state.hp;
            }
        }
    }

    // Snapshot-derived velocity estimates for the lerped entities, so
    // locomotion/facing can animate remote characters (their sim Velocity is
    // never driven). Collected inside the view borrow, inserted after it.
    let mut net_motions: Vec<(Entity, Vec3)> = Vec::new();
    {
        // One view for the whole batch instead of two world.gets per entity.
        let mut lerp_q = world.query::<(&mut NetLerp, &Transform)>();
        let mut lerp_view = lerp_q.view();
        for state in &states {
            if own_state.is_some_and(|(own, _)| state.id == own) {
                continue;
            }
            let Some(&entity) = known.get(&state.id) else { continue };
            // Restart the lerp from wherever the entity is currently displayed.
            let Some((lerp, transform)) = lerp_view.get_mut(entity) else { continue };
            net_motions.push((entity, (state.pos - transform.position) * SNAPSHOT_HZ as f32));
            lerp.from = transform.position;
            lerp.to = state.pos;
            lerp.t = 0.0;
        }
    }
    for (entity, velocity) in net_motions {
        let _ = world.insert_one(entity, crate::locomotion::NetMotion { velocity });
    }

    if let Some((own, server_pos)) = own_state {
        if let Some(&entity) = known.get(&own) {
            reconcile_own(world, resources, entity, server_pos, last_processed_seq);
        }
    }

    resources.get_mut::<NetClientState>().unwrap().entities = known;
}

/// Rewind + replay: rebase our player onto the server's authoritative position
/// and re-apply every intent the server hasn't processed yet. The result is
/// where we SHOULD be — but movement stays optimistic: errors inside the trust
/// band are ignored, mid-size drift is handed to NetCorrectionSystem to blend
/// out, only real mispredictions (server-side corrections) snap.
fn reconcile_own(
    world: &mut World,
    resources: &mut Resources,
    entity: Entity,
    server_pos: Vec3,
    last_processed_seq: u32,
) {
    let speed = world.get::<&Player>(entity).map(|p| p.speed).unwrap_or(0.0);
    let replayed = {
        let state = resources.get_mut::<NetClientState>().unwrap();
        state.pending.retain(|p| p.seq > last_processed_seq);
        replay_position(server_pos, speed, state.pending.iter())
    };
    let Ok(mut transform) = world.get::<&mut Transform>(entity) else { return };
    let error = replayed - transform.position;
    let correction = match classify_error(error) {
        Correction::Trust => Vec3::ZERO,
        Correction::Smooth => {
            log::debug!("prediction drift: {:.3} units", error.length());
            error
        }
        Correction::Snap => {
            log::debug!("prediction snap: {:.2} units off", error.length());
            transform.position = replayed;
            Vec3::ZERO
        }
    };
    drop(transform);
    resources.get_mut::<NetClientState>().unwrap().correction = correction;
}

/// Position after replaying pending intents on top of the server's
/// authoritative position — the same movement rule the simulation runs.
fn replay_position<'a>(
    server_pos: Vec3,
    speed: f32,
    pending: impl Iterator<Item = &'a PendingIntent>,
) -> Vec3 {
    pending.fold(server_pos, |pos, p| pos + movement_velocity(p.dir, speed) * p.dt)
}

/// What to do about a reconciliation error.
#[derive(Debug, PartialEq)]
enum Correction {
    /// Inside the trust band — keep the predicted position untouched.
    Trust,
    /// Real drift — blend it out over time.
    Smooth,
    /// Way off — hard snap.
    Snap,
}

fn classify_error(error: Vec3) -> Correction {
    let d2 = error.length_squared();
    if d2 < TRUST_DISTANCE * TRUST_DISTANCE {
        Correction::Trust
    } else if d2 > SNAP_DISTANCE * SNAP_DISTANCE {
        Correction::Snap
    } else {
        Correction::Smooth
    }
}

/// Portion of the outstanding correction to apply after `dt` (exponential decay).
fn correction_step(correction: Vec3, dt: f32) -> Vec3 {
    correction * (1.0 - (-(std::f32::consts::LN_2 / CORRECTION_HALF_LIFE) * dt).exp())
}

/// Folds the outstanding reconciliation error into the predicted position a
/// little each fixed Update tick. Corrections applied here are rendered as
/// interpolated motion like any other movement; applying them where they are
/// detected (Phase::Input) pops instead, because SaveTransformSystem captures
/// PreviousTransform afterward and the offset is never interpolated.
pub struct NetCorrectionSystem;

impl System for NetCorrectionSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let (entity, step) = {
            let state = resources.get_mut::<NetClientState>().unwrap();
            if state.correction.length_squared() < 1e-8 {
                return;
            }
            let Some(entity) = state.own_entity() else { return };
            let step = correction_step(state.correction, delta);
            state.correction -= step;
            (entity, step)
        };
        if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
            transform.position += step;
        }
    }
}

/// World-clock mapping received from the server: world time = synced server
/// time + offset. World time drives day/night and world-event tint as pure
/// local functions (DESIGN.md §4).
pub struct WorldTime {
    offset_micros: i64,
    synced: bool,
}

/// Drives the light uniform from world time: the day/night cycle, overridden
/// by the active world event's tint. Pure function of the synced clock and
/// shared event defs — every client shows the same sky at the same instant,
/// including clients that joined mid-event.
pub struct DayNightSystem;

impl System for DayNightSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let world_now = {
            let wt = resources.get::<WorldTime>().unwrap();
            if !wt.synced {
                return;
            }
            let state = resources.get::<NetClientState>().unwrap();
            let Some(server_now) = state.client.server_now_micros() else { return };
            (server_now as i64 + wt.offset_micros).max(0) as u64
        };
        let world_seconds = world_now as f64 * 1e-6;

        let (dir, color, ambient) = match resources.get::<WorldEventsDef>() {
            Some(def) => match active_event(def, world_seconds) {
                Some(i) => {
                    // Event tint: keep the current sun angle, swap the mood.
                    let fraction = (world_seconds.rem_euclid(def.day_seconds) / def.day_seconds) as f32;
                    let (dir, _, _) = day_night_light(fraction);
                    (dir, def.events[i].ambient, 0.3)
                }
                None => {
                    let fraction = (world_seconds.rem_euclid(def.day_seconds) / def.day_seconds) as f32;
                    day_night_light(fraction)
                }
            },
            // No defs loaded: fall back to a fixed-length cycle.
            None => day_night_light((world_seconds.rem_euclid(120.0) / 120.0) as f32),
        };
        engine_renderer::set_light(dir, color, ambient, resources);
    }
}

/// A telegraph visual: counts down to the mechanic's resolve time. Purely
/// client-local — never in the replication map; despawns itself at T.
struct TelegraphVisual {
    resolve_at_micros: u64,
    duration_micros: u64,
}

const TELEGRAPH_DIM: Vec3 = Vec3::new(0.45, 0.08, 0.08);
// Components above 1.0 are HDR emissive (VQ-C3): an about-to-resolve
// telegraph blooms threat red-orange (VQ-A4).
const TELEGRAPH_BRIGHT: Vec3 = Vec3::new(2.2, 0.45, 0.15);

fn spawn_telegraph(
    world: &mut World,
    resources: &mut Resources,
    prefab: &str,
    pos: Vec3,
    radius: f32,
    resolve_at_micros: u64,
    duration_micros: u64,
) {
    match spawn_prefab(prefab, pos, &mut SpawnContext { world, resources }) {
        Ok(entity) => {
            if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
                transform.scale = Vec3::new(radius * 2.0, 0.1, radius * 2.0);
            }
            let _ = world.insert_one(entity, TelegraphVisual { resolve_at_micros, duration_micros });
        }
        Err(e) => log::error!("telegraph spawn '{prefab}' failed: {e}"),
    }
}

/// Animates telegraph fill as a PURE FUNCTION of synced server time vs
/// resolve_at (DESIGN.md §3) — zero per-frame network updates, and the visual
/// completes exactly at T (the hit-test moment) on every client. Runs once
/// per display frame so the fill is smooth at any refresh rate.
pub struct TelegraphFillSystem;

impl System for TelegraphFillSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let Some(now) = resources.get::<NetClientState>().unwrap().client.server_now_micros() else {
            return;
        };
        let mut finished: Vec<Entity> = Vec::new();
        for (entity, telegraph, shape) in world.query::<(Entity, &TelegraphVisual, &mut RenderShape)>().iter() {
            if now >= telegraph.resolve_at_micros {
                finished.push(entity);
                continue;
            }
            let remaining = (telegraph.resolve_at_micros - now) as f32;
            let fill = 1.0 - (remaining / telegraph.duration_micros as f32).clamp(0.0, 1.0);
            shape.color = TELEGRAPH_DIM.lerp(TELEGRAPH_BRIGHT, fill);
        }
        // The scheduled-ability impact beat (VQ-E1/E4): the resolve moment
        // pops threat-colored sparks + ground dust where the telegraph was.
        let impact_positions: Vec<Vec3> = finished
            .iter()
            .filter_map(|&e| world.get::<&Transform>(e).ok().map(|t| t.position))
            .collect();
        if let Some(sim) = resources.get_mut::<crate::vfx::ParticleSim>() {
            for pos in impact_positions {
                sim.burst_def(pos, &vordar_game::vfx::BurstDef {
                    count: 20,
                    speed: 4.5,
                    size:  0.13,
                    color: TELEGRAPH_BRIGHT,
                    cell:  1,
                    blend: vordar_game::vfx::ParticleBlend::Additive,
                    ttl: (0.25, 0.5),
                    gravity: -7.0,
                    drag: 2.5,
                    stretch: 0.0,
                });
                sim.burst_def(pos, &vordar_game::vfx::BurstDef {
                    count: 8,
                    speed: 1.6,
                    size:  0.35,
                    color: Vec3::new(0.35, 0.28, 0.24),
                    cell:  3,
                    blend: vordar_game::vfx::ParticleBlend::Alpha,
                    ttl: (0.6, 1.0),
                    gravity: -1.0,
                    drag: 1.5,
                    stretch: 0.0,
                });
            }
        }
        for entity in finished {
            resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
        }
    }
}

/// Keys for the edge-triggered ability slots (slot 1, slot 2). Slot 0 is the
/// LMB held-repeat attack.
const SLOT_KEYS: [winit::keyboard::KeyCode; 2] =
    [winit::keyboard::KeyCode::KeyQ, winit::keyboard::KeyCode::KeyE];

/// Casts the local class's abilities at the cursor's ground point: slot 0
/// auto-fires while LMB is held (at the cooldown rate), later slots are
/// edge-triggered keys (Q, E). Targets for ranged-capped effects are clamped
/// so an honest cast is never rejected. The client gate is display/traffic
/// hygiene — the server re-validates class, cooldown, and range.
pub struct AbilityCastSystem {
    /// Edge state per keyed slot.
    was_down: [bool; SLOT_KEYS.len()],
}

impl AbilityCastSystem {
    pub fn new() -> Self {
        Self { was_down: [false; SLOT_KEYS.len()] }
    }
}

impl System for AbilityCastSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        /// Slot metadata for the local class.
        struct SlotMeta {
            id: String,
            /// Range clamp for targeted effects.
            range: Option<f32>,
            cooldown_secs: f32,
            /// Leap cast time if it's a dash (drives the optimistic impulse).
            leap_micros: Option<u64>,
            /// Per-ability cast animation (cosmetic).
            anim: Option<String>,
            anim_secs: Option<f32>,
        }
        let Some(class) = crate::local_class(world, resources) else { return };
        let slots: Vec<SlotMeta> = {
            let Some(library) = resources.get::<vordar_game::class::ClassLibrary>() else { return };
            library
                .abilities_of(&class)
                .iter()
                .map(|a| {
                    let (range, leap_micros) = match &a.effect {
                        vordar_game::skills::AbilityEffect::Scheduled { max_range, .. } => (Some(*max_range), None),
                        vordar_game::skills::AbilityEffect::Projectile { .. } => (None, None),
                        vordar_game::skills::AbilityEffect::Leap { max_range, cast_micros, .. } => {
                            (Some(*max_range), Some(*cast_micros))
                        }
                    };
                    SlotMeta {
                        id: a.id.clone(),
                        range,
                        cooldown_secs: a.cooldown_micros as f32 / 1e6,
                        leap_micros,
                        anim: a.anim.clone(),
                        anim_secs: a.anim_secs,
                    }
                })
                .collect()
        };
        {
            let cooldowns: Vec<f32> = slots.iter().map(|s| s.cooldown_secs).collect();
            let cast = resources.get_mut::<crate::CastState>().unwrap();
            cast.sync(&class, &cooldowns);
            cast.tick(delta);
        }

        let mut triggered: Vec<usize> = Vec::new();
        let lmb = resources
            .get::<engine_app::input::MouseState>()
            .map(|m| m.is_pressed(winit::event::MouseButton::Left))
            .unwrap_or(false);
        if lmb {
            triggered.push(0);
        }
        for (i, key) in SLOT_KEYS.iter().enumerate() {
            let down = resources
                .get::<engine_app::input::KeyboardState>()
                .map(|kb| kb.is_pressed(*key))
                .unwrap_or(false);
            if down && !self.was_down[i] {
                triggered.push(i + 1);
            }
            self.was_down[i] = down;
        }
        triggered.retain(|&s| {
            s < slots.len() && resources.get::<crate::CastState>().map(|c| c.ready(s)).unwrap_or(false)
        });
        if triggered.is_empty() {
            return;
        }

        let Some(cursor) = resources.get::<engine_app::input::MouseState>().and_then(|m| m.cursor()) else {
            return;
        };
        let Some(ground) = engine_renderer::screen_to_ground(cursor, resources) else { return };
        let Some(origin) = own_entity(resources)
            .and_then(|e| world.get::<&Transform>(e).ok().map(|t| t.position))
        else {
            return;
        };

        for slot in triggered {
            let SlotMeta { id, range, leap_micros, anim, anim_secs, .. } = &slots[slot];
            let from = Vec2::new(origin.x, origin.z);
            let mut target = Vec2::new(ground.x, ground.z);
            if let Some(max_range) = range {
                let offset = target - from;
                if offset.length() > *max_range {
                    target = from + offset.normalize() * *max_range;
                }
            }
            let (own, predict) = {
                let state = resources.get_mut::<NetClientState>().unwrap();
                let Some(t_server_micros) = state.client.server_now_micros() else { return };
                state.seq += 1;
                state.client.send(encode(&ClientMsg::CastIntent {
                    seq: state.seq,
                    t_server_micros,
                    skill: id.clone(),
                    target,
                }));
                (state.own_entity(), state.predict)
            };
            resources.get_mut::<crate::CastState>().unwrap().fire(slot);
            if let Some(entity) = own {
                crate::pose::trigger_swing(world, entity);
                // Skinned-mesh cast animation (per-ability clip) — no-op if not animated.
                crate::locomotion::trigger_attack_clip(world, entity, anim.as_deref(), *anim_secs);
                let tint = crate::vfx::class_tint(resources, &class);
                crate::vfx::cast_burst(world, resources, entity, id, tint);
            }
            // Optimistic dash: same deterministic velocity math the server
            // runs, so reconciliation only ever sees ordinary drift. Rare
            // server-side rejects surface as a correction snap.
            if let (Some(cast_micros), Some(entity), true) = (leap_micros, own, predict) {
                let cast_secs = *cast_micros as f32 / 1e6;
                let to = Vec3::new(target.x, 0.0, target.y);
                let _ = world.insert_one(entity, vordar_game::combat::LeapImpulse {
                    velocity: vordar_game::combat::leap::leap_velocity(origin, to, cast_secs),
                    remaining: cast_secs,
                });
            }
        }
    }
}

/// Sends our movement intent each Input tick, stamped with synced server time.
/// Nothing is sent until the clock sync has at least one sample. When
/// predicting, the intent is also emitted locally for the shared movement
/// system and remembered for reconciliation replay.
pub struct NetSendInputSystem;

impl System for NetSendInputSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, delta: f32) {
        let dir = read_move_dir(resources);
        let predicted_entity = {
            let state = resources.get_mut::<NetClientState>().unwrap();
            let Some(t_server_micros) = state.client.server_now_micros() else { return };
            state.seq += 1;
            state.client.send(encode(&ClientMsg::MoveIntent { seq: state.seq, t_server_micros, dir }));

            let entity = if state.predict { state.own_entity() } else { None };
            if entity.is_some() {
                state.pending.push_back(PendingIntent { seq: state.seq, dir, dt: delta });
                if state.pending.len() > MAX_PENDING_INTENTS {
                    state.pending.pop_front();
                }
            }
            entity
        };
        if let Some(entity) = predicted_entity {
            let bus = resources.get_mut::<EventBus>().expect("EventBus not in resources");
            bus.emit(MoveIntent { entity, dir });
        }
    }
}

/// Advances every replicated entity toward its latest snapshot position over
/// one snapshot interval.
pub struct NetLerpSystem;

impl System for NetLerpSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, delta: f32) {
        for (lerp, transform) in world.query::<(&mut NetLerp, &mut Transform)>().iter() {
            lerp.t = (lerp.t + delta * SNAPSHOT_HZ).min(1.0);
            transform.position = lerp.from.lerp(lerp.to, lerp.t);
        }
    }
}

/// Follows our own player (identified by the Welcome message) at its
/// interpolated render position. Runs Phase::RenderSync — see
/// CameraFollowSystem for why the camera must move at render cadence.
pub struct NetCameraFollowSystem;

impl System for NetCameraFollowSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let target = {
            let state = resources.get::<NetClientState>().unwrap();
            state.own_entity().and_then(|e| crate::render_position(world, e, resources))
        };
        orbit_and_follow(target, resources, delta);
    }
}

/// Benchmark seam (vordar-benches only): exposes the private snapshot-apply /
/// reconciliation machinery so the client hot path is measurable headless.
/// The NetClientState's socket points at an unroutable address — nothing the
/// benches call ever touches the network.
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod bench {
    use super::*;

    /// NetClientState with no live connection: the net thread's connect
    /// attempt fails in the background while the benched paths only read
    /// and write the state fields.
    pub fn state_for_bench(own_id: Option<u64>, predict: bool) -> NetClientState {
        NetClientState {
            client: NetClient::connect("127.0.0.1:9".parse().unwrap(), PROTOCOL_VERSION)
                .expect("bench NetClient"),
            user: "bench".into(),
            own_id,
            entities: HashMap::new(),
            seq: 0,
            predict,
            pending: VecDeque::new(),
            correction: Vec3::ZERO,
            simulated_rtt: Duration::ZERO,
        }
    }

    /// server-id → local-entity mapping (the enters path builds this normally).
    pub fn map_entity(state: &mut NetClientState, id: u64, entity: Entity) {
        state.entities.insert(id, entity);
    }

    pub fn push_pending(state: &mut NetClientState, seq: u32, dir: Vec2, dt: f32) {
        state.pending.push_back(PendingIntent { seq, dir, dt });
    }

    pub fn apply_snapshot(
        world: &mut World,
        resources: &mut Resources,
        last_processed_seq: u32,
        enters: Vec<EntityState>,
        leaves: Vec<u64>,
        states: Vec<EntityPos>,
    ) {
        super::apply_snapshot(world, resources, last_processed_seq, enters, leaves, states);
    }

    pub fn reconcile_own(
        world: &mut World,
        resources: &mut Resources,
        entity: Entity,
        server_pos: Vec3,
        last_processed_seq: u32,
    ) {
        super::reconcile_own(world, resources, entity, server_pos, last_processed_seq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn intent(seq: u32, dir: Vec2) -> PendingIntent {
        PendingIntent { seq, dir, dt: DT }
    }

    #[test]
    fn replay_applies_unacked_intents() {
        let pending = vec![intent(1, Vec2::new(1.0, 0.0)), intent(2, Vec2::new(1.0, 0.0))];
        let pos = replay_position(Vec3::ZERO, 6.0, pending.iter());
        assert!((pos.x - 2.0 * 6.0 * DT).abs() < 1e-6);
        assert_eq!(pos.y, 0.0);
        assert_eq!(pos.z, 0.0);
    }

    #[test]
    fn replay_normalizes_direction_like_the_simulation() {
        // An over-unit dir must move exactly as fast as a unit dir.
        let cheat = vec![intent(1, Vec2::new(30.0, 40.0))];
        let fair = vec![intent(1, Vec2::new(0.6, 0.8))];
        let a = replay_position(Vec3::ZERO, 6.0, cheat.iter());
        let b = replay_position(Vec3::ZERO, 6.0, fair.iter());
        assert!((a - b).length() < 1e-6);
    }

    #[test]
    fn error_classification_bands() {
        // Optimistic movement: jitter-scale disagreement never tugs the player.
        assert_eq!(classify_error(Vec3::new(0.2, 0.0, 0.0)), Correction::Trust);
        assert_eq!(classify_error(Vec3::new(0.5, 0.0, 0.0)), Correction::Smooth);
        assert_eq!(classify_error(Vec3::new(2.0, 0.0, 0.0)), Correction::Snap);
    }

    #[test]
    fn correction_decays_smoothly_to_zero() {
        let mut remaining = Vec3::new(0.9, 0.0, 0.0);
        let mut largest_step = 0.0f32;
        for _ in 0..120 {
            let step = correction_step(remaining, DT);
            largest_step = largest_step.max(step.length());
            remaining -= step;
        }
        assert!(remaining.length() < 1e-3, "did not converge: {remaining}");
        // Every nudge stays below one tick of run-speed movement — corrections
        // must read as motion, not teleports.
        assert!(largest_step < 6.0 * DT, "step too large: {largest_step}");
    }
}
