// Networked-client plugin — replicates the server's world into this one.
//
// Phase 2 model: remote entities are server-driven — each carries a
// tick-indexed sample buffer (NetBuffer) and is rendered by a playback
// cursor a fixed ~200 ms behind the newest received snapshot tick
// (NetInterpolateSystem), absorbing jitter and single-datagram loss without
// freezing or warbling (networking rework 4, finding 1). Our OWN player is
// predicted: each Input tick we send the intent AND emit it locally, so the
// shared vordar-game movement systems apply it immediately. Snapshots then
// reconcile: rebase onto the server's authoritative position and replay the
// intents the server hasn't processed yet (`last_processed_seq`). Both
// phases run Fixed(60), so one sent intent maps 1:1 to one local integration
// step.

mod apply;
mod interpolate;
mod lifecycle;
mod prediction;

use crate::read_move_dir;
use engine_app::app::App;
use engine_app::events::EventBus;
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::components::{Health, Transform};
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use engine_net::NetClient;
use glam::{Vec2, Vec3};
use hecs::Entity;
use interpolate::{NetBuffer, NetInterpolateSystem};
use lifecycle::{reconnect_backoff, NetReceiveSystem, Reconnect};
use prediction::{reconcile_own, NetCorrectionSystem, NetSendInputSystem, PendingIntent};
pub(crate) use prediction::start_predicted_leap;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_game::events::MoveIntent;
use vordar_game::motion::MovementSystem;
use vordar_game::player::{movement_velocity, PlayerMovementSystem};
use vordar_game::Player;
use vordar_protocol::{
    encode, AccountToken, ClientMsg, EntityPos, EntityState, MoveIntentEntry, PROTOCOL_VERSION,
    SNAPSHOT_HZ, TICK_HZ,
};


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
        let mut state =
            NetClientState::new(client, self.server_addr, self.user.clone(), self.token, self.predict, self.simulated_rtt);
        state.reconnect = reconnect;
        app.insert_resource(state)
        .add_system(NetReceiveSystem, Phase::Input, SystemOrder::Default)
        .add_system(NetSendInputSystem, Phase::Input, SystemOrder::after::<NetReceiveSystem>())
        .add_system(crate::cast::AbilityCastSystem::new(), Phase::Input, SystemOrder::after::<NetSendInputSystem>())
        .insert_resource(crate::world_time::WorldTime { offset_micros: 0, synced: false })
        .insert_resource(crate::CastState::new())
        .insert_resource(crate::presentation::CurrentZone("start".into()))
        .insert_resource(vordar_game::zones::load_zones("content/zones/zones.ron"))
        .insert_resource(crate::vfx::ParticleSim::new())
        .add_system(NetInterpolateSystem, Phase::Update, SystemOrder::First)
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
        .add_system(crate::NetCameraFollowSystem, Phase::RenderSync, SystemOrder::First)
        .add_system(crate::telegraph::TelegraphFillSystem, Phase::RenderSync, SystemOrder::First)
        .add_system(crate::world_time::DayNightSystem, Phase::RenderSync, SystemOrder::First);
        crate::ui::install(app);
        if self.predict {
            // The shared simulation moves our own player. Remote players stay
            // playback-driven (NetInterpolateSystem): no intents are emitted
            // for them, so the shared systems hold their velocity at zero.
            // LeapSystem mirrors the server's dash override so an Onslaught
            // moves the own view immediately instead of waiting a round-trip.
            app.add_system(PlayerMovementSystem, Phase::Update, SystemOrder::First)
                .add_system(vordar_game::combat::leap::LeapSystem, Phase::Update, SystemOrder::Default)
                .add_system(MovementSystem, Phase::Update, SystemOrder::Last)
                .add_system(NetCorrectionSystem, Phase::Update, SystemOrder::Last);
        }
    }
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
    /// This zone's prefab name table (protocol v13, networking rework 5
    /// finding 4): index = the `u16` `EntityState::prefab` rides on the wire.
    /// Empty until `ServerMsg::PrefabTable` arrives (right after `Welcome`,
    /// before the first `Snapshot`); cleared on teardown so a redirect or
    /// reconnect adopts the new zone's table instead of the old one's.
    prefab_names: Vec<String>,
    seq: u32,
    predict: bool,
    pending: VecDeque<PendingIntent>,
    /// Last `MOVE_RING_LEN` sent `MoveIntentEntry`s, oldest first (protocol
    /// v15, networking rework 3 finding 5) — resent every tick as the
    /// `ClientMsg::MoveIntents` batch. Cleared in `teardown_replicated_world`
    /// alongside `seq` so a redirect/reconnect starts a fresh window instead
    /// of resending the old connection's seqs.
    move_ring: VecDeque<MoveIntentEntry>,
    /// Outstanding reconciliation error, folded into the predicted position a
    /// little each Update tick by NetCorrectionSystem.
    correction: Vec3,
    /// Kept so a zone Redirect (or a reconnect) redials with the same
    /// latency knob.
    simulated_rtt: Duration,
    /// Set while disconnected and a redial is scheduled/in flight; read by
    /// the UI to show a "reconnecting" indicator (`reconnect_attempt`).
    reconnect: Option<Reconnect>,
    /// Highest `ServerMsg::Snapshot.tick` applied so far. `Snapshot` rides an
    /// unreliable datagram (protocol v14, networking rework 3 finding 4), so
    /// a copy can arrive late or out of order; any snapshot whose tick is not
    /// strictly greater is dropped before any field is read (ack included).
    /// Reset in `teardown_replicated_world` so a redirect/reconnect doesn't
    /// compare against the old zone's ticks.
    latest_state_tick: u64,
    /// The render playback cursor, in server-tick units — `None` until the
    /// first tick it's driven, which hard-snaps it to
    /// `latest_state_tick as f64 - INTERP_DELAY_TICKS` instead of slewing
    /// from an arbitrary start. Reset to `None` in `teardown_replicated_world`
    /// alongside `latest_state_tick` (networking rework 4, finding 1).
    playback: Option<f64>,
}

impl NetClientState {
    /// Builds a fresh session-state struct with every bookkeeping field at
    /// its connect-time default; callers set the few fields they care about
    /// on the returned value (all callers are inside `net`, so private-field
    /// assignment compiles).
    pub(crate) fn new(
        client: Option<NetClient>,
        server_addr: SocketAddr,
        user: String,
        token: AccountToken,
        predict: bool,
        simulated_rtt: Duration,
    ) -> Self {
        NetClientState {
            client,
            server_addr,
            user,
            token,
            login_denied: false,
            own_id: None,
            entities: HashMap::new(),
            prefab_names: Vec::new(),
            seq: 0,
            predict,
            pending: VecDeque::new(),
            move_ring: VecDeque::new(),
            correction: Vec3::ZERO,
            simulated_rtt,
            reconnect: None,
            latest_state_tick: 0,
            playback: None,
        }
    }

    fn own_entity(&self) -> Option<Entity> {
        self.own_id.and_then(|id| self.entities.get(&id).copied())
    }

    /// The synced server clock, if the connection has one — `None` while
    /// disconnected or before the handshake's clock sync completes.
    pub(crate) fn server_now_micros(&self) -> Option<u64> {
        self.client.as_ref().and_then(|c| c.server_now_micros())
    }

    /// Whether our own player is locally predicted (vs. server-driven).
    pub(crate) fn predicting(&self) -> bool {
        self.predict
    }

    /// Stamps, seqs, and sends a `ClientMsg::CastIntent` for `skill` at
    /// `target`. Returns false (no send, no seq bump) if the clock isn't
    /// synced yet — same gate `NetSendInputSystem` uses for movement intents.
    pub(crate) fn send_cast_intent(&mut self, skill: String, target: Vec2) -> bool {
        let Some(t_server_micros) = self.server_now_micros() else { return false };
        self.seq += 1;
        if let Some(client) = &self.client {
            client.send(encode(&ClientMsg::CastIntent { seq: self.seq, t_server_micros, skill, target }));
        }
        true
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
        let mut state = NetClientState::new(
            Some(NetClient::connect(server_addr, PROTOCOL_VERSION).expect("bench NetClient")),
            server_addr,
            "bench".into(),
            [0u8; 32],
            predict,
            Duration::ZERO,
        );
        state.own_id = own_id;
        state
    }

    /// server-id → local-entity mapping (the enters path builds this normally).
    pub fn map_entity(state: &mut NetClientState, id: u32, entity: Entity) {
        state.entities.insert(id, entity);
    }

    /// Seeds the client's cached prefab name table directly — bypasses the
    /// `ServerMsg::PrefabTable` wire round trip so benches can build `enters`
    /// with `u16` refs against a known table (protocol v13, networking
    /// rework 5 finding 4).
    pub fn set_prefab_table(state: &mut NetClientState, names: Vec<String>) {
        state.prefab_names = names;
    }

    pub fn push_pending(state: &mut NetClientState, seq: u32, dir: Vec2, dt: f32) {
        state.pending.push_back(PendingIntent { seq, dir, dt, leap: None });
    }

    pub fn apply_aoi_delta(
        world: &mut World,
        resources: &mut Resources,
        tick: u64,
        enters: Vec<EntityState>,
        leaves: Vec<u32>,
    ) {
        super::apply::apply_aoi_delta(world, resources, tick, enters, leaves);
    }

    pub fn apply_states(
        world: &mut World,
        resources: &mut Resources,
        tick: u64,
        last_processed_seq: u32,
        states: Vec<EntityPos>,
    ) {
        super::apply::apply_states(world, resources, tick, last_processed_seq, states);
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
    use crate::world_time::WorldTime;
    use engine_app::time::Time;
    use engine_net::{ClientEvent, NetClient};
    use prediction::{MOVE_RING_LEN, SNAP_DISTANCE};
    use vordar_protocol::{decode, ServerMsg};

    const DT: f32 = 1.0 / 60.0;

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
        resources.insert(NetClientState::new(
            Some(NetClient::connect(addr, PROTOCOL_VERSION).expect("victim connect")),
            addr,
            "reconnect-victim".into(),
            name_token("reconnect-victim"),
            false,
            Duration::ZERO,
        ));

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
        resources.insert(NetClientState::new(
            Some(
                NetClient::connect_with_latency(addr, PROTOCOL_VERSION, Duration::from_millis(150))
                    .expect("dasher connect"),
            ),
            addr,
            "onslaught-dasher".into(),
            name_token("onslaught-dasher"),
            true,
            Duration::from_millis(150),
        ));

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

        // Finding 1 of docs/reviews/networking/plan-networking-rework-1-2026-07-13.md:
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
        assert!(resources.get_mut::<NetClientState>().unwrap().send_cast_intent("onslaught".into(), cast_target));
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

    /// Nearest-rank percentile helper for the smoothness probe below — mirrors
    /// `server/vordar-server/tests/loss.rs`'s `pct`.
    fn pct(sorted: &[f32], p: f64) -> f32 {
        sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)]
    }

    /// Sleeps until the next precise 60 Hz tick boundary, then advances
    /// `next` by exactly one tick. The connection-wait loops elsewhere in
    /// this file pace themselves with a flat 16 ms sleep, which drifts ~4 %
    /// fast against the true 16.667 ms tick — harmless for a coarse "did the
    /// Welcome arrive yet" wait, but enough drift for an extra render call to
    /// occasionally land inside the smoothness probe's degenerate
    /// reversal-cancellation window (see `mover_tick`'s doc comment),
    /// spuriously inflating its measured zero-motion run by one tick.
    fn pace_tick(next: &mut Instant) {
        *next += Duration::from_secs_f64(1.0 / TICK_HZ as f64);
        let now = Instant::now();
        if *next > now {
            std::thread::sleep(*next - now);
        } else {
            *next = now; // fell behind — resync instead of trying to catch up
        }
    }

    /// Sends the mover's next `ClientMsg::MoveIntents` datagram — the same
    /// last-3 redundancy ring `NetSendInputSystem` keeps (net.rs:1203-1212) —
    /// and reverses `dir`'s X sign every ~2 s so the mover walks back and
    /// forth instead of leaving the observer's AOI (networking rework 4,
    /// finding 3's smoothness probe). 2170 ms, not a whole multiple of the
    /// 100 ms `SNAPSHOT_HZ` cadence, so a reversal's phase against the sample
    /// boundaries drifts instead of relocking to the same offset every cycle.
    /// `dir`'s Z component is a small constant (never reversed): a pure ±X
    /// flip can make one buffered sample nearly *identical* to its neighbor
    /// whenever the flip lands near a sample's midpoint (out, then most of
    /// the way back, within one 100 ms interval) — a real linear-
    /// interpolation artifact, not a freeze regression, but one that (when it
    /// coincides with a lost/late next sample) can extrapolate from that
    /// near-zero velocity for longer than this probe's zero-motion gate
    /// allows. The steady Z drift guarantees no two samples are ever exactly
    /// equal, so the interpolated segment is always genuinely moving.
    fn mover_tick(
        client: &NetClient,
        seq: &mut u32,
        ring: &mut VecDeque<MoveIntentEntry>,
        dir: &mut Vec2,
        last_reverse: &mut Instant,
    ) {
        const REVERSE_INTERVAL: Duration = Duration::from_millis(2170);
        if last_reverse.elapsed() >= REVERSE_INTERVAL {
            dir.x = -dir.x;
            *last_reverse = Instant::now();
        }
        let Some(t_server_micros) = client.server_now_micros() else { return };
        *seq += 1;
        ring.push_back(MoveIntentEntry { seq: *seq, t_server_micros, dir: *dir });
        if ring.len() > MOVE_RING_LEN {
            ring.pop_front();
        }
        let intents: Vec<MoveIntentEntry> = ring.iter().cloned().collect();
        client.send_datagram(encode(&ClientMsg::MoveIntents { intents }));
    }

    /// Networking rework 4, finding 3
    /// (`docs/reviews/networking/plan-networking-rework-4-2026-07-14.md`): the loss
    /// probes (`server/vordar-server/tests/loss.rs`) measure arrival gaps and
    /// intent-ack lag only — nothing measures what a player actually SEES: the
    /// per-tick rendered motion of a remote entity under loss and jitter. A
    /// real headless server, a real "mover" (a second raw `NetClient`, the
    /// kicker pattern above) streaming `MoveIntents` datagrams ±X at 6 u/s,
    /// and a real WAN-impaired "observer" running the actual client systems
    /// (`NetReceiveSystem` + `NetInterpolateSystem`, `predict: false` — a
    /// non-predicting own player is buffered like any remote) prove the
    /// fixed-delay playback buffer (findings 1-2) keeps the rendered path
    /// continuous instead of freezing/warbling at every late or lost
    /// snapshot. Permanent regression gates, run like the loss probes:
    /// `cargo test -p vordar-client --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "loss probe — run with --release --ignored --nocapture"]
    fn remote_render_smoothness_under_loss_probe() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::env::set_current_dir(&root).unwrap();
        if cfg!(debug_assertions) {
            eprintln!("WARNING: loss probe running in debug — results will not be representative");
        }

        const SPEED: f32 = 6.0;
        const WINDOW: Duration = Duration::from_secs(20);
        const SETTLE: Duration = Duration::from_secs(2);

        let addr: SocketAddr = "127.0.0.1:25404".parse().unwrap();
        std::thread::spawn(move || {
            vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(60 * 60));
        });
        std::thread::sleep(Duration::from_millis(300));

        // The mover: a second raw NetClient (the kicker pattern above) that
        // logs in and streams MoveIntents — unimpaired; only the observer's
        // connection below carries the WAN impairment.
        let mut mover = NetClient::connect(addr, PROTOCOL_VERSION).expect("mover connect");
        let mut mover_seq = 0u32;
        let mut mover_ring: VecDeque<MoveIntentEntry> = VecDeque::new();
        // Small constant Z drift alongside the ±X reversal — see
        // `mover_tick`'s doc comment for why this is needed.
        let mut mover_dir = Vec2::new(1.0, 0.1).normalize();
        let mut last_reverse = Instant::now();
        let mover_id = {
            let mut id = None;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                for ev in mover.poll() {
                    match ev {
                        ClientEvent::Connected => {
                            mover.send(encode(&ClientMsg::Login {
                                name: "smoothness-mover".into(),
                                token: name_token("smoothness-mover"),
                            }));
                        }
                        ClientEvent::Message(data) => {
                            if let Some(ServerMsg::Welcome { player_id }) = decode::<ServerMsg>(&data) {
                                id = Some(player_id);
                            }
                        }
                        _ => {}
                    }
                }
                if id.is_some() {
                    break;
                }
                assert!(Instant::now() < deadline, "mover never got its Welcome");
                std::thread::sleep(Duration::from_millis(16));
            }
            id.unwrap()
        };

        // The observer: the onslaught test's world verbatim (prefab registry
        // + real NetReceiveSystem/NetInterpolateSystem), connected WAN-
        // impaired (100 ms RTT, 30 ms jitter, 3 % downstream loss).
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
        resources.insert(NetClientState::new(
            Some(
                NetClient::connect_impaired(addr, PROTOCOL_VERSION, engine_net::Impairment {
                    rtt: Duration::from_millis(100),
                    jitter: Duration::from_millis(30),
                    downstream_loss: 0.03,
                    ..Default::default()
                })
                .expect("observer connect"),
            ),
            addr,
            "smoothness-observer".into(),
            name_token("smoothness-observer"),
            false,
            Duration::from_millis(100),
        ));

        let mut recv = NetReceiveSystem;
        let mut render_sys = NetInterpolateSystem;
        // Precise 60 Hz pacing (see `pace_tick`) — shared across every loop
        // below so the whole run stays on one continuous tick boundary
        // instead of drifting at each stage transition.
        let mut next_tick = Instant::now();

        // Welcome + clock sync, same wait the onslaught test uses.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            recv.run(&mut world, &mut resources, DT);
            let ready = {
                let state = resources.get::<NetClientState>().unwrap();
                state.own_id.is_some() && state.client.as_ref().unwrap().server_now_micros().is_some()
            };
            if ready {
                break;
            }
            assert!(Instant::now() < deadline, "observer never got Welcome + clock sync");
            pace_tick(&mut next_tick);
        }

        // Wait for the mover's own entity to enter the observer's AOI.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mover_entity = loop {
            mover_tick(&mover, &mut mover_seq, &mut mover_ring, &mut mover_dir, &mut last_reverse);
            let _ = mover.poll();
            recv.run(&mut world, &mut resources, DT);
            render_sys.run(&mut world, &mut resources, DT);
            if let Some(&e) = resources.get::<NetClientState>().unwrap().entities.get(&mover_id) {
                break e;
            }
            assert!(Instant::now() < deadline, "observer never saw the mover's entity (id {mover_id}) in its AOI");
            pace_tick(&mut next_tick);
        };

        // Let the buffer/playback cursor lock onto its steady-state delay
        // before recording (mirrors loss.rs's `common::settle`).
        let settle_deadline = Instant::now() + SETTLE;
        while Instant::now() < settle_deadline {
            mover_tick(&mover, &mut mover_seq, &mut mover_ring, &mut mover_dir, &mut last_reverse);
            let _ = mover.poll();
            recv.run(&mut world, &mut resources, DT);
            render_sys.run(&mut world, &mut resources, DT);
            pace_tick(&mut next_tick);
        }

        // The measurement window: record the mover entity's rendered
        // Transform.position after every Update tick.
        let mut prev_pos = world.get::<&Transform>(mover_entity).unwrap().position;
        let mut steps: Vec<f32> = Vec::new();
        let mut zero_run = 0u32;
        let mut max_zero_run = 0u32;
        let window_deadline = Instant::now() + WINDOW;
        while Instant::now() < window_deadline {
            mover_tick(&mover, &mut mover_seq, &mut mover_ring, &mut mover_dir, &mut last_reverse);
            let _ = mover.poll();
            recv.run(&mut world, &mut resources, DT);
            render_sys.run(&mut world, &mut resources, DT);

            let pos = world.get::<&Transform>(mover_entity).unwrap().position;
            let step = (pos - prev_pos).length();
            steps.push(step);
            if step < 1e-4 {
                zero_run += 1;
                max_zero_run = max_zero_run.max(zero_run);
            } else {
                zero_run = 0;
            }
            prev_pos = pos;

            pace_tick(&mut next_tick);
        }

        assert!(steps.len() > 500, "smoothness probe only recorded {} ticks — window too short", steps.len());
        let nominal = SPEED / 60.0;
        steps.sort_by(|a, b| a.total_cmp(b));
        let p50 = pct(&steps, 0.50);
        let p99 = pct(&steps, 0.99);
        let max = *steps.last().unwrap();
        println!(
            "remote render smoothness: ticks={} step_u p50={:.4} p99={:.4} max={:.4} longest_zero_run={}",
            steps.len(),
            p50,
            p99,
            max,
            max_zero_run
        );
        // Permanent regression gates (networking rework 4, finding 3): the
        // pre-rework-4 client froze 10-18 ticks at every late/lost snapshot
        // and then caught up at ~2x steps — both margins are >=2x here.
        assert!(
            max_zero_run <= 5,
            "longest zero-motion run {max_zero_run} ticks exceeds the 5-tick (~83 ms) freeze gate"
        );
        assert!(
            p99 <= 1.5 * nominal,
            "p99 per-tick step {p99:.4} exceeds 1.5x nominal ({:.4}) — catch-up warble regression",
            1.5 * nominal
        );
    }
}
