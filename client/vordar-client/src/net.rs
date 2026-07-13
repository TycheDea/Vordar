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
use std::time::{Duration, Instant};
use vordar_game::events::MoveIntent;
use vordar_game::motion::MovementSystem;
use vordar_game::player::{movement_velocity, PlayerMovementSystem};
use vordar_game::Player;
use vordar_game::world::{active_event, day_night_light, WorldEventsDef};
use vordar_protocol::{
    decode, encode, AccountToken, ClientMsg, EntityPos, EntityState, ServerMsg, PROTOCOL_VERSION, SNAPSHOT_HZ,
};

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

/// Initial wait before the first redial after an unexpected disconnect — an
/// ordinary blip (brief loss, a moment of server-side hiccup) clears fast.
const RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
/// Backoff cap so a genuinely dead server doesn't spin the network thread,
/// while still retrying at a steady cadence (networking audit 2026-07-11,
/// finding 7: disconnect used to be a log line with no recovery at all).
const RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(8);
/// How long a redial is given to resolve (Connected or Disconnected) before
/// the backoff timer is allowed to fire again — must clear engine-net's own
/// handshake timeout (`client::HANDSHAKE_TIMEOUT`, 5 s) with margin.
const RECONNECT_ATTEMPT_GRACE: Duration = Duration::from_secs(6);

/// Backoff before reconnect attempt `attempt` (1-indexed): doubles each
/// attempt, capped at `RECONNECT_MAX_BACKOFF`.
fn reconnect_backoff(attempt: u32) -> Duration {
    let doublings = attempt.saturating_sub(1).min(8);
    RECONNECT_INITIAL_BACKOFF.saturating_mul(1u32 << doublings).min(RECONNECT_MAX_BACKOFF)
}

/// Reconnect-in-progress bookkeeping: which attempt is current, and when to
/// act next — either "redial now" (waiting out the backoff) or "give up
/// waiting on the in-flight redial and reconsider" (`RECONNECT_ATTEMPT_GRACE`
/// after issuing it). `Some` for as long as the connection is down; cleared
/// the moment `ClientEvent::Connected` fires again.
struct Reconnect {
    attempt: u32,
    retry_at: Instant,
}

pub struct NetClientPlugin {
    pub server_addr: SocketAddr,
    /// Predict own movement locally; off reproduces the Phase 1 server-driven
    /// feel (one round-trip of input latency) for comparison.
    pub predict: bool,
    /// Artificial round-trip latency added by engine-net (testing knob).
    pub simulated_rtt: Duration,
    /// Character name sent as the first message after connect.
    pub user: String,
    /// Account credential presented with `user` on every `Login` (networking
    /// rework 1, finding 3) — see `credentials::load_or_mint`.
    pub token: AccountToken,
}

impl Plugin for NetClientPlugin {
    fn build(&self, app: &mut App) {
        // A failed first connect no longer panics (networking audit
        // 2026-07-11, finding 7): fall back to the same reconnect state
        // machine that handles a later drop, so a transient failure (server
        // not up yet, brief DNS/route hiccup) resolves in the background
        // instead of crashing the client before a single frame renders.
        let (client, reconnect) =
            match NetClient::connect_with_latency(self.server_addr, PROTOCOL_VERSION, self.simulated_rtt) {
                Ok(client) => (Some(client), None),
                Err(e) => {
                    log::error!("net: failed to start network client: {e} — retrying in the background");
                    (None, Some(Reconnect { attempt: 1, retry_at: Instant::now() + reconnect_backoff(1) }))
                }
            };
        app.insert_resource(NetClientState {
            client,
            server_addr: self.server_addr,
            reconnect,
            user: self.user.clone(),
            token: self.token,
            login_denied: false,
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
        // Weapons glue to the freshly rebuilt hand sockets (same slot as VFX).
        .add_system(crate::weapons::WeaponAttachSystem::default(), Phase::RenderSync, SystemOrder::after::<engine_renderer::MeshRenderSyncSystem>())
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
/// Replayed on top of each snapshot of our own player. `leap` mirrors a
/// LeapImpulse active on the entity when this tick's intent was recorded —
/// replay reproduces the dash's straight-line displacement instead of
/// dead-reckoning plain WASD movement through it (networking audit
/// 2026-07-11, finding 11).
struct PendingIntent {
    seq: u32,
    dir: Vec2,
    dt: f32,
    leap: Option<Vec3>,
}

pub struct NetClientState {
    /// None while the connection is down (initial connect failure, or an
    /// unexpected drop awaiting a redial) — networking audit 2026-07-11,
    /// finding 7.
    client: Option<NetClient>,
    /// Address to redial after an unexpected disconnect. A zone Redirect
    /// overwrites this with the new zone's address.
    server_addr: SocketAddr,
    user: String,
    /// Account credential presented on every `Login` (networking rework 1,
    /// finding 3).
    token: AccountToken,
    /// Set once a `LoginDenied` arrives — stops `handle_disconnected` and
    /// `maybe_reconnect` from scheduling further redials: retrying with the
    /// same bad credential would only be denied again.
    login_denied: bool,
    own_id: Option<u32>,
    /// server entity id → local entity
    entities: HashMap<u32, Entity>,
    seq: u32,
    predict: bool,
    pending: VecDeque<PendingIntent>,
    /// Outstanding reconciliation error, folded into the predicted position a
    /// little each Update tick by NetCorrectionSystem.
    correction: Vec3,
    /// Kept so a zone Redirect (or a reconnect) redials with the same
    /// latency knob.
    simulated_rtt: Duration,
    /// Set while disconnected and a redial is scheduled/in flight; read by
    /// the UI to show a "reconnecting" indicator (`reconnect_attempt`).
    reconnect: Option<Reconnect>,
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

/// Current reconnect attempt number, for the UI banner. None while connected
/// (or offline — no NetClientState at all): networking audit 2026-07-11,
/// finding 7.
pub(crate) fn reconnect_attempt(resources: &Resources) -> Option<u32> {
    resources.get::<NetClientState>().and_then(|s| s.reconnect.as_ref().map(|r| r.attempt))
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
        // A due redial happens on its own clock, independent of any event
        // arriving this tick (networking audit 2026-07-11, finding 7).
        maybe_reconnect(resources);

        let events = {
            let state = resources.get_mut::<NetClientState>().unwrap();
            state.client.as_mut().map(|c| c.poll()).unwrap_or_default()
        };

        for event in events {
            match event {
                ClientEvent::Connected => {
                    // Identity first: the server spawns us and sends Welcome
                    // only after Login (loads the character's saved state).
                    let state = resources.get_mut::<NetClientState>().unwrap();
                    state.reconnect = None;
                    let name = state.user.clone();
                    let token = state.token;
                    if let Some(client) = &state.client {
                        client.send(encode(&ClientMsg::Login { name: name.clone(), token }));
                    }
                    log::info!("connected to server, logging in as '{name}'");
                }
                ClientEvent::Disconnected => handle_disconnected(world, resources),
                // A hard handshake rejection (e.g. version mismatch) — log it
                // distinctly from an ordinary drop; `Disconnected` still
                // follows and drives the existing teardown/reconnect path.
                ClientEvent::Rejected(reason) => log::error!("connection rejected by server: {reason}"),
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
                    Some(ServerMsg::LoginDenied { reason }) => {
                        // Denials are messages, not kicks (networking rework
                        // 1, finding 3): the server leaves the connection
                        // open, so WE close it — same lesson as Redirect and
                        // the Phase-6 takeover, a server-side kick could
                        // outrace this frame. `login_denied` then stops
                        // `handle_disconnected` from scheduling a redial that
                        // would only be denied again with the same credential.
                        log::error!("login denied: {reason:?}");
                        resources.get_mut::<NetClientState>().unwrap().login_denied = true;
                        handle_disconnected(world, resources);
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
        let offset = resources.get::<NetClientState>().unwrap().client.as_ref().and_then(|c| c.server_offset_micros());
        if let Some(offset) = offset {
            resources.get_mut::<Time>().unwrap().server_offset_micros = offset;
        }
    }
}

/// Despawns every replicated entity and telegraph visual and resets the
/// per-connection reconciliation state. Shared by a zone Redirect and an
/// unexpected disconnect (networking audit 2026-07-11, finding 7): both leave
/// the client needing a fresh AOI rebuild off the next Welcome.
fn teardown_replicated_world(world: &mut World, resources: &mut Resources) {
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
    resources.get_mut::<WorldTime>().unwrap().synced = false;
}

/// Tear down the old zone's replicated world and reconnect to the new one.
/// The fresh connection's Connected event re-triggers Login; the server
/// spawns us at the position the transfer (or login routing) persisted.
fn handle_redirect(world: &mut World, resources: &mut Resources, zone: &str, addr: SocketAddr) {
    log::info!("redirected to zone '{zone}' at {addr}");
    teardown_replicated_world(world, resources);

    let state = resources.get_mut::<NetClientState>().unwrap();
    state.server_addr = addr;
    // Any in-flight backoff belonged to the old zone's address.
    state.reconnect = None;
    // Dropping the old NetClient closes the QUIC connection — the server sees
    // a normal Disconnected (which finds no PlayerConn and does nothing). A
    // failed redial here no longer panics: it falls into the same reconnect
    // state machine an unexpected drop uses (networking audit 2026-07-11,
    // finding 7) instead of crashing with the character already persisted
    // into the target zone.
    match NetClient::connect_with_latency(addr, PROTOCOL_VERSION, state.simulated_rtt) {
        Ok(client) => state.client = Some(client),
        Err(e) => {
            log::error!("net: failed to connect to zone '{zone}' at {addr}: {e} — retrying in the background");
            state.client = None;
            state.reconnect = Some(Reconnect { attempt: 1, retry_at: Instant::now() + reconnect_backoff(1) });
        }
    }
    // ZoneDressingSystem rebuilds the floor/portals for the new zone.
    if let Some(current) = resources.get_mut::<crate::presentation::CurrentZone>() {
        current.0 = zone.to_owned();
    }
}

/// An unexpected disconnect (server killed, brief network loss, redial
/// failure): tear down the replicated world exactly like a zone Redirect,
/// then schedule (or advance) a backoff-retried redial of the same address.
/// Networking audit 2026-07-11, finding 7 — this used to be a bare log line
/// with no recovery.
fn handle_disconnected(world: &mut World, resources: &mut Resources) {
    teardown_replicated_world(world, resources);
    let state = resources.get_mut::<NetClientState>().unwrap();
    state.client = None;
    if state.login_denied {
        log::warn!("net: not reconnecting — the last login was denied");
        state.reconnect = None;
        return;
    }
    let attempt = state.reconnect.as_ref().map_or(1, |r| r.attempt + 1);
    let backoff = reconnect_backoff(attempt);
    log::warn!("net: disconnected from server — reconnect attempt {attempt} in {backoff:?}");
    state.reconnect = Some(Reconnect { attempt, retry_at: Instant::now() + backoff });
}

/// Redials `state.server_addr` once the current backoff/grace window has
/// elapsed. Runs every Input tick regardless of which events (if any) were
/// just drained — a due retry has nothing to do with the last message
/// received. Networking audit 2026-07-11, finding 7.
fn maybe_reconnect(resources: &mut Resources) {
    let state = resources.get_mut::<NetClientState>().unwrap();
    if state.login_denied {
        return;
    }
    let Some(reconnect) = &state.reconnect else { return };
    if Instant::now() < reconnect.retry_at {
        return;
    }
    let attempt = reconnect.attempt;
    let addr = state.server_addr;
    let simulated_rtt = state.simulated_rtt;
    match NetClient::connect_with_latency(addr, PROTOCOL_VERSION, simulated_rtt) {
        Ok(client) => {
            log::info!("net: reconnect attempt {attempt} dialing {addr}");
            state.client = Some(client);
            // Give this attempt a chance to resolve (Connected clears
            // `reconnect`; Disconnected reschedules with the real backoff)
            // before the timer is allowed to fire again.
            state.reconnect = Some(Reconnect { attempt, retry_at: Instant::now() + RECONNECT_ATTEMPT_GRACE });
        }
        Err(e) => {
            let next = attempt + 1;
            log::warn!("net: reconnect attempt {attempt} failed to start: {e}");
            state.reconnect = Some(Reconnect { attempt: next, retry_at: Instant::now() + reconnect_backoff(next) });
        }
    }
}

/// The server's death signal (v8): burst + cosmetic corpse for the dying
/// entity. Snapshots stop mentioning it the same tick, so its local entity is
/// despawned here too instead of waiting for the AOI leave. Our own death is
/// burst-only — the server re-Welcomes us into a respawned entity.
fn handle_entity_died(world: &mut World, resources: &mut Resources, id: u32, pos: Vec3) {
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
    leaves: Vec<u32>,
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
    let (replayed, still_reconciling_a_dash) = {
        let state = resources.get_mut::<NetClientState>().unwrap();
        state.pending.retain(|p| p.seq > last_processed_seq);
        // Not just "is the local LeapImpulse still active": the server mirrors
        // the same cast only after its own one-way network delay, so its copy
        // of the dash finishes strictly later than the local one, and the
        // MoveIntent queue it drains at one-per-tick can lag further behind
        // still. Any unacked intent recorded during the dash means the
        // server hasn't caught up on the dash yet.
        let still_reconciling_a_dash = state.pending.iter().any(|p| p.leap.is_some());
        (replay_position(server_pos, speed, state.pending.iter()), still_reconciling_a_dash)
    };
    // Collision response isn't replayed (finding 11 — full collision-in-replay
    // is rework-scale, `reworks-networking-2026-07-11.md` finding 7): mid-dash
    // the free-flight `replayed` position and a wall-clamped real one can
    // differ for reasons that aren't mispredictions, so corrections stay
    // suppressed until the server has caught up on the whole dash instead of
    // tugging every snapshot.
    if still_reconciling_a_dash {
        return;
    }
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
/// authoritative position — the same movement rule the simulation runs,
/// including a leap override where one was active (finding 11) — collision
/// response is the one part of the shared rule still unreplayed (rework-scale,
/// `reworks-networking-2026-07-11.md` finding 7).
fn replay_position<'a>(
    server_pos: Vec3,
    speed: f32,
    pending: impl Iterator<Item = &'a PendingIntent>,
) -> Vec3 {
    pending.fold(server_pos, |pos, p| {
        let velocity = p.leap.unwrap_or_else(|| movement_velocity(p.dir, speed));
        pos + velocity * p.dt
    })
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
            let Some(server_now) = state.client.as_ref().and_then(|c| c.server_now_micros()) else { return };
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
        let Some(now) = resources.get::<NetClientState>().unwrap().client.as_ref().and_then(|c| c.server_now_micros()) else {
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
                let Some(t_server_micros) = state.client.as_ref().and_then(|c| c.server_now_micros()) else {
                    return;
                };
                state.seq += 1;
                if let Some(client) = &state.client {
                    client.send(encode(&ClientMsg::CastIntent {
                        seq: state.seq,
                        t_server_micros,
                        skill: id.clone(),
                        target,
                    }));
                }
                (state.own_entity(), state.predict)
            };
            resources.get_mut::<crate::CastState>().unwrap().fire(slot);
            if let Some(entity) = own {
                crate::pose::trigger_swing(world, entity);
                // Skinned-mesh cast animation (per-ability clip) — no-op if not animated.
                crate::locomotion::trigger_attack_clip(world, entity, anim.as_deref(), *anim_secs);
                // Turn toward the cast target (cosmetic, works while standing).
                crate::locomotion::aim_at(world, entity, Vec3::new(target.x, 0.0, target.y));
                let tint = crate::vfx::class_tint(resources, &class);
                crate::vfx::cast_burst(world, resources, entity, id, tint);
            }
            // Optimistic dash: same deterministic velocity math the server
            // runs, so reconciliation only ever sees ordinary drift. Rare
            // server-side rejects surface as a correction snap.
            if let (Some(cast_micros), Some(entity), true) = (leap_micros, own, predict) {
                let cast_secs = *cast_micros as f32 / 1e6;
                let to = Vec3::new(target.x, 0.0, target.y);
                let velocity = vordar_game::combat::leap::leap_velocity(origin, to, cast_secs);
                start_predicted_leap(world, resources, entity, velocity, cast_secs);
            }
        }
    }
}

/// Inserts the client-predicted LeapImpulse for a dash cast and retags this
/// tick's already-recorded PendingIntent (NetSendInputSystem runs earlier in
/// the same Input phase, before the dash existed) so replay reproduces the
/// dash from its very first tick too, not just the ticks after — networking
/// audit 2026-07-11, finding 11.
fn start_predicted_leap(world: &mut World, resources: &mut Resources, entity: Entity, velocity: Vec3, cast_secs: f32) {
    let _ = world.insert_one(entity, vordar_game::combat::LeapImpulse { velocity, remaining: cast_secs });
    if let Some(state) = resources.get_mut::<NetClientState>() {
        if let Some(pending) = state.pending.back_mut() {
            pending.leap = Some(velocity);
        }
    }
}

/// Sends our movement intent each Input tick, stamped with synced server time.
/// Nothing is sent until the clock sync has at least one sample. When
/// predicting, the intent is also emitted locally for the shared movement
/// system and remembered for reconciliation replay.
pub struct NetSendInputSystem;

impl System for NetSendInputSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let dir = read_move_dir(resources);
        let predicted_entity = {
            let state = resources.get_mut::<NetClientState>().unwrap();
            let Some(t_server_micros) = state.client.as_ref().and_then(|c| c.server_now_micros()) else {
                return;
            };
            state.seq += 1;
            if let Some(client) = &state.client {
                client.send(encode(&ClientMsg::MoveIntent { seq: state.seq, t_server_micros, dir }));
            }

            let entity = if state.predict { state.own_entity() } else { None };
            if let Some(entity) = entity {
                // A LeapImpulse already on the entity when this tick's intent
                // is recorded means the Update-phase LeapSystem (later this
                // same tick) will override this tick's velocity too — mirror
                // that into the pending record so replay reconstructs the
                // dash instead of dead-reckoning plain movement (networking
                // audit 2026-07-11, finding 11). A dash that starts THIS tick
                // is retagged onto this same entry by `start_predicted_leap`,
                // called later in this same Input phase.
                let leap = world.get::<&vordar_game::combat::leap::LeapImpulse>(entity).ok().map(|l| l.velocity);
                state.pending.push_back(PendingIntent { seq: state.seq, dir, dt: delta, leap });
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
    pub fn state_for_bench(own_id: Option<u32>, predict: bool) -> NetClientState {
        let server_addr = "127.0.0.1:9".parse().unwrap();
        NetClientState {
            client: Some(
                NetClient::connect(server_addr, PROTOCOL_VERSION).expect("bench NetClient"),
            ),
            server_addr,
            user: "bench".into(),
            token: [0u8; 32],
            login_denied: false,
            own_id,
            entities: HashMap::new(),
            seq: 0,
            predict,
            pending: VecDeque::new(),
            correction: Vec3::ZERO,
            simulated_rtt: Duration::ZERO,
            reconnect: None,
        }
    }

    /// server-id → local-entity mapping (the enters path builds this normally).
    pub fn map_entity(state: &mut NetClientState, id: u32, entity: Entity) {
        state.entities.insert(id, entity);
    }

    pub fn push_pending(state: &mut NetClientState, seq: u32, dir: Vec2, dt: f32) {
        state.pending.push_back(PendingIntent { seq, dir, dt, leap: None });
    }

    pub fn apply_snapshot(
        world: &mut World,
        resources: &mut Resources,
        last_processed_seq: u32,
        enters: Vec<EntityState>,
        leaves: Vec<u32>,
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
        PendingIntent { seq, dir, dt: DT, leap: None }
    }

    /// Deterministic token for `name` (mirrors `tests/common/mod.rs`'s
    /// `name_token` in vordar-server): name bytes zero-padded/truncated into
    /// the 32-byte account token, so a victim and a same-name kicker in the
    /// same test always agree on a token without either hardcoding the
    /// other's literal.
    fn name_token(name: &str) -> AccountToken {
        let mut token = [0u8; 32];
        let bytes = name.as_bytes();
        let n = bytes.len().min(32);
        token[..n].copy_from_slice(&bytes[..n]);
        token
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

    /// Networking audit 2026-07-11, finding 11: at 150 ms RTT an Onslaught
    /// (cast_secs 0.4 s, content/classes/ravager.ron) replays ~9 unacked
    /// intents while the dash is in flight. Folding plain WASD movement
    /// through them (the pre-fix behaviour, `leap: None` throughout) misses
    /// the dash's real displacement by more than SNAP_DISTANCE — exactly the
    /// mid-dash teleport the finding describes. Leap-aware replay
    /// (`leap: Some(velocity)`) must instead land exactly where the dash
    /// actually went.
    #[test]
    fn replay_reconstructs_a_dash_leap_instead_of_dead_reckoning_wasd() {
        let dash_velocity = Vec3::new(30.0, 0.0, 0.0); // 12 units over a 0.4 s cast
        let ticks: u32 = 9;
        // `dir` is deliberately non-zero and irrelevant: a LeapImpulse
        // overrides velocity outright, so replay must ignore `dir` too.
        let leaping: Vec<PendingIntent> = (1..=ticks)
            .map(|seq| PendingIntent { seq, dir: Vec2::new(0.0, 1.0), dt: DT, leap: Some(dash_velocity) })
            .collect();
        let dashed = replay_position(Vec3::ZERO, 6.0, leaping.iter());
        let expected = dash_velocity * DT * ticks as f32;
        assert!((dashed - expected).length() < 1e-4, "leap-aware replay must follow the dash exactly: {dashed:?}");

        let plain: Vec<PendingIntent> = (1..=ticks)
            .map(|seq| PendingIntent { seq, dir: Vec2::new(0.0, 1.0), dt: DT, leap: None })
            .collect();
        let dead_reckoned = replay_position(Vec3::ZERO, 6.0, plain.iter());
        assert!(
            (dashed - dead_reckoned).length() > SNAP_DISTANCE,
            "dead-reckoned WASD must diverge from the real dash past SNAP_DISTANCE, got {:.2}",
            (dashed - dead_reckoned).length()
        );
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

    /// Networking audit 2026-07-11, finding 7: "disconnect is a log line, no
    /// reconnect, no teardown" — this drives a real server, a real
    /// `NetClient` connection, and the real `NetReceiveSystem` (no
    /// reimplemented logic). A second login under the same character name
    /// makes the server's existing session-takeover kick the first
    /// connection (mirrors `phase6_login_takeover` in vordar-server's e2e
    /// suite) — a genuine, unannounced disconnect. engine-net's listening
    /// socket is never released once bound (findings 13/14/18 — no shutdown
    /// story), so an in-process bind/drop/rebind on the same port isn't
    /// possible; kicking the connection is the closest headless equivalent
    /// of "the server process restarted" that exercises the same
    /// Disconnected → teardown → backoff-redial → relogin path. The victim
    /// must notice, tear down, and relogin entirely on its own.
    #[test]
    fn kicked_connection_reconnects_and_relogs_in() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::env::set_current_dir(root).unwrap();

        let addr: SocketAddr = "127.0.0.1:25400".parse().unwrap();
        std::thread::spawn(move || {
            vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(1800));
        });
        std::thread::sleep(Duration::from_millis(300));

        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(DespawnQueue::new());
        resources.insert(Time::new());
        resources.insert(WorldTime { offset_micros: 0, synced: false });
        resources.insert(NetClientState {
            client: Some(NetClient::connect(addr, PROTOCOL_VERSION).expect("victim connect")),
            server_addr: addr,
            user: "reconnect-victim".into(),
            token: name_token("reconnect-victim"),
            login_denied: false,
            own_id: None,
            entities: HashMap::new(),
            seq: 0,
            predict: false,
            pending: VecDeque::new(),
            correction: Vec3::ZERO,
            simulated_rtt: Duration::ZERO,
            reconnect: None,
        });

        let mut recv = NetReceiveSystem;
        let deadline = Instant::now() + Duration::from_secs(5);
        while resources.get::<NetClientState>().unwrap().own_id.is_none() {
            assert!(Instant::now() < deadline, "victim never received its first Welcome");
            recv.run(&mut world, &mut resources, DT);
            std::thread::sleep(Duration::from_millis(16));
        }
        let first_id = resources.get::<NetClientState>().unwrap().own_id.unwrap();

        // Kick it: a second login under the same name takes over the
        // session server-side, closing the victim's connection out from
        // under it. Wait for the kicker's own Welcome before dropping it —
        // `Connection::close` tears the connection down immediately,
        // without flushing pending stream writes, so dropping right after
        // `send` can race the Login frame right off the wire.
        let mut kicker = NetClient::connect(addr, PROTOCOL_VERSION).expect("kicker connect");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut got_welcome = false;
            for ev in kicker.poll() {
                match ev {
                    ClientEvent::Connected => {
                        // Same derived token as the victim's login above — a
                        // takeover requires a token match (networking rework
                        // 1, finding 3); this test is about the reconnect
                        // state machine, not credential mismatches, so the
                        // kicker must present the victim's own token.
                        kicker.send(encode(&ClientMsg::Login {
                            name: "reconnect-victim".into(),
                            token: name_token("reconnect-victim"),
                        }));
                    }
                    ClientEvent::Message(data) => {
                        if let Some(ServerMsg::Welcome { .. }) = decode::<ServerMsg>(&data) {
                            got_welcome = true;
                        }
                    }
                    ClientEvent::Disconnected => {}
                    ClientEvent::Rejected(_) => {}
                }
            }
            if got_welcome {
                break;
            }
            assert!(Instant::now() < deadline, "kicker never got its own Welcome");
            std::thread::sleep(Duration::from_millis(16));
        }
        drop(kicker);

        // The victim must notice on its own — no test code touches its
        // NetClientState between here and full reconnection.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            recv.run(&mut world, &mut resources, DT);
            if reconnect_attempt(&resources).is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "the kick was never detected as a disconnect");
            std::thread::sleep(Duration::from_millis(16));
        }
        // Torn down immediately, not left dangling — the frozen-world half
        // of this finding's gap.
        assert!(
            resources.get::<NetClientState>().unwrap().own_id.is_none(),
            "an unexpected disconnect must clear own_id right away"
        );

        // Entirely automatic from here: backoff, redial, Connected, Login, Welcome.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            recv.run(&mut world, &mut resources, DT);
            let state = resources.get::<NetClientState>().unwrap();
            if state.own_id.is_some() && state.reconnect.is_none() {
                break;
            }
            assert!(Instant::now() < deadline, "the client never reconnected on its own");
            std::thread::sleep(Duration::from_millis(16));
        }
        let second_id = resources.get::<NetClientState>().unwrap().own_id.unwrap();
        assert_ne!(first_id, second_id, "reconnect must relogin into a fresh body");
    }

    /// Networking audit 2026-07-11, finding 11: "Prediction replay models
    /// plain movement only — leaps ... produce snaps at real latency." A real
    /// headless server plus a real predicting `NetClient` (150 ms simulated
    /// RTT, the finding's own number) cast a real Onslaught (the Ravager's
    /// gap-closer, `content/classes/ravager.ron`: 0.4 s dash, 8 s cooldown,
    /// 12-unit range) and drive exactly the systems `NetClientPlugin`
    /// registers for a predicting player (`NetReceiveSystem`,
    /// `NetSendInputSystem`, `PlayerMovementSystem`, `LeapSystem`,
    /// `MovementSystem`, `NetCorrectionSystem`) — no reimplemented logic.
    ///
    /// `reconcile_own` (invoked from inside `NetReceiveSystem::run` via
    /// `apply_snapshot`) snaps `transform.position` straight to `replayed`
    /// whenever the reconciliation error exceeds SNAP_DISTANCE; every other
    /// path (Trust, Smooth) leaves `transform.position` untouched. So a snap
    /// shows up as a position jump bigger than SNAP_DISTANCE measured across a
    /// single `NetReceiveSystem::run` call — that must never happen, during
    /// the dash or right after it.
    #[test]
    fn onslaught_dash_replay_never_snaps_at_150ms_rtt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::env::set_current_dir(&root).unwrap();

        let addr: SocketAddr = "127.0.0.1:25402".parse().unwrap();
        std::thread::spawn(move || {
            vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(2400));
        });
        std::thread::sleep(Duration::from_millis(300));

        // Real ability data (cast time, range) instead of hardcoded numbers —
        // the same content the server and a real client both load.
        let mut classes = vordar_game::class::ClassLibrary::new();
        classes.load_dir("content/classes");
        let onslaught = classes.get("ravager", "onslaught").expect("ravager has onslaught");
        let (cast_secs, max_range) = match &onslaught.effect {
            vordar_game::skills::AbilityEffect::Leap { cast_micros, max_range, .. } => {
                (*cast_micros as f32 / 1e6, *max_range)
            }
            _ => panic!("onslaught must be a Leap effect"),
        };

        // Component/prefab setup mirrors engine_app::prefab_plugin::PrefabPlugin
        // + vordar_game::plugin::GameComponentsPlugin without a full App — this
        // test drives systems directly, same as `kicked_connection_reconnects_and_relogs_in` above.
        let mut registry = engine_core::prefab::ComponentRegistry::new();
        engine_core::prefab::register_core_components(&mut registry);
        registry.register::<Player>("Player");
        registry.register::<vordar_game::enemies::Enemy>("Enemy");
        registry.register::<vordar_game::ContactDamage>("ContactDamage");
        registry.register::<vordar_game::CombatStats>("CombatStats");
        registry.register::<vordar_game::class::ClassId>("Class");
        registry.register::<vordar_game::class::RaceId>("Race");
        registry.register::<vordar_game::vfx::VfxTrail>("VfxTrail");
        let mut prefabs = engine_core::prefab::PrefabLibrary::new();
        prefabs.load_dir("content/prefabs");

        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(registry);
        resources.insert(prefabs);
        resources.insert(DespawnQueue::new());
        resources.insert(Time::new());
        resources.insert(WorldTime { offset_micros: 0, synced: false });
        resources.insert(EventBus::new());
        resources.insert(engine_app::input::KeyboardState::new());
        resources.insert(NetClientState {
            client: Some(
                NetClient::connect_with_latency(addr, PROTOCOL_VERSION, Duration::from_millis(150))
                    .expect("dasher connect"),
            ),
            server_addr: addr,
            user: "onslaught-dasher".into(),
            token: name_token("onslaught-dasher"),
            login_denied: false,
            own_id: None,
            entities: HashMap::new(),
            seq: 0,
            predict: true,
            pending: VecDeque::new(),
            correction: Vec3::ZERO,
            simulated_rtt: Duration::from_millis(150),
            reconnect: None,
        });

        let mut recv = NetReceiveSystem;
        let mut send = NetSendInputSystem;
        let mut player_move = PlayerMovementSystem;
        let mut leap_sys = vordar_game::combat::leap::LeapSystem;
        let mut move_sys = MovementSystem;
        let mut correction_sys = NetCorrectionSystem;
        let mut max_recv_jump = 0.0f32;

        // Input-phase half of one tick (NetReceiveSystem, NetSendInputSystem)
        // — watches every NetReceiveSystem call for a snap-sized position jump.
        let mut run_input = |world: &mut World, resources: &mut Resources| {
            resources.get_mut::<EventBus>().unwrap().clear();
            let before = own_entity(resources).and_then(|e| world.get::<&Transform>(e).ok().map(|t| t.position));
            recv.run(world, resources, DT);
            if let Some(before) = before {
                if let Some(after) =
                    own_entity(resources).and_then(|e| world.get::<&Transform>(e).ok().map(|t| t.position))
                {
                    max_recv_jump = max_recv_jump.max((after - before).length());
                }
            }
            send.run(world, resources, DT);
        };
        // Update-phase half (PlayerMovementSystem, LeapSystem, MovementSystem,
        // NetCorrectionSystem) — same order NetClientPlugin registers them in.
        let mut run_update = |world: &mut World, resources: &mut Resources| {
            player_move.run(world, resources, DT);
            leap_sys.run(world, resources, DT);
            move_sys.run(world, resources, DT);
            correction_sys.run(world, resources, DT);
        };

        // Welcome + clock sync.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            run_input(&mut world, &mut resources);
            run_update(&mut world, &mut resources);
            let ready = {
                let state = resources.get::<NetClientState>().unwrap();
                state.own_id.is_some() && state.client.as_ref().unwrap().server_now_micros().is_some()
            };
            if ready {
                break;
            }
            assert!(Instant::now() < deadline, "never got Welcome + clock sync");
            std::thread::sleep(Duration::from_millis(16));
        }

        // Finding 1 of docs/reviews/plan-networking-rework-1-2026-07-13.md:
        // cooldowns now persist as remainders instead of pessimistically
        // seeding full cooldown at spawn, so a fresh character's "onslaught"
        // is castable immediately — no cooldown-clearing wait needed. The
        // predicted entity itself is still created only once the first
        // Snapshot's `enters` list reaches this client (Welcome alone
        // doesn't spawn it), so pump until it exists.
        let entity_deadline = Instant::now() + Duration::from_secs(2);
        while own_entity(&resources).is_none() {
            run_input(&mut world, &mut resources);
            run_update(&mut world, &mut resources);
            assert!(Instant::now() < entity_deadline, "predicted entity never appeared after Welcome");
            std::thread::sleep(Duration::from_millis(16));
        }
        let entity = own_entity(&resources).expect("predicted entity must exist by now");
        let origin = world.get::<&Transform>(entity).unwrap().position;
        let target = origin + Vec3::new(max_range * 0.5, 0.0, 0.0);
        let cast_target = Vec2::new(target.x, target.z);
        let velocity = vordar_game::combat::leap::leap_velocity(origin, target, cast_secs);

        // The cast tick: Input phase first (the real wire CastIntent, so the
        // server mirrors the identical dash, same as NetSendInputSystem's own
        // MoveIntent send moments earlier), then the client-predicted
        // insertion `start_predicted_leap` — exactly what AbilityCastSystem
        // calls for a Leap ability, invoked directly here because
        // AbilityCastSystem itself needs a renderer (mouse-cursor ground
        // projection) this headless test has none of — then Update phase, so
        // the dash begins immediately this same tick like the real system.
        run_input(&mut world, &mut resources);
        {
            let state = resources.get_mut::<NetClientState>().unwrap();
            let t_server_micros = state.client.as_ref().unwrap().server_now_micros().unwrap();
            state.seq += 1;
            let seq = state.seq;
            state.client.as_ref().unwrap().send(encode(&ClientMsg::CastIntent {
                seq,
                t_server_micros,
                skill: "onslaught".into(),
                target: cast_target,
            }));
        }
        start_predicted_leap(&mut world, &mut resources, entity, velocity, cast_secs);
        run_update(&mut world, &mut resources);

        // Run through the dash and settle afterward, watching every tick's
        // NetReceiveSystem call for a snap.
        let dash_deadline = Instant::now() + Duration::from_secs(4);
        let mut elapsed = 0.0f32;
        while elapsed < cast_secs + 1.0 {
            assert!(Instant::now() < dash_deadline, "test loop stalled mid-dash");
            std::thread::sleep(Duration::from_millis(16));
            run_input(&mut world, &mut resources);
            run_update(&mut world, &mut resources);
            elapsed += DT;
        }

        assert!(
            max_recv_jump < SNAP_DISTANCE,
            "reconciliation snapped {max_recv_jump:.2} units mid-dash — leap-aware replay must keep \
             corrections under SNAP_DISTANCE ({SNAP_DISTANCE})"
        );
    }
}
