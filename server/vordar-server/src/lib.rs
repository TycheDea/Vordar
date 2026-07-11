// vordar-server — the authoritative zone server as a library.
//
// One zone = one headless App running the shared simulation (vordar-game) plus
// NetServerPlugin, which is the only place network state touches the world:
//   Input phase      — drain connections/messages, validate intents, emit
//                      MoveIntent events for the shared movement system
//   PostUpdate phase — broadcast full entity snapshots at SNAPSHOT_HZ
//
// Anti-cheat caps live in intent validation here, from protocol v1 on
// (DESIGN.md §3): monotonic timestamps, arrival deadline bounded below by
// max(RTT, MAX_REWIND) — MAX_REWIND is a floor while RTT estimates settle,
// contained by the separate resolve-time rewind cap — and positions only
// ever computed from intents.

pub mod db;
pub mod net_plugin;

use db::DbHandle;
use engine_app::app::App;
use engine_app::prefab_plugin::PrefabPlugin;
use engine_net::NetServer;
use engine_physics::PhysicsPlugin;
use net_plugin::NetServerPlugin;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;
use vordar_game::zones::ZoneDef;
use vordar_game::CoreGamePlugin;
use vordar_protocol::PROTOCOL_VERSION;

pub const TICK_HZ: f64 = 60.0;

/// Assemble a single-zone server App listening on `addr`, persisting
/// characters to the SQLite database at `db_path` (`:memory:` for throwaway).
/// Panics if the QUIC endpoint cannot bind or the database cannot open —
/// the server is useless without either.
pub fn build_server_app(addr: SocketAddr, db_path: &str) -> App {
    let mut app = App::new();
    app.add_plugin(PhysicsPlugin)
        .add_plugin(PrefabPlugin)
        .add_plugin(CoreGamePlugin)
        .add_plugin(NetServerPlugin { addr, db_path: db_path.to_owned() });
    app
}

/// Assemble one zone's App for a multi-zone server: shared DB worker handle,
/// shared world-time origin, and the full zone directory for Redirects.
/// Must be called ON the zone's own thread — a built App cannot move between
/// threads (systems aren't Send). Panics if the QUIC endpoint cannot bind.
pub fn build_zone_app(
    addr: SocketAddr,
    db: DbHandle,
    zone: ZoneDef,
    directory: HashMap<String, SocketAddr>,
    world_origin: Instant,
) -> App {
    let server = NetServer::bind(addr, PROTOCOL_VERSION)
        .unwrap_or_else(|e| panic!("zone '{}': failed to bind {addr}: {e}", zone.name));
    let mut app = App::new();
    app.add_plugin(PhysicsPlugin)
        .add_plugin(PrefabPlugin)
        .add_plugin(CoreGamePlugin);
    net_plugin::install(&mut app, server, db, None, zone, directory, world_origin);
    app
}
