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
pub mod net;
pub mod supervisor;

pub use supervisor::{supervise_zone, join_zone_threads};

use db::DbHandle;
use engine_app::app::App;
use engine_app::prefab_plugin::PrefabPlugin;
use engine_core::prefab::PrefabLibrary;
use engine_net::{NetLimits, NetServer};
use engine_physics::PhysicsPlugin;
use net::NetServerPlugin;
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
/// the server is useless without either. Uses the transport's hostile-client
/// default connection caps — see [`build_server_app_with_limits`] to override
/// them.
pub fn build_server_app(addr: SocketAddr, db_path: &str) -> App {
    build_server_app_with_limits(addr, db_path, NetLimits::default())
}

/// Like [`build_server_app`], but with explicit connection-cap configuration
/// — e.g. a soak harness modeling many distinct bot clients from one source
/// IP raises `max_connections_per_ip` to its bot count here instead of the
/// transport weakening its own default.
pub fn build_server_app_with_limits(addr: SocketAddr, db_path: &str, limits: NetLimits) -> App {
    let mut app = App::new();
    app.add_plugin(PhysicsPlugin)
        .add_plugin(PrefabPlugin)
        .add_plugin(CoreGamePlugin)
        .add_plugin(NetServerPlugin { addr, db_path: db_path.to_owned(), limits });
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
    check_prefab_library(&mut app, &zone.name);
    net::install(&mut app, server, db, None, zone, directory, world_origin);
    app
}

/// Panics if any prefab dir failed to load, or the library ended up empty —
/// a zone must not come up "healthy" serving no prefabs (wrong cwd, corrupt
/// file). Callers that add more prefab dirs after this App is built (chapter
/// plugins, installed post-`build_zone_app`) must call this again once those
/// dirs have loaded.
pub fn check_prefab_library(app: &mut App, zone_name: &str) {
    let lib = app.resource_or_default::<PrefabLibrary>();
    if lib.error_count() > 0 || lib.is_empty() {
        panic!(
            "zone '{zone_name}': prefab library unhealthy ({} load errors, {} prefabs loaded)",
            lib.error_count(),
            lib.len()
        );
    }
}
