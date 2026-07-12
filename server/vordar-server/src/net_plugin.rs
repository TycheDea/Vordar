// NetServerPlugin — the seam between engine-net and the simulation.

use crate::db::{CharacterRecord, DbHandle, DbLoaded, DbWorker};
use engine_app::app::App;
use engine_app::events::{EventBus, HealthDepleted};
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_app::tick_rate::TickRate;
use engine_core::components::{Health, Transform};
use engine_core::prefab::{spawn_prefab, PrefabId};
use engine_core::spatial::SpatialGrid;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use engine_net::{ConnId, NetLimits, NetMetrics, NetServer, ServerEvent};
use glam::{Vec2, Vec3};
use hecs::Entity;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use vordar_game::combat::buff::{ravager_mods, RavagerRageSystem};
use vordar_game::combat::leap::{leap_velocity, LeapImpulse};
use vordar_game::combat::projectile::spawn_projectile;
use vordar_game::combat::stats::{compute_damage, DamageType};
use vordar_game::events::{DamageDealt, MoveIntent};
use vordar_game::player::class::{ClassId, ClassLibrary, DEFAULT_CLASS};
use vordar_game::player::movement_velocity;
use vordar_game::skills::AbilityEffect;
use vordar_game::{CombatStats, Enemy, Mechanic, Player, Provoked};
use vordar_game::world::WorldTimeRes;
use vordar_game::zones::{portal_hit, ZoneDef};
use vordar_protocol::{decode, encode, ClientMsg, EntityPos, EntityState, ServerMsg, PROTOCOL_VERSION, SNAPSHOT_HZ};

/// Lag-compensation rewind cap (DESIGN.md §3): high-latency players get
/// degraded forgiveness, not infinite rewind.
const MAX_REWIND_MICROS: u64 = 200_000;
/// Slack on the arrival deadline for clock-sync error and jitter.
const ARRIVAL_MARGIN_MICROS: u64 = 100_000;
/// Intents may not be stamped further in the future than clock-sync error allows.
const FUTURE_SLACK_MICROS: u64 = 50_000;
/// Validated intents waiting to be applied (~250 ms of input). Jitter bursts
/// buffer here; beyond the cap the oldest are dropped (the client re-converges
/// via reconciliation). Flooding buys queue latency, never extra speed.
const INTENT_QUEUE_CAP: usize = 16;

/// What a character spawns as. The Ravager is the playable class while there
/// is no character-creation/class-picker; the "player" (Human) prefab and its
/// kit stay shipped and tested.
const PLAYER_PREFAB: &str = "ravager";
/// Area-of-interest radius around each player: only entities inside it are
/// replicated to that client. Comfortably beyond the camera's view.
const AOI_RADIUS: f32 = 40.0;
/// Applied-intent history kept per connection for mechanic-resolve rewind
/// (~530 ms at 60 Hz — covers MAX_REWIND plus resolve-tick slack).
const HISTORY_CAP: usize = 32;
/// Crowd throttling: at most this many `states` entries per snapshot. Only
/// positions are capped — enters/leaves/known stay full-AOI, or the diff
/// protocol would corrupt. ~64 × 17 B × 10 Hz ≈ 11 KB/s per client steady
/// state, regardless of crowd size.
const MAX_SNAPSHOT_STATES: usize = 64;
/// Of the budget, the nearest N entities are always included; the rest of
/// the AOI shares the remaining slots round-robin (full refresh within
/// ~500 ms even in a 200-crowd; NetLerp absorbs the lower rate).
const NEAREST_GUARANTEED: usize = 32;
/// Fixed server tick duration — each applied intent integrates exactly this.
const TICK_DT: f32 = 1.0 / 60.0;
/// PostUpdate runs at the sim rate; the 10 Hz systems below self-gate on it.
const POST_HZ: f32 = 60.0;
/// Snapshot stagger: each connection is served every STAGGER-th PostUpdate
/// run (still SNAPSHOT_HZ per client) — the fan-out cost splits into STAGGER
/// slices instead of landing on one tick.
const STAGGER: u64 = (POST_HZ / SNAPSHOT_HZ) as u64;
/// Autosave every Nth PostUpdate run (60 Hz → ~30 s).
const AUTOSAVE_TICKS: u64 = 1800;

pub struct NetServerPlugin {
    pub addr: SocketAddr,
    /// SQLite path for character persistence (`:memory:` for throwaway).
    pub db_path: String,
    /// Bind-time connection-cap configuration (networking audit 2026-07-11,
    /// finding 20). `NetLimits::default()` for production; a soak harness
    /// modeling many distinct clients from one source IP raises
    /// `max_connections_per_ip` here instead.
    pub limits: NetLimits,
}

impl Plugin for NetServerPlugin {
    fn build(&self, app: &mut App) {
        // Single-zone convenience: a lone "start" zone with no portals that
        // owns its DbWorker. Multi-zone servers call `install` directly with
        // a shared worker and the real topology.
        let server = NetServer::bind_with_limits(self.addr, PROTOCOL_VERSION, self.limits)
            .unwrap_or_else(|e| panic!("failed to bind {}: {e}", self.addr));
        let db_owner = DbWorker::spawn(&self.db_path)
            .unwrap_or_else(|e| panic!("failed to open db '{}': {e}", self.db_path));
        let db = db_owner.handle();
        let zone = ZoneDef { name: "start".into(), chapter: None, portals: Vec::new(), visuals: Default::default() };
        let directory = HashMap::from([("start".to_owned(), server.local_addr())]);
        install(app, server, db, Some(db_owner), zone, directory, Instant::now());
    }
}

/// Wire networking, persistence, and zone identity into a zone App.
/// `directory` maps every zone name to its public address (for Redirects);
/// `world_origin` is the shared world-time epoch — pass the SAME Instant to
/// every zone so world events fire simultaneously across them.
pub fn install(
    app: &mut App,
    server: NetServer,
    db: DbHandle,
    db_owner: Option<DbWorker>,
    zone: ZoneDef,
    directory: HashMap<String, SocketAddr>,
    world_origin: Instant,
) {
    app.insert_resource(NetServerState::new(server, db, db_owner, zone, directory, world_origin))
        // World time published every tick for world systems (events, day/night).
        .insert_resource(WorldTimeRes(0))
        .set_phase_rate(Phase::PostUpdate, TickRate::Fixed(POST_HZ))
        .add_system(NetReceiveSystem, Phase::Input, SystemOrder::Default)
        // Deaths broadcast before the flush removes the dying entity.
        .add_system(DeathBroadcastSystem, Phase::DespawnFlush, SystemOrder::First)
        // Resolve before broadcasting so deaths reach the same snapshot wave.
        .add_system(MechanicResolveSystem::new(), Phase::PostUpdate, SystemOrder::before::<SnapshotBroadcastSystem>())
        // Rage stacks read the tick's DamageDealt events: CollisionResolve's
        // (earlier phase) and MechanicResolve's (just above) in one pass.
        .add_system(RavagerRageSystem, Phase::PostUpdate, SystemOrder::after::<MechanicResolveSystem>())
        // Transfer before broadcasting: a redirected player must not receive
        // one more snapshot after their Redirect.
        .add_system(ZoneTransferSystem::new(), Phase::PostUpdate, SystemOrder::before::<SnapshotBroadcastSystem>())
        .add_system(SnapshotBroadcastSystem::new(), Phase::PostUpdate, SystemOrder::Default)
        .add_system(AutosaveSystem { ticks: 0 }, Phase::PostUpdate, SystemOrder::Default);
}

struct PlayerConn {
    entity: Entity,
    /// Character name — the persistence key (identity without auth during
    /// development; accounts land later).
    name: String,
    /// Validated intents as (seq, client stamp, dir) in arrival order, applied
    /// ONE PER TICK. The client emits exactly one intent per fixed Input tick,
    /// so consuming one per fixed server tick integrates the same dirs over
    /// the same steps — the client's prediction replay matches bit-for-bit.
    queue: VecDeque<(u32, u64, Vec2)>,
    /// Seq of the last intent APPLIED to the simulation — the snapshot ack.
    applied_seq: u32,
    /// Seq/stamp of the last intent RECEIVED — validation monotonicity.
    last_seq: u32,
    last_t: u64,
    /// Entity ids currently inside this client's AOI — diffed each snapshot
    /// to produce enter/leave messages.
    known: HashSet<u64>,
    /// Recently APPLIED intents as (client stamp, dir) — each entry is exactly
    /// one tick of integration. Mechanic resolution rewinds through these to
    /// evaluate "position at T" by stamp time (favor-the-defender).
    history: VecDeque<(u64, Vec2)>,
    /// Server time of the last accepted cast PER SKILL (cooldown
    /// enforcement) — a fast skill must not eat a slow skill's cooldown.
    last_cast: HashMap<String, u64>,
    /// Round-robin cursor for snapshot `states` throttling — where the
    /// non-nearest rotation resumes next snapshot.
    rr_cursor: usize,
}

pub struct NetServerState {
    server: NetServer,
    db: DbHandle,
    /// Owns the worker when this zone spawned it (single-zone path).
    /// Declared after `db` so the handle's senders drop first — the owner's
    /// Drop joins the worker, which exits only when all senders are gone.
    _db_owner: Option<DbWorker>,
    /// This App's zone: its name (login routing, saves) and its portals.
    zone: ZoneDef,
    /// Zone name → public address, for Redirect messages.
    directory: HashMap<String, SocketAddr>,
    /// world time − server time, fixed at install. Both clocks are Instant-
    /// based monotonic time, so the offset never drifts; deriving world time
    /// from ONE server-clock read keeps the WorldClock mapping bit-identical
    /// across every message this zone sends. Zones sharing a `world_origin`
    /// agree on world time up to one clock-read of sampling error (µs).
    world_offset_micros: i64,
    conns: HashMap<ConnId, PlayerConn>,
    /// Logins whose character load is in flight: conn → name. Spawn + Welcome
    /// happen when the DbLoaded result arrives.
    loading: HashMap<ConnId, String>,
    tick: u64,
    next_mechanic_id: u64,
}

impl NetServerState {
    fn new(
        server: NetServer,
        db: DbHandle,
        db_owner: Option<DbWorker>,
        zone: ZoneDef,
        directory: HashMap<String, SocketAddr>,
        world_origin: Instant,
    ) -> Self {
        let world_offset_micros = world_origin.elapsed().as_micros() as i64 - server.now_micros() as i64;
        Self {
            server,
            db,
            _db_owner: db_owner,
            zone,
            directory,
            world_offset_micros,
            conns: HashMap::new(),
            loading: HashMap::new(),
            tick: 0,
            next_mechanic_id: 0,
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.server.local_addr()
    }

    /// Network-layer counters, including the busy-time proxy (networking
    /// audit 2026-07-11, finding 14 step 1) — exposed so the soak harness can
    /// sample the network thread's saturation directly.
    pub fn metrics(&self) -> Arc<NetMetrics> {
        self.server.metrics()
    }

    /// World time corresponding to server time `at_server_micros`.
    fn world_at(&self, at_server_micros: u64) -> u64 {
        (at_server_micros as i64 + self.world_offset_micros).max(0) as u64
    }

    /// Current world time (microseconds since the shared origin).
    fn world_micros(&self) -> u64 {
        self.world_at(self.server.now_micros())
    }
}

/// Spread spawn points so simultaneous joins don't stack on the origin.
fn spawn_position(conn: ConnId) -> Vec3 {
    let angle = (conn as f32) * (std::f32::consts::TAU / 8.0);
    Vec3::new(angle.cos() * 3.0, 0.0, angle.sin() * 3.0)
}

/// Connections whose player is within AOI range of `center` — the interest-
/// management filter for the mechanic sends below (Finding 5 of
/// docs/reviews/audit-networking-2026-07-11.md: `state.server.broadcast` used
/// to fan MechanicScheduled/HitResult out to EVERY connection, including
/// pre-login ones, which both wasted O(players × casts) bandwidth and gave a
/// cheating client a zone-wide radar off telegraph positions). Uses the same
/// `AOI_RADIUS` as snapshot replication so a telegraph/hit result reaches
/// exactly the clients who could plausibly see it. A client that walks into
/// range only after the message already went out simply misses that one
/// telegraph/hit notification: telegraphs last seconds and `HitResult` still
/// resolves hits correctly regardless of who was told about the schedule, so
/// the miss is cosmetic — accepted rather than adding re-send bookkeeping.
fn aoi_conns(conns: &HashMap<ConnId, PlayerConn>, world: &World, center: Vec3) -> Vec<ConnId> {
    conns
        .iter()
        .filter(|(_, pc)| {
            world
                .get::<&Transform>(pc.entity)
                .is_ok_and(|tr| tr.position.distance_squared(center) <= AOI_RADIUS * AOI_RADIUS)
        })
        .map(|(&conn, _)| conn)
        .collect()
}

pub struct NetReceiveSystem;

impl System for NetReceiveSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Cloned once up front: ClassLibrary is read-only content, and this
        // sidesteps holding an immutable Resources borrow across the event
        // loop's many `resources.get_mut::<NetServerState>()` calls below.
        let class_library = resources.get::<ClassLibrary>()
            .expect("ClassLibrary not in resources")
            .clone();

        // Publish the world clock for world systems (events, future schedules).
        let world_now = resources.get::<NetServerState>().unwrap().world_micros();
        resources.get_mut::<WorldTimeRes>().unwrap().0 = world_now;

        let events = resources.get_mut::<NetServerState>().unwrap().server.poll();

        // Projectile casts accepted this tick — spawned after the event loop
        // releases the NetServerState borrow (spawn_projectile needs resources).
        let mut pending_bolts: Vec<(String, Vec3, Vec3, f32, i32, DamageType, f32, Entity)> = Vec::new();

        for event in events {
            match event {
                ServerEvent::Connected(conn) => {
                    // The connection isn't in the game until Login arrives:
                    // identity picks the character, the character picks the
                    // spawn (loaded position + health).
                    log::info!("conn {conn}: connected, awaiting login");
                }
                ServerEvent::Disconnected(conn) => {
                    let state = resources.get_mut::<NetServerState>().unwrap();
                    state.loading.remove(&conn);
                    if let Some(pc) = state.conns.remove(&conn) {
                        // Persist before queuing the despawn — DespawnFlush
                        // runs later in the frame, the entity is still alive.
                        if let (Ok(tr), Ok(hp)) = (world.get::<&Transform>(pc.entity), world.get::<&Health>(pc.entity)) {
                            let zone = state.zone.name.clone();
                            state.db.save(pc.name.clone(), zone, tr.position, hp.current);
                        }
                        resources.get_mut::<DespawnQueue>().unwrap().push(pc.entity, None);
                        log::info!("conn {conn}: disconnected, despawning {:?}", pc.entity);
                    }
                }
                ServerEvent::Message { conn, data, recv_micros } => {
                    let state = resources.get_mut::<NetServerState>().unwrap();
                    let Some(msg) = decode::<ClientMsg>(&data) else {
                        log::warn!("conn {conn}: undecodable message ({} bytes)", data.len());
                        continue;
                    };
                    // Login comes from a connection that has no PlayerConn
                    // yet — handle it before the guard below.
                    if let ClientMsg::Login { name } = &msg {
                        if name.len() > 32 || !name.chars().all(|c| c.is_ascii_graphic() && c != ' ') {
                            log::warn!("conn {conn}: invalid login name");
                            continue;
                        }
                        if state.conns.contains_key(&conn) || state.loading.contains_key(&conn) {
                            log::debug!("conn {conn}: duplicate login ignored");
                            continue;
                        }
                        // Session takeover: the newest connection wins. The
                        // old one is usually a stale session — a closed client
                        // whose QUIC close never arrived (process exit can
                        // outrace the close frame) lingers until the idle
                        // timeout, and ignoring the relogin until then would
                        // leave the new client waiting forever for Welcome.
                        let old = state.conns.iter()
                            .find(|(_, pc)| pc.name == *name)
                            .map(|(&c, _)| c);
                        if let Some(old_conn) = old {
                            let pc = state.conns.remove(&old_conn).unwrap();
                            // Same save-then-despawn as a real disconnect, so
                            // the takeover load (FIFO behind it) restores the
                            // freshest state.
                            if let (Ok(tr), Ok(hp)) = (world.get::<&Transform>(pc.entity), world.get::<&Health>(pc.entity)) {
                                let zone = state.zone.name.clone();
                                state.db.save(pc.name.clone(), zone, tr.position, hp.current);
                            }
                            state.server.disconnect(old_conn);
                            log::info!("conn {conn}: '{name}' takes over session from conn {old_conn}");
                            resources.get_mut::<DespawnQueue>().unwrap().push(pc.entity, None);
                        }
                        let state = resources.get_mut::<NetServerState>().unwrap();
                        // A same-name load still in flight belongs to another
                        // stale connection — forget it (its DbLoaded result
                        // gets discarded) and kick that connection too.
                        if let Some(stale) = state.loading.iter().find(|(_, n)| *n == name).map(|(&c, _)| c) {
                            state.loading.remove(&stale);
                            state.server.disconnect(stale);
                        }
                        log::info!("conn {conn}: login as '{name}', loading character");
                        state.loading.insert(conn, name.clone());
                        // Defaults seed a NEW character only: ring spawn +
                        // the player prefab's full health (player.ron is
                        // the source of truth; the DB merely overrides
                        // Health.current after spawn).
                        // (The zone field is decorative here: the schema
                        // default puts every NEW character in 'start'.)
                        let defaults = CharacterRecord { zone: "start".into(), pos: spawn_position(conn), health: 100 };
                        state.db.load_or_create(conn, name.clone(), defaults);
                        continue;
                    }
                    let rtt = state.server.rtt_micros(conn).unwrap_or(0);
                    let Some(pc) = state.conns.get_mut(&conn) else { continue };

                    match msg {
                        ClientMsg::MoveIntent { seq, t_server_micros: t, dir } => {
                            if let Err(reason) = validate_intent(pc, seq, t, recv_micros, rtt) {
                                log::debug!("conn {conn}: intent rejected ({reason})");
                                // Operational visibility (networking audit 2026-07-11,
                                // finding 18): validate_intent's callers used to only
                                // log::debug!, so a client spamming invalid intents was
                                // as invisible to NetMetrics as one behaving normally.
                                state.server.metrics().record_reject();
                                continue;
                            }
                            pc.last_seq = seq;
                            pc.last_t = t;
                            // Max-speed validation: direction can never exceed unit length.
                            // Reject only genuine violations (NaN/Inf, or well past unit
                            // length); tolerate epsilon-scale float noise from the client's
                            // f32 `normalize()` and clamp it — same rule as the shared
                            // `movement_velocity` the client replays, so validation and
                            // simulation agree instead of forking.
                            if !dir.is_finite() || dir.length_squared() > 1.0 + 1e-3 { continue; }
                            let dir = if dir.length_squared() > 1.0 { dir.normalize() } else { dir };
                            pc.queue.push_back((seq, t, dir));
                            if pc.queue.len() > INTENT_QUEUE_CAP {
                                pc.queue.pop_front();
                            }
                        }
                        ClientMsg::CastIntent { seq, t_server_micros: t, skill: skill_id, target } => {
                            if let Err(reason) = validate_intent(pc, seq, t, recv_micros, rtt) {
                                log::warn!("conn {conn}: cast rejected ({reason})");
                                // See the MoveIntent arm above (finding 18).
                                state.server.metrics().record_reject();
                                continue;
                            }
                            pc.last_seq = seq;
                            pc.last_t = t;
                            let caster = pc.entity;
                            let class_id = world.get::<&ClassId>(caster)
                                .map(|c| c.id.clone())
                                .unwrap_or_else(|_| DEFAULT_CLASS.to_owned());
                            let Some(def) = class_library.get(&class_id, &skill_id) else {
                                log::warn!("conn {conn}: unknown ability '{skill_id}' for class '{class_id}'");
                                continue;
                            };
                            let now = state.server.now_micros();
                            let on_cooldown = pc.last_cast.get(&skill_id)
                                .is_some_and(|&last| now.saturating_sub(last) < def.cooldown_micros);
                            if on_cooldown {
                                log::debug!("conn {conn}: '{skill_id}' on cooldown");
                                continue;
                            }
                            let Ok(caster_pos) = world.get::<&Transform>(caster).map(|tr| tr.position) else {
                                continue;
                            };
                            let target = Vec3::new(target.x, 0.0, target.y);
                            if !target.is_finite() { continue; }
                            match &def.effect {
                                AbilityEffect::Scheduled { telegraph_prefab, radius, damage, damage_type, cast_micros, max_range } => {
                                    let (telegraph_prefab, radius, damage, damage_type, cast_micros, max_range) =
                                        (telegraph_prefab.clone(), *radius, *damage, *damage_type, *cast_micros, *max_range);
                                    if caster_pos.distance_squared(target) > max_range * max_range {
                                        log::debug!("conn {conn}: cast out of range");
                                        continue;
                                    }
                                    pc.last_cast.insert(skill_id.clone(), now);
                                    state.next_mechanic_id += 1;
                                    let id = state.next_mechanic_id;
                                    // Schedule in ABSOLUTE server time and tell everyone the
                                    // same thing (DESIGN.md §3) — T = telegraph completion.
                                    let resolve_at_micros = now + cast_micros;
                                    world.spawn((
                                        Transform::new(target),
                                        Mechanic {
                                            id,
                                            radius,
                                            damage,
                                            damage_type,
                                            resolve_at_micros,
                                            caster,
                                        },
                                    ));
                                    let frame = encode(&ServerMsg::MechanicScheduled {
                                        id,
                                        telegraph_prefab,
                                        pos: target,
                                        radius,
                                        resolve_at_micros,
                                        duration_micros: cast_micros,
                                    });
                                    for c in aoi_conns(&state.conns, world, target) {
                                        state.server.send(c, frame.clone());
                                    }
                                    log::info!("conn {conn}: mechanic {id} ('{skill_id}') resolves at {resolve_at_micros}");
                                }
                                AbilityEffect::Projectile { prefab, speed, damage, damage_type, ttl_secs, spawn_offset } => {
                                    let (prefab, speed, damage, damage_type, ttl_secs, spawn_offset) =
                                        (prefab.clone(), *speed, *damage, *damage_type, *ttl_secs, *spawn_offset);
                                    // No range gate: the target only fixes the
                                    // flight direction; the projectile itself
                                    // is the range limit (speed × ttl).
                                    let mut dir = target - caster_pos;
                                    dir.y = 0.0;
                                    if dir.length_squared() < 1e-6 {
                                        continue; // degenerate aim at own feet
                                    }
                                    let dir = dir.normalize();
                                    pc.last_cast.insert(skill_id.clone(), now);
                                    pending_bolts.push((
                                        prefab,
                                        caster_pos + dir * spawn_offset,
                                        dir,
                                        speed,
                                        damage,
                                        damage_type,
                                        ttl_secs,
                                        caster,
                                    ));
                                }
                                AbilityEffect::Leap { telegraph_prefab, radius, damage, damage_type, cast_micros, max_range } => {
                                    let (telegraph_prefab, radius, damage, damage_type, cast_micros, max_range) =
                                        (telegraph_prefab.clone(), *radius, *damage, *damage_type, *cast_micros, *max_range);
                                    if caster_pos.distance_squared(target) > max_range * max_range {
                                        log::debug!("conn {conn}: leap out of range");
                                        continue;
                                    }
                                    pc.last_cast.insert(skill_id.clone(), now);
                                    state.next_mechanic_id += 1;
                                    let id = state.next_mechanic_id;
                                    // Same scheduling as Scheduled — the arrival hit test IS a
                                    // Mechanic — plus a dash whose countdown ends at the same
                                    // instant (both derived from cast_micros).
                                    let resolve_at_micros = now + cast_micros;
                                    let cast_secs = cast_micros as f32 / 1e6;
                                    world.spawn((
                                        Transform::new(target),
                                        Mechanic {
                                            id,
                                            radius,
                                            damage,
                                            damage_type,
                                            resolve_at_micros,
                                            caster,
                                        },
                                    ));
                                    let _ = world.insert_one(caster, LeapImpulse {
                                        velocity: leap_velocity(caster_pos, target, cast_secs),
                                        remaining: cast_secs,
                                    });
                                    let frame = encode(&ServerMsg::MechanicScheduled {
                                        id,
                                        telegraph_prefab,
                                        pos: target,
                                        radius,
                                        resolve_at_micros,
                                        duration_micros: cast_micros,
                                    });
                                    for c in aoi_conns(&state.conns, world, target) {
                                        state.server.send(c, frame.clone());
                                    }
                                    log::info!("conn {conn}: leap mechanic {id} ('{skill_id}') resolves at {resolve_at_micros}");
                                }
                            }
                        }
                        // Handled before the PlayerConn guard above.
                        ClientMsg::Login { .. } => {}
                    }
                }
            }
        }

        // Spawn the projectiles accepted above (player-fired: damages enemies).
        for (prefab, origin, dir, speed, damage, damage_type, ttl, caster) in pending_bolts {
            spawn_projectile(world, resources, &prefab, origin, dir, speed, damage, damage_type, ttl, caster, false);
        }

        // Finished character loads → spawn + Welcome. The connection enters
        // the game only now; anything it sent earlier was dropped by the
        // PlayerConn guard.
        let loaded = resources.get_mut::<NetServerState>().unwrap().db.poll();
        for DbLoaded { conn, name, record } in loaded {
            if resources.get_mut::<NetServerState>().unwrap().loading.remove(&conn).is_none() {
                continue; // disconnected while the load was in flight
            }
            // Login routing: this zone serves only characters it owns. The
            // owner's address comes from the directory; the client closes
            // this connection and logs in there instead.
            {
                let state = resources.get_mut::<NetServerState>().unwrap();
                if record.zone != state.zone.name {
                    match state.directory.get(&record.zone) {
                        Some(&addr) => {
                            log::info!("conn {conn}: '{name}' belongs to zone '{}' — redirecting to {addr}", record.zone);
                            state.server.send(conn, encode(&ServerMsg::Redirect { zone: record.zone, addr }));
                        }
                        None => {
                            log::error!("conn {conn}: '{name}' in unknown zone '{}' — disconnecting", record.zone);
                            state.server.disconnect(conn);
                        }
                    }
                    continue;
                }
            }
            let result = spawn_prefab(PLAYER_PREFAB, record.pos, &mut SpawnContext { world, resources });
            let state = resources.get_mut::<NetServerState>().unwrap();
            match result {
                Ok(entity) => {
                    // The prefab is the source of truth for everything but
                    // the persisted fields; the DB overrides Health.current.
                    if let Ok(mut hp) = world.get::<&mut Health>(entity) {
                        hp.current = record.health;
                    }
                    // Finding 8 of docs/reviews/audit-networking-2026-07-11.md:
                    // cooldowns live only in this connection's `last_cast` map,
                    // which relogging used to recreate empty — burn a cooldown,
                    // disconnect, relog, cast again for free. Pessimistically
                    // start every ability of the character's class on full
                    // cooldown as of spawn: relog is never an advantage even
                    // though the true remaining cooldown isn't persisted yet.
                    let class_id = world.get::<&ClassId>(entity)
                        .map(|c| c.id.clone())
                        .unwrap_or_else(|_| DEFAULT_CLASS.to_owned());
                    let spawn_now = state.server.now_micros();
                    let last_cast: HashMap<String, u64> = class_library.abilities_of(&class_id)
                        .iter()
                        .map(|a| (a.id.clone(), spawn_now))
                        .collect();
                    state.conns.insert(conn, PlayerConn {
                        entity,
                        name: name.clone(),
                        queue: VecDeque::new(),
                        applied_seq: 0,
                        last_seq: 0,
                        last_t: 0,
                        known: HashSet::new(),
                        history: VecDeque::new(),
                        last_cast,
                        rr_cursor: 0,
                    });
                    state.server.send(conn, encode(&ServerMsg::Welcome { player_id: entity.to_bits().get() }));
                    let at_server_micros = state.server.now_micros();
                    let world_micros = state.world_at(at_server_micros);
                    state.server.send(conn, encode(&ServerMsg::WorldClock { world_micros, at_server_micros }));
                    log::info!("conn {conn}: '{name}' joined as {entity:?} ({} online)", state.conns.len());
                }
                Err(e) => log::error!("conn {conn}: player spawn failed: {e}"),
            }
        }

        // A connection must always own a live player: combat can kill the
        // entity (real death/respawn design lands with later phases) — until
        // then, respawn at the connection's spawn point and re-Welcome the
        // client so prediction and snapshots rebind to the new body.
        let dead: Vec<ConnId> = {
            let state = resources.get::<NetServerState>().unwrap();
            state.conns.iter()
                .filter(|&(_, pc)| !world.contains(pc.entity))
                .map(|(&conn, _)| conn)
                .collect()
        };
        for conn in dead {
            let result = spawn_prefab(PLAYER_PREFAB, spawn_position(conn), &mut SpawnContext { world, resources });
            let state = resources.get_mut::<NetServerState>().unwrap();
            let Some(pc) = state.conns.get_mut(&conn) else { continue };
            match result {
                Ok(entity) => {
                    pc.entity = entity;
                    pc.queue.clear();
                    state.server.send(conn, encode(&ServerMsg::Welcome { player_id: entity.to_bits().get() }));
                    log::info!("conn {conn}: player died — respawned as {entity:?}");
                }
                Err(e) => log::error!("conn {conn}: respawn failed: {e}"),
            }
        }

        // Apply exactly one queued intent per connection per tick for the
        // shared movement system. An empty queue (arrival jitter) means one
        // tick standing still — the position deficit stays accounted for in
        // the client's pending replay, so prediction error remains zero.
        let intents: Vec<(Entity, Vec2)> = {
            let state = resources.get_mut::<NetServerState>().unwrap();
            state.conns.values_mut()
                .filter_map(|pc| {
                    let (seq, stamp, dir) = pc.queue.pop_front()?;
                    pc.applied_seq = seq;
                    pc.history.push_back((stamp, dir));
                    if pc.history.len() > HISTORY_CAP {
                        pc.history.pop_front();
                    }
                    Some((pc.entity, dir))
                })
                .collect()
        };
        let bus = resources.get_mut::<EventBus>().unwrap();
        for (entity, dir) in intents {
            bus.emit(MoveIntent { entity, dir });
        }
    }
}

/// Anti-cheat caps from DESIGN.md §3, in the protocol from v1.
fn validate_intent(pc: &PlayerConn, seq: u32, t: u64, recv_micros: u64, rtt: u64) -> Result<(), &'static str> {
    // seq=0 is PlayerConn::last_seq's "nothing received yet" sentinel, never a
    // value a genuine client sends (the client's own seq counter starts at 1)
    // — reject it outright first, so a spoofed/replayed seq=0 intent can't
    // hide behind the sentinel and pass monotonicity forever.
    if seq == 0 {
        return Err("stale seq");
    }
    // Monotonic, stream-consistent: replays and backdated contradictions are free rejects.
    if seq <= pc.last_seq {
        return Err("stale seq");
    }
    if t < pc.last_t {
        return Err("timestamp not monotonic");
    }
    // No future stamps beyond plausible clock-sync error.
    if t > recv_micros + FUTURE_SLACK_MICROS {
        return Err("timestamp in the future");
    }
    // Arrival deadline: an input claiming time T must arrive within ~one RTT
    // of T. MAX_REWIND acts as a floor while RTT estimates settle; the actual
    // lag-compensation rewind (Phase 4 snapshot tests) is capped separately.
    let max_age = rtt.max(MAX_REWIND_MICROS) + ARRIVAL_MARGIN_MICROS;
    if recv_micros.saturating_sub(t) > max_age {
        return Err("arrived past deadline");
    }
    Ok(())
}

/// The scheduled-snapshot test (DESIGN.md §3): at the first resolve tick past
/// each mechanic's T, decide who was inside its area AT T — players via
/// stamp-based rewind through their applied-intent history (an input stamped
/// ≤ T counts even though it arrived after T: favor-the-defender), NPCs at
/// their current server-driven position. Damage flows through Health, so
/// deaths take the existing HealthDepleted/despawn path.
pub struct MechanicResolveSystem {
    ticks: u64,
}

impl MechanicResolveSystem {
    pub fn new() -> Self {
        Self { ticks: 0 }
    }
}

impl System for MechanicResolveSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // PostUpdate runs at POST_HZ; resolve keeps its 10 Hz cadence.
        let due_now = self.ticks % STAGGER == 0;
        self.ticks += 1;
        if !due_now {
            return;
        }
        let now = resources.get::<NetServerState>().unwrap().server.now_micros();

        let due: Vec<(Entity, Mechanic, Vec3)> = world
            .query::<(Entity, &Transform, &Mechanic)>()
            .iter()
            .filter(|(_, _, m)| now >= m.resolve_at_micros)
            .map(|(e, t, m)| (e, *m, t.position))
            .collect();
        if due.is_empty() {
            return;
        }

        for (mech_entity, mech, center) in due {
            // Rewind to T, but never further back from now than the cap —
            // high-latency players get degraded forgiveness, not infinite rewind.
            let t_eff = mech.resolve_at_micros.max(now.saturating_sub(MAX_REWIND_MICROS));

            let targets: Vec<(Entity, Vec3)> = world
                .query::<(Entity, &Transform, &Health)>()
                .iter()
                .filter(|&(e, ..)| e != mech.caster)
                .map(|(e, t, _)| (e, t.position))
                .collect();

            let mut hits: Vec<u64> = Vec::new();
            let mut hit_entities: Vec<Entity> = Vec::new();
            {
                let state = resources.get::<NetServerState>().unwrap();
                for (entity, pos) in targets {
                    let pos_at_t = match state.conns.values().find(|pc| pc.entity == entity) {
                        Some(pc) => {
                            let speed = world.get::<&Player>(entity).map(|p| p.speed).unwrap_or(0.0);
                            rewound_position(pos, speed, &pc.history, t_eff)
                        }
                        None => pos,
                    };
                    if pos_at_t.distance_squared(center) <= mech.radius * mech.radius {
                        hits.push(entity.to_bits().get());
                        hit_entities.push(entity);
                    }
                }
            }

            for &entity in &hit_entities {
                let dmg = {
                    let atk = world.get::<&CombatStats>(mech.caster).ok();
                    let def = world.get::<&CombatStats>(entity).ok();
                    let seed = mech.id ^ entity.to_bits().get().rotate_left(21);
                    let (bonus_power, mult) = ravager_mods(world, mech.caster, entity);
                    let base = compute_damage(mech.damage + bonus_power, mech.damage_type, atk.as_deref(), def.as_deref(), seed);
                    (base as f32 * mult).round() as i32
                };
                if let Ok(mut health) = world.get::<&mut Health>(entity) {
                    health.current -= dmg;
                    resources
                        .get_mut::<EventBus>()
                        .unwrap()
                        .emit(DamageDealt { attacker: mech.caster, target: entity, amount: dmg });
                }
                // Targeted damage wakes passive enemies, same as projectiles.
                if world.get::<&Enemy>(entity).is_ok() {
                    let _ = world.insert_one(entity, Provoked);
                }
            }

            log::info!("mechanic {} resolved: {} hit", mech.id, hit_entities.len());
            let state = resources.get::<NetServerState>().unwrap();
            let frame = encode(&ServerMsg::HitResult { mechanic: mech.id, hits });
            for c in aoi_conns(&state.conns, world, center) {
                state.server.send(c, frame.clone());
            }
            let _ = world.despawn(mech_entity);
        }
    }
}

/// Walk the applied-intent history backwards, undoing every tick whose intent
/// was STAMPED after `t_eff`. Each entry is exactly one tick of integration
/// (the 1-intent-per-tick queue model), so this reconstructs the position the
/// player had committed to by time T on their own synced clock.
fn rewound_position(current: Vec3, speed: f32, history: &VecDeque<(u64, Vec2)>, t_eff: u64) -> Vec3 {
    let mut pos = current;
    for &(stamp, dir) in history.iter().rev() {
        if stamp <= t_eff {
            break;
        }
        pos -= movement_velocity(dir, speed) * TICK_DT;
    }
    pos
}

/// Portal handoff (Phase 7): persist → despawn → redirect. The character is
/// saved into the TARGET zone at the portal's arrival point, the body leaves
/// this zone, and the client is told where to log in next. The CLIENT closes
/// the connection — kicking here could outrace the Redirect frame (the
/// Phase 6 takeover lesson). The eventual Disconnected finds no PlayerConn,
/// so no stale save can clobber the transfer save.
pub struct ZoneTransferSystem {
    ticks: u64,
}

impl ZoneTransferSystem {
    pub fn new() -> Self {
        Self { ticks: 0 }
    }
}

impl System for ZoneTransferSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // PostUpdate runs at POST_HZ; transfers keep their 10 Hz cadence.
        let due_now = self.ticks % STAGGER == 0;
        self.ticks += 1;
        if !due_now {
            return;
        }
        let transfers: Vec<ConnId> = {
            let state = resources.get::<NetServerState>().unwrap();
            if state.zone.portals.is_empty() {
                return;
            }
            state.conns.iter()
                .filter(|(_, pc)| {
                    world.get::<&Transform>(pc.entity)
                        .is_ok_and(|tr| portal_hit(&state.zone.portals, tr.position).is_some())
                })
                .map(|(&conn, _)| conn)
                .collect()
        };

        for conn in transfers {
            let state = resources.get_mut::<NetServerState>().unwrap();
            let Some(pc) = state.conns.get(&conn) else { continue };
            let Ok(pos) = world.get::<&Transform>(pc.entity).map(|tr| tr.position) else { continue };
            let portal = portal_hit(&state.zone.portals, pos).unwrap().clone();
            let Some(&addr) = state.directory.get(&portal.target_zone) else {
                // Content validation makes this unreachable; never strand the
                // player in a half-transferred state over a config bug.
                log::error!("portal targets unlisted zone '{}' — ignoring", portal.target_zone);
                continue;
            };
            let pc = state.conns.remove(&conn).unwrap();
            let health = world.get::<&Health>(pc.entity).map(|hp| hp.current).unwrap_or(100);
            // Save FIRST: the FIFO db queue puts this ahead of the relogin
            // load the redirected client is about to trigger in the target.
            state.db.save(pc.name.clone(), portal.target_zone.clone(), portal.target_pos, health);
            state.server.send(conn, encode(&ServerMsg::Redirect { zone: portal.target_zone.clone(), addr }));
            resources.get_mut::<DespawnQueue>().unwrap().push(pc.entity, None);
            log::info!("conn {conn}: '{}' transfers to zone '{}' via portal", pc.name, portal.target_zone);
        }
    }
}

/// Pick which AOI entries get a position update this snapshot: everything if
/// the crowd fits the budget, else the `nearest` closest entries (by dist²,
/// id-tiebroken) plus a round-robin rotation over the rest. Returns selected
/// indices into `entries` and the advanced cursor. Pure — unit-tested.
fn select_states(entries: &[(u64, f32)], cursor: usize, max: usize, nearest: usize) -> (Vec<usize>, usize) {
    if entries.len() <= max {
        return ((0..entries.len()).collect(), cursor);
    }
    let mut by_dist: Vec<usize> = (0..entries.len()).collect();
    by_dist.sort_by(|&a, &b| {
        entries[a].1.total_cmp(&entries[b].1).then(entries[a].0.cmp(&entries[b].0))
    });
    let mut selected: Vec<usize> = by_dist[..nearest].to_vec();
    let in_nearest: HashSet<usize> = selected.iter().copied().collect();
    // The rotation pool in stable id order, so the cursor sweeps the same
    // sequence between snapshots and every entity refreshes within
    // ceil(pool / budget) snapshots.
    let mut pool: Vec<usize> = (0..entries.len()).filter(|i| !in_nearest.contains(i)).collect();
    pool.sort_by_key(|&i| entries[i].0);
    let budget = max - nearest;
    for k in 0..budget {
        selected.push(pool[(cursor + k) % pool.len()]);
    }
    (selected, cursor + budget)
}

pub struct SnapshotBroadcastSystem {
    /// Per-run scratch, reused across runs: grid candidates, the dedupe set,
    /// and the id set swapped with each conn's `known` (no per-conn realloc).
    aoi_scratch: Vec<Entity>,
    seen: HashSet<Entity>,
    current_ids: HashSet<u64>,
}

impl SnapshotBroadcastSystem {
    pub fn new() -> Self {
        Self { aoi_scratch: Vec::new(), seen: HashSet::new(), current_ids: HashSet::new() }
    }
}

impl System for SnapshotBroadcastSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let (tick, conn_players): (u64, Vec<(ConnId, Entity)>) = {
            let state = resources.get_mut::<NetServerState>().unwrap();
            state.tick += 1;
            // Periodic world-clock re-sync (every ~10 s at POST_HZ).
            if state.tick % 600 == 0 {
                let at_server_micros = state.server.now_micros();
                let world_micros = state.world_at(at_server_micros);
                state.server.broadcast(encode(&ServerMsg::WorldClock { world_micros, at_server_micros }));

                // Periodic net metrics dump — the operational visibility the
                // dead NetMetrics facade claimed but never provided.
                let m = state.server.metrics();
                log::info!(
                    "net metrics: frames_in={} frames_out={} bytes_in={} bytes_out={} rejects={} writer_queue_depth={} busy_micros={}",
                    m.frames_in.load(Ordering::Relaxed),
                    m.frames_out.load(Ordering::Relaxed),
                    m.bytes_in.load(Ordering::Relaxed),
                    m.bytes_out.load(Ordering::Relaxed),
                    m.rejects.load(Ordering::Relaxed),
                    m.writer_queue_depth.load(Ordering::Relaxed),
                    m.busy_micros.load(Ordering::Relaxed),
                );
            }
            // Stagger: only this tick's slice of connections is served — each
            // conn still gets exactly SNAPSHOT_HZ snapshots per second.
            let tick = state.tick;
            let conns = state.conns.iter()
                .filter(|&(&conn, _)| conn % STAGGER == tick % STAGGER)
                .map(|(&conn, pc)| (conn, pc.entity))
                .collect();
            (tick, conns)
        };
        if conn_players.is_empty() {
            return;
        }

        // Per-client AOI: grid cells are coarse and multi-cell entities appear
        // more than once, so dedupe and apply the exact radius test — a fuzzy
        // border would make entities flap in and out between snapshots.
        let mut per_conn: Vec<(ConnId, Vec<(u64, Entity, Vec3, i32, f32)>)> = Vec::with_capacity(conn_players.len());
        {
            let grid = resources.get::<SpatialGrid>().expect("SpatialGrid not in resources");
            // One view for the whole gather: the replication filter (PrefabId),
            // position, and health come from a single lookup per candidate.
            let mut repl_q = world.query::<(&Transform, &PrefabId, Option<&Health>)>();
            let repl_view = repl_q.view();
            for &(conn, player) in &conn_players {
                let Ok(center) = world.get::<&Transform>(player).map(|t| t.position) else { continue };
                self.aoi_scratch.clear();
                grid.query_radius_into(center, AOI_RADIUS, &mut self.aoi_scratch);
                self.seen.clear();
                let mut current: Vec<(u64, Entity, Vec3, i32, f32)> = Vec::with_capacity(self.aoi_scratch.len());
                for &entity in &self.aoi_scratch {
                    if !self.seen.insert(entity) {
                        continue;
                    }
                    let Some((t, _, hp)) = repl_view.get(entity) else { continue };
                    let dist_sq = t.position.distance_squared(center);
                    if dist_sq > AOI_RADIUS * AOI_RADIUS {
                        continue;
                    }
                    let hp = hp.map(|h| h.current).unwrap_or(0);
                    current.push((entity.to_bits().get(), entity, t.position, hp, dist_sq));
                }
                per_conn.push((conn, current));
            }
        }

        let state = resources.get_mut::<NetServerState>().unwrap();
        for (conn, current) in per_conn {
            let Some(pc) = state.conns.get_mut(&conn) else { continue };

            self.current_ids.clear();
            self.current_ids.extend(current.iter().map(|&(id, ..)| id));
            let leaves: Vec<u64> = pc.known.difference(&self.current_ids).copied().collect();
            let enters: Vec<EntityState> = current
                .iter()
                .filter(|(id, ..)| !pc.known.contains(id))
                .filter_map(|&(id, entity, pos, hp, _)| {
                    let prefab = world.get::<&PrefabId>(entity).ok()?.0.clone();
                    Some(EntityState { id, prefab, pos, hp })
                })
                .collect();
            // Crowd throttling: only `states` is budgeted — identity (enters/
            // leaves/known) must track the full AOI or the diff corrupts.
            let entries: Vec<(u64, f32)> = current.iter().map(|&(id, _, _, _, d)| (id, d)).collect();
            let (selected, cursor) = select_states(&entries, pc.rr_cursor, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
            pc.rr_cursor = cursor;
            let states: Vec<EntityPos> = selected
                .into_iter()
                .map(|i| {
                    let (id, _, pos, hp, _) = current[i];
                    EntityPos { id, pos, hp }
                })
                .collect();
            // The old known set becomes next conn's current_ids scratch.
            std::mem::swap(&mut pc.known, &mut self.current_ids);

            let last_processed_seq = pc.applied_seq;
            state.server.send(conn, encode(&ServerMsg::Snapshot {
                tick,
                last_processed_seq,
                enters,
                leaves,
                states,
            }));
        }
    }
}

/// Broadcasts `EntityDied` (v8) for entities whose Health depleted this tick.
/// Phase::DespawnFlush, First — after DeathSystem emitted the event
/// (CollisionResolve) but before the flush removes the entity, so its final
/// position is still readable. Snapshots stop mentioning the entity the same
/// tick; this message is the client's only death signal (corpse + burst).
/// Sent only to connections whose known set contains the entity.
pub struct DeathBroadcastSystem;

impl System for DeathBroadcastSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let deaths: Vec<(u64, Vec3)> = resources
            .get::<EventBus>()
            .map(|bus| {
                bus.read::<HealthDepleted>()
                    .filter_map(|e| {
                        let pos = world.get::<&Transform>(e.entity).ok()?.position;
                        Some((e.entity.to_bits().get(), pos))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if deaths.is_empty() {
            return;
        }
        let state = resources.get_mut::<NetServerState>().unwrap();
        for (id, pos) in deaths {
            let msg = encode(&ServerMsg::EntityDied { id, pos });
            let targets: Vec<ConnId> = state
                .conns
                .iter()
                .filter(|(_, pc)| pc.known.contains(&id))
                .map(|(&conn, _)| conn)
                .collect();
            for conn in targets {
                state.server.send(conn, msg.clone());
            }
        }
    }
}

/// Whether `conn`'s autosave falls due on this PostUpdate `tick` — spreads a
/// crowd's saves across the whole AUTOSAVE_TICKS window (same trick as
/// SnapshotBroadcastSystem's STAGGER, `conn % STAGGER == tick % STAGGER`
/// above) instead of every connection landing on the exact same tick and
/// bursting the DB worker's FIFO request channel, queuing any relogin load
/// behind the whole wave (networking audit 2026-07-11, finding 13).
fn autosave_due(conn: ConnId, tick: u64) -> bool {
    conn % AUTOSAVE_TICKS == tick % AUTOSAVE_TICKS
}

/// Periodic character persistence: over each AUTOSAVE_TICKS window (~30 s),
/// hand each connected player's position + health to the DB worker — one
/// save per connection per window, staggered by `autosave_due` so a crowd's
/// saves don't all land on the same tick. Fire-and-forget — disconnect-save
/// covers the gap on clean exits.
pub struct AutosaveSystem {
    ticks: u64,
}

impl System for AutosaveSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let tick = self.ticks;
        self.ticks += 1;
        let state = resources.get_mut::<NetServerState>().unwrap();
        for (&conn, pc) in &state.conns {
            if !autosave_due(conn, tick) {
                continue;
            }
            if let (Ok(tr), Ok(hp)) = (world.get::<&Transform>(pc.entity), world.get::<&Health>(pc.entity)) {
                state.db.save(pc.name.clone(), state.zone.name.clone(), tr.position, hp.current);
            }
        }
    }
}

/// Benchmark seam (vordar-benches only): exposes just enough of the private
/// snapshot/mechanic machinery to measure it. Sends to the fabricated ConnIds
/// are silently dropped by engine-net's router (no such connection), so the
/// benches measure the full sim-thread cost with zero network I/O.
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod bench {
    use super::*;

    pub const MAX_STATES: usize = MAX_SNAPSHOT_STATES;
    pub const NEAREST: usize = NEAREST_GUARANTEED;
    pub const AOI: f32 = AOI_RADIUS;
    pub const STAGGER_TICKS: u64 = STAGGER;

    pub fn select_states(
        entries: &[(u64, f32)],
        cursor: usize,
        max: usize,
        nearest: usize,
    ) -> (Vec<usize>, usize) {
        super::select_states(entries, cursor, max, nearest)
    }

    /// NetServerState with one PlayerConn per entity, keyed by fabricated
    /// ConnIds 1..=n.
    pub fn state_with_fake_conns(server: NetServer, db: DbHandle, players: &[Entity]) -> NetServerState {
        let zone = ZoneDef { name: "bench".into(), chapter: None, portals: Vec::new(), visuals: Default::default() };
        let directory = HashMap::from([("bench".to_owned(), server.local_addr())]);
        let mut state = NetServerState::new(server, db, None, zone, directory, Instant::now());
        for (i, &entity) in players.iter().enumerate() {
            state.conns.insert(
                (i + 1) as ConnId,
                PlayerConn {
                    entity,
                    name: format!("bench-{i}"),
                    queue: VecDeque::new(),
                    applied_seq: 0,
                    last_seq: 0,
                    last_t: 0,
                    known: HashSet::new(),
                    history: VecDeque::new(),
                    last_cast: HashMap::new(),
                    rr_cursor: 0,
                },
            );
        }
        state
    }

    /// Fill every conn's applied-intent history to HISTORY_CAP with stamps at
    /// `stamp` — mechanic resolution then rewinds the full history per player
    /// target (the worst case) whenever `stamp` exceeds the rewind horizon.
    pub fn fill_histories(state: &mut NetServerState, stamp: u64) {
        for pc in state.conns.values_mut() {
            pc.history.clear();
            for k in 0..HISTORY_CAP {
                pc.history.push_back((stamp + k as u64, Vec2::X));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` entries with id = index and distance growing with the index.
    fn entries(n: usize) -> Vec<(u64, f32)> {
        (0..n).map(|i| (i as u64, i as f32)).collect()
    }

    #[test]
    fn small_crowds_pass_through_untouched() {
        let e = entries(MAX_SNAPSHOT_STATES);
        let (sel, cursor) = select_states(&e, 5, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
        assert_eq!(sel.len(), e.len());
        assert_eq!(cursor, 5);
    }

    #[test]
    fn nearest_always_included_over_budget() {
        let e = entries(200);
        for cursor in [0, 7, 1000] {
            let (sel, _) = select_states(&e, cursor, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
            assert_eq!(sel.len(), MAX_SNAPSHOT_STATES);
            for i in 0..NEAREST_GUARANTEED {
                assert!(sel.contains(&i), "nearest entry {i} missing at cursor {cursor}");
            }
        }
    }

    #[test]
    fn rotation_refreshes_every_entity() {
        let e = entries(200);
        let pool = e.len() - NEAREST_GUARANTEED; // 168
        let budget = MAX_SNAPSHOT_STATES - NEAREST_GUARANTEED; // 32
        let rounds = pool.div_ceil(budget); // ceil(168/32) = 6
        let mut cursor = 0;
        let mut seen: HashSet<usize> = HashSet::new();
        for _ in 0..rounds {
            let (sel, next) = select_states(&e, cursor, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
            seen.extend(sel);
            cursor = next;
        }
        // Every entry got at least one position update within the window.
        assert_eq!(seen.len(), e.len());
    }

    #[test]
    fn no_duplicate_indices_in_selection() {
        let e = entries(70); // barely over budget: pool of 38, budget 32
        let (sel, _) = select_states(&e, 31, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
        let unique: HashSet<usize> = sel.iter().copied().collect();
        assert_eq!(unique.len(), sel.len());
    }

    /// Regression test for the seq=0 replay wedge (networking audit
    /// 2026-07-11, finding 16). `last_seq: 0` is the connection's "nothing
    /// received yet" sentinel; before this fix, `pc.last_seq != 0 && seq <=
    /// pc.last_seq` skipped the monotonicity check entirely whenever seq was
    /// 0, so a spoofed/replayed seq=0 intent passed validation every single
    /// time instead of ever advancing past the sentinel. A genuine client's
    /// own seq counter starts at 1, so seq=0 should never be a legitimate
    /// value on the wire.
    #[test]
    fn zero_seq_is_always_rejected() {
        let mut world = World::new();
        let entity = world.spawn(());
        let pc = PlayerConn {
            entity,
            name: "victim".into(),
            queue: VecDeque::new(),
            applied_seq: 0,
            last_seq: 0,
            last_t: 0,
            known: HashSet::new(),
            history: VecDeque::new(),
            last_cast: HashMap::new(),
            rr_cursor: 0,
        };
        // Otherwise-well-formed intent (monotonic t, arrives on time) —
        // the only thing wrong with it is seq == 0.
        let result = validate_intent(&pc, 0, 1_000, 1_000, 0);
        assert_eq!(result, Err("stale seq"), "seq=0 must never pass validation");
    }

    /// Regression test for the autosave burst (networking audit
    /// 2026-07-11, finding 13). Before this fix, `AutosaveSystem` gated on
    /// the global tick alone (`self.ticks % AUTOSAVE_TICKS != 0`), so every
    /// connected player was handed to the DB worker on the exact same tick —
    /// a crowd's worth of saves bursting the FIFO request channel and
    /// queuing any relogin load behind the whole wave. `autosave_due` must
    /// give each connection exactly one due tick per window, but spread a
    /// crowd's due ticks across more than one tick of the window.
    #[test]
    fn autosave_spreads_a_crowd_across_the_window_instead_of_bursting() {
        let conns: Vec<ConnId> = (1..=50).collect();
        let mut due_ticks: HashSet<u64> = HashSet::new();
        let mut due_count: HashMap<ConnId, u32> = HashMap::new();
        for tick in 0..AUTOSAVE_TICKS {
            for &conn in &conns {
                if autosave_due(conn, tick) {
                    due_ticks.insert(tick);
                    *due_count.entry(conn).or_insert(0) += 1;
                }
            }
        }
        // Every connection autosaves exactly once per window.
        for &conn in &conns {
            assert_eq!(due_count.get(&conn).copied().unwrap_or(0), 1, "conn {conn} did not save exactly once");
        }
        // The 50-strong crowd's saves land on more than one tick — not a
        // single-tick burst.
        assert!(due_ticks.len() > 1, "all autosaves landed on the same tick: {due_ticks:?}");
    }
}
