// NetServerPlugin — the seam between engine-net and the simulation.
//
// Module family: receive = Input edge, broadcast/mechanics/transfer/autosave = PostUpdate,
// shutdown = Input; state lives here.

use crate::db::{CharacterRecord, DbHandle, DbWorker};
use engine_app::app::App;
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, SystemOrder};
use engine_core::components::{Health, Transform};
use engine_core::World;
use engine_net::{ConnId, NetLimits, NetMetrics, NetServer};
use glam::{Vec2, Vec3};
use hecs::Entity;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use vordar_game::combat::buff::RavagerRageSystem;
use vordar_game::world::WorldTime;
use vordar_game::zones::ZoneDef;
use vordar_protocol::{AccountToken, PROTOCOL_VERSION, SNAPSHOT_HZ, TICK_HZ};

mod autosave;
mod broadcast;
mod login;
mod mechanics;
mod receive;
mod repl_ids;
mod shutdown;
mod transfer;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod bench;

pub use shutdown::ShutdownFlag;

use autosave::AutosaveSystem;
use broadcast::{DeathBroadcastSystem, SnapshotBroadcastSystem};
use login::LoginFailures;
use mechanics::MechanicResolveSystem;
use receive::{NetReceiveSystem, XpCarrySystem};
use repl_ids::ReplIds;
use shutdown::ShutdownSystem;
use transfer::ZoneTransferSystem;

/// Lag-compensation rewind cap (DESIGN.md §3): high-latency players get
/// degraded forgiveness, not infinite rewind.
const MAX_REWIND_MICROS: u64 = 200_000;
/// Area-of-interest radius around each player: only entities inside it are
/// replicated to that client. Comfortably beyond the camera's view.
const AOI_RADIUS: f32 = 40.0;
/// Applied-intent history kept per connection for mechanic-resolve rewind
/// (~530 ms at 60 Hz — covers MAX_REWIND plus resolve-tick slack).
const HISTORY_CAP: usize = 32;
/// PostUpdate runs at the sim rate; the 10 Hz systems below self-gate on it.
/// Defined from `vordar_protocol::TICK_HZ`: the client's playback cursor
/// treats that constant as the rate ticks advance at, so the two must never
/// drift apart.
const POST_HZ: f32 = TICK_HZ;
/// Snapshot stagger: each connection is served every STAGGER-th PostUpdate
/// run (still SNAPSHOT_HZ per client) — the fan-out cost splits into STAGGER
/// slices instead of landing on one tick.
const STAGGER: u64 = (POST_HZ / SNAPSHOT_HZ) as u64;

pub struct NetServerPlugin {
    pub addr: SocketAddr,
    /// SQLite path for character persistence (`:memory:` for throwaway).
    pub db_path: String,
    /// Bind-time connection-cap configuration: `NetLimits::default()` for
    /// production; a soak harness modeling many distinct clients from one
    /// source IP raises `max_connections_per_ip` here instead.
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
        .insert_resource(WorldTime(0))
        .add_system(NetReceiveSystem, Phase::Input, SystemOrder::Default)
        // Unconditional: no-ops wherever `ShutdownFlag` is absent (every
        // existing test/bench) or the flag hasn't been flipped.
        .add_system(ShutdownSystem, Phase::Input, SystemOrder::Default)
        // Deaths broadcast before the flush removes the dying entity.
        .add_system(DeathBroadcastSystem, Phase::DespawnFlush, SystemOrder::First)
        // Same pre-flush window: capture a dying player's Xp before the body
        // is removed, so the respawn below can carry it to the new one.
        .add_system(XpCarrySystem, Phase::DespawnFlush, SystemOrder::First)
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
    /// Character name — the persistence key.
    name: String,
    /// The account token this session logged in with: compared synchronously
    /// against a same-name login's presented token to gate session takeover —
    /// a mismatch denies the new connection without touching this one, no DB
    /// roundtrip needed.
    token: AccountToken,
    /// Validated intents as (seq, client stamp, dir) in arrival order, applied
    /// ONE PER TICK. The client emits exactly one intent per fixed Input tick,
    /// so consuming one per fixed server tick integrates the same dirs over
    /// the same steps — the client's prediction replay matches bit-for-bit.
    queue: VecDeque<(u32, u64, Vec2)>,
    /// Seq of the last intent APPLIED to the simulation — the snapshot ack.
    applied_seq: u32,
    /// Seq/stamp of the last MOVE intent RECEIVED — validation monotonicity
    /// for the movement lane only. Casts ride their own pair below: the two
    /// lanes take separate routes (ordered stream vs. unreliable datagram),
    /// so a shared counter lets a cast that overtakes an in-flight move
    /// invalidate that move permanently.
    last_seq: u32,
    last_t: u64,
    /// Seq/stamp of the last CAST intent RECEIVED — the cast lane's own
    /// monotonicity pair, which is what keeps a replayed cast a free reject.
    cast_seq: u32,
    cast_t: u64,
    /// Entity ids currently inside this client's AOI — diffed each snapshot
    /// to produce enter/leave messages.
    known: HashSet<u32>,
    /// Recently APPLIED intents as (client stamp, integrated velocity) — each
    /// entry is exactly one tick of integration, recording the velocity that
    /// actually moved the player that tick (a LeapImpulse override, not the
    /// WASD dir, during a dash). Mechanic resolution rewinds through these to
    /// evaluate "position at T" by stamp time (favor-the-defender).
    history: VecDeque<(u64, Vec3)>,
    /// Server time each skill is next castable (cooldown enforcement) — a
    /// fast skill must not eat a slow skill's cooldown. Persisted as a
    /// remainder (`ready_at − now`) on every save and restored as
    /// `spawn_now + remaining` on load, so a relog or zone transfer
    /// preserves the exact remaining cooldown instead of resetting it.
    cooldown_ready: HashMap<String, u64>,
    /// Round-robin cursor for snapshot `states` throttling — where the
    /// non-nearest rotation resumes next snapshot.
    rr_cursor: usize,
    /// The XP value to seed onto the next body this connection spawns —
    /// updated in the pre-flush death window (`XpCarrySystem`) so a body
    /// death never launders away the player's progression.
    carried_xp: u32,
}

type PrefabTable = (Arc<Vec<String>>, HashMap<String, u16>);

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
    /// Connections past the handshake that have not yet logged in: conn →
    /// server time of `Connected`. The only budget on pre-login slot-holding
    /// — the transport idle timeout resets on any traffic (including
    /// keepalives), so without this a connection that never sends `Login`
    /// would hold its slot indefinitely. Cleared on a successful login
    /// (entry moves to `loading`) or on disconnect.
    pending: HashMap<ConnId, u64>,
    /// Logins whose character load is in flight: conn → (name, presented
    /// token). The token rides along so a later same-name login (takeover or
    /// stale-loading eviction) can be gated on a match before the in-flight
    /// load is disturbed, and so a granted load can seed the new
    /// `PlayerConn.token` without re-reading the wire. Spawn + Welcome happen
    /// when the DbLoaded result arrives.
    loading: HashMap<ConnId, (String, AccountToken)>,
    /// Failed-login attempts per source IP — gates further Login attempts
    /// from an over-budget IP with `RateLimited` before any credential check
    /// runs.
    login_failures: LoginFailures,
    tick: u64,
    next_mechanic_id: u64,
    /// Zone-local wire id allocator — see `ReplIds`.
    repl_ids: ReplIds,
    /// This zone's prefab name table: `None` until the first login grant
    /// builds it from the zone's fully-populated `PrefabLibrary` (sorted
    /// names, deterministic — every chapter's prefab dir has loaded by
    /// App-build time). `Arc` makes resending it to every new connection a
    /// cheap clone instead of a fresh sort/alloc; the `HashMap` is the
    /// reverse index used to encode `EntityState::prefab` at snapshot-gather
    /// time.
    prefab_table: Option<PrefabTable>,
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
            pending: HashMap::new(),
            loading: HashMap::new(),
            login_failures: LoginFailures::new(),
            tick: 0,
            next_mechanic_id: 0,
            repl_ids: ReplIds::new(),
            prefab_table: None,
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.server.local_addr()
    }

    /// Network-layer counters, including the busy-time proxy — exposed so
    /// the soak harness can sample the network thread's saturation directly.
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

/// Convert absolute `ready_at` cooldown stamps into remaining-microsecond
/// durations as of `now`, for persistence. Entries whose cooldown has
/// already elapsed (`ready_at <= now`) are dropped — a save never needs to
/// carry an already-expired cooldown, and a relog would otherwise restore a
/// stale zero-remainder entry it can never grow back from. Pure — the same
/// representation change (`ready_at` replacing `last_cast`) that lets this
/// function skip any `ClassLibrary` lookup at save time.
fn cooldown_remainders(ready: &HashMap<String, u64>, now: u64) -> HashMap<String, u64> {
    ready
        .iter()
        .filter_map(|(id, &ready_at)| {
            let remaining = ready_at.saturating_sub(now);
            (remaining > 0).then(|| (id.clone(), remaining))
        })
        .collect()
}

/// Persist a connected player's live state (position, health, cooldown
/// remainders) under this zone's name. A player whose entity is already
/// gone from the world has nothing to save — silently skipped.
fn save_character(world: &World, state: &NetServerState, pc: &PlayerConn) {
    if let (Ok(tr), Ok(hp)) = (world.get::<&Transform>(pc.entity), world.get::<&Health>(pc.entity)) {
        let cooldowns = cooldown_remainders(&pc.cooldown_ready, state.server.now_micros());
        let xp = world.get::<&vordar_game::progression::Xp>(pc.entity).map(|x| x.0).unwrap_or(pc.carried_xp);
        state.db.save(
            pc.name.clone(),
            CharacterRecord { zone: state.zone.name.clone(), pos: tr.position, health: hp.current, cooldowns, xp, cooldowns_corrupt: false },
        );
    }
}

/// Connections whose player is within AOI range of `center` — the interest-
/// management filter for the mechanic sends below, so a cheating client
/// cannot get a zone-wide radar off telegraph positions and aggregate
/// mechanic traffic never scales past every connection regardless of
/// distance. Uses the same `AOI_RADIUS` as snapshot replication so a
/// telegraph/hit result reaches exactly the clients who could plausibly see
/// it. A client that walks into range only after the message already went
/// out simply misses that one telegraph/hit notification: telegraphs last
/// seconds and `HitResult` still resolves hits correctly regardless of who
/// was told about the schedule, so the miss is cosmetic — accepted rather
/// than adding re-send bookkeeping.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `cooldown_remainders` is the pure conversion from absolute `ready_at`
    /// stamps to save-time remainders — it must subtract correctly for a
    /// skill still cooling down, and drop (not zero-fill) any entry whose
    /// cooldown has already elapsed as of `now`.
    #[test]
    fn cooldown_remainders_drops_expired_and_subtracts_correctly() {
        let mut ready: HashMap<String, u64> = HashMap::new();
        ready.insert("still_cooling".into(), 5_000_000); // ready 4 s from now
        ready.insert("exactly_now".into(), 1_000_000); // ready exactly now
        ready.insert("long_expired".into(), 500_000); // ready well in the past
        let now = 1_000_000;

        let remainders = cooldown_remainders(&ready, now);

        assert_eq!(remainders.get("still_cooling"), Some(&4_000_000));
        assert!(!remainders.contains_key("exactly_now"), "an elapsed cooldown must not persist");
        assert!(!remainders.contains_key("long_expired"), "an already-expired cooldown must not persist");
        assert_eq!(remainders.len(), 1, "only the still-cooling skill should remain: {remainders:?}");
    }
}
