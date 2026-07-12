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
use engine_net::{NetLimits, NetServer};
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
/// the server is useless without either. Uses the transport's hostile-client
/// default connection caps — see [`build_server_app_with_limits`] to override
/// them.
pub fn build_server_app(addr: SocketAddr, db_path: &str) -> App {
    build_server_app_with_limits(addr, db_path, NetLimits::default())
}

/// Like [`build_server_app`], but with explicit connection-cap configuration
/// (networking audit 2026-07-11, finding 20) — e.g. a soak harness modeling
/// many distinct bot clients from one source IP raises
/// `max_connections_per_ip` to its bot count here instead of the transport
/// weakening its own default.
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
    net_plugin::install(&mut app, server, db, None, zone, directory, world_origin);
    app
}

/// Join every zone thread, logging loudly if one panicked instead of exiting
/// cleanly (networking audit 2026-07-11, finding 18). `main` used to discard
/// `handle.join()`'s `Result` outright (`let _ = handle.join();`), so a
/// panicked zone died with no trace at all: its listener stayed bound but
/// dead, and every other zone kept redirecting players into that now-dead
/// address forever. This closes the visibility gap; actually recovering the
/// zone (restart, or pulling it from the shared directory so other zones stop
/// redirecting into it) needs `NetServer` to gain a shutdown path first — see
/// `reworks-networking-2026-07-11.md` finding 10.
pub fn join_zone_threads(handles: Vec<(String, std::thread::JoinHandle<()>)>) {
    for (name, handle) in handles {
        if let Err(payload) = handle.join() {
            log::error!(
                "zone '{name}' thread panicked and exited ({}); its listener is now dead — \
                 other zones will keep redirecting players into a stale address until the \
                 process is restarted",
                panic_message(&payload),
            );
        }
    }
}

/// Best-effort human-readable text from a caught panic payload — `panic!`
/// with a literal yields `&'static str`, `panic!("...{}...")` yields `String`;
/// anything else (a custom payload via `panic_any`) has no reliable `Display`.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn panic_message_extracts_string_literal_payload() {
        let handle = std::thread::spawn(|| panic!("zone exploded"));
        let payload = handle.join().unwrap_err();
        assert_eq!(panic_message(&payload), "zone exploded");
    }

    #[test]
    fn panic_message_extracts_formatted_string_payload() {
        let cause = "bad zones.ron";
        let handle = std::thread::spawn(move || panic!("startup failed: {cause}"));
        let payload = handle.join().unwrap_err();
        assert_eq!(panic_message(&payload), "startup failed: bad zones.ron");
    }

    /// Regression test for finding 18: a panicked zone thread must not
    /// prevent `join_zone_threads` from joining (and thus not silently
    /// dropping) the other zone threads behind it, and must not itself
    /// propagate the panic and take the whole process down with it.
    #[test]
    fn a_panicked_zone_does_not_stop_the_others_from_being_joined() {
        let joined = Arc::new(AtomicUsize::new(0));
        let healthy_before = {
            let joined = joined.clone();
            std::thread::Builder::new().spawn(move || { joined.fetch_add(1, Ordering::SeqCst); }).unwrap()
        };
        let panicking = std::thread::Builder::new().spawn(|| panic!("zone crashed")).unwrap();
        let healthy_after = {
            let joined = joined.clone();
            std::thread::Builder::new().spawn(move || { joined.fetch_add(1, Ordering::SeqCst); }).unwrap()
        };
        join_zone_threads(vec![
            ("start".to_owned(), healthy_before),
            ("east".to_owned(), panicking),
            ("west".to_owned(), healthy_after),
        ]);
        assert_eq!(joined.load(Ordering::SeqCst), 2, "both healthy zone threads must still be joined despite the panic in between");
    }
}
