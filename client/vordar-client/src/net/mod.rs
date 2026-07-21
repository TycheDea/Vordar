// Networked-client plugin — replicates the server's world into this one.
//
// Remote entities are server-driven — each carries a
// tick-indexed sample buffer (NetBuffer) and is rendered by a playback
// cursor a fixed ~200 ms behind the newest received snapshot tick
// (NetInterpolateSystem), absorbing jitter and single-datagram loss without
// freezing or warbling. Our OWN player is
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
use engine_core::components::{Anchored, CollisionShape, Health, Hitbox, Solid, Transform};
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use engine_net::NetClient;
use glam::{Vec2, Vec3};
use hecs::Entity;
use interpolate::{NetBuffer, NetInterpolateSystem};
pub use interpolate::NetMotion;
use lifecycle::{reconnect_backoff, NetReceiveSystem, Reconnect};
use prediction::{
    reconcile_own, NetCorrectionSystem, NetSendInputSystem, PendingIntent, PredictedStaticCollisionSystem,
};
pub(crate) use prediction::start_predicted_leap;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_game::events::MoveIntent;
use vordar_game::motion::{anchored_push, predict_step, MovementSystem, PlayRadius};
use vordar_game::player::{movement_velocity, PlayerMovementSystem};
use vordar_game::Player;
use vordar_protocol::{
    encode, AccountToken, ClientMsg, EntityPos, EntityState, MoveIntentEntry, PROTOCOL_VERSION,
    SNAPSHOT_HZ, TICK_HZ,
};


pub struct NetClientPlugin {
    pub server_addr: SocketAddr,
    /// Predict own movement locally; off reproduces the server-driven feel
    /// (one round-trip of input latency) for comparison.
    pub predict: bool,
    /// Artificial round-trip latency added by engine-net (testing knob).
    pub simulated_rtt: Duration,
    /// Character name sent as the first message after connect.
    pub user: String,
    /// Account credential presented with `user` on every `Login` — see
    /// `credentials::load_or_mint`.
    pub token: AccountToken,
}

impl Plugin for NetClientPlugin {
    fn build(&self, app: &mut App) {
        // A failed first connect falls back to the same reconnect state
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
        .add_plugin(crate::PresentationPlugin)
        .add_system(NetInterpolateSystem, Phase::Update, SystemOrder::First)
        .add_system(crate::NetCameraFollowSystem, Phase::RenderSync, SystemOrder::First)
        .add_system(crate::telegraph::TelegraphFillSystem, Phase::RenderSync, SystemOrder::First);
        if self.predict {
            // The shared simulation moves our own player. Remote players stay
            // playback-driven (NetInterpolateSystem): no intents are emitted
            // for them, so the shared systems hold their velocity at zero.
            // LeapSystem mirrors the server's dash override so an Onslaught
            // moves the own view immediately instead of waiting a round-trip.
            app.add_system(PlayerMovementSystem, Phase::Update, SystemOrder::First)
                .add_system(vordar_game::combat::leap::LeapSystem, Phase::Update, SystemOrder::Default)
                .add_system(MovementSystem, Phase::Update, SystemOrder::Last)
                .add_system(NetCorrectionSystem, Phase::Update, SystemOrder::Last)
                // Must be SystemOrder::Last, not after::<NetCorrectionSystem>(): Last
                // carries an implicit "after every non-Last system" edge, so a
                // Default-tier After pointed at a Last system is a contradiction the
                // scheduler rejects as a cycle. Two Last peers resolve by registration
                // order instead, which this call chain already places correctly.
                .add_system(PredictedStaticCollisionSystem, Phase::Update, SystemOrder::Last);
        }
    }
}

pub struct NetClientState {
    /// None while the connection is down (initial connect failure, or an
    /// unexpected drop awaiting a redial).
    client: Option<NetClient>,
    /// Address to redial after an unexpected disconnect. A zone Redirect
    /// overwrites this with the new zone's address.
    server_addr: SocketAddr,
    user: String,
    /// Account credential presented on every `Login`.
    token: AccountToken,
    /// Set once a `LoginDenied` arrives — stops `handle_disconnected` and
    /// `maybe_reconnect` from scheduling further redials: retrying with the
    /// same bad credential would only be denied again.
    login_denied: bool,
    own_id: Option<u32>,
    /// server entity id → local entity
    entities: HashMap<u32, Entity>,
    /// This zone's prefab name table: index = the `u16`
    /// `EntityState::prefab` rides on the wire.
    /// Empty until `ServerMsg::PrefabTable` arrives (right after `Welcome`,
    /// before the first `Snapshot`); cleared on teardown so a redirect or
    /// reconnect adopts the new zone's table instead of the old one's.
    prefab_names: Vec<String>,
    seq: u32,
    predict: bool,
    pending: VecDeque<PendingIntent>,
    /// Last `MOVE_RING_LEN` sent `MoveIntentEntry`s, oldest first — resent
    /// every tick as the `ClientMsg::MoveIntents` batch. Cleared in
    /// `teardown_replicated_world` alongside `seq` so a redirect/reconnect
    /// starts a fresh window instead of resending the old connection's seqs.
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
    /// unreliable datagram, so a copy can arrive late or out of order; any
    /// snapshot whose tick is not strictly greater is dropped before any
    /// field is read (ack included).
    /// Reset in `teardown_replicated_world` so a redirect/reconnect doesn't
    /// compare against the old zone's ticks.
    latest_state_tick: u64,
    /// The render playback cursor, in server-tick units — `None` until the
    /// first tick it's driven, which hard-snaps it to
    /// `latest_state_tick as f64 - INTERP_DELAY_TICKS` instead of slewing
    /// from an arbitrary start. Reset to `None` in `teardown_replicated_world`
    /// alongside `latest_state_tick`.
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
/// (or offline — no NetClientState at all).
pub(crate) fn reconnect_attempt(resources: &Resources) -> Option<u32> {
    resources.get::<NetClientState>().and_then(|s| s.reconnect.as_ref().map(|r| r.attempt))
}



/// Benchmark seam (vordar-benches only): exposes the private snapshot-apply /
/// reconciliation machinery so the client hot path is measurable headless.
/// The NetClientState's socket points at an unroutable address — nothing the
/// benches call ever touches the network.
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod bench;

#[cfg(test)]
mod e2e;
