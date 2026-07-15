// Vordar dedicated server — one headless App per zone, runs until killed.
//
//   cargo run -p vordar-server [base_addr]      (default 127.0.0.1:5151)
//
// Zone i from content/zones/zones.ron listens on base_port + i, so "start"
// (index 0) is the address clients connect to first; portals redirect them
// onward. One shared DB worker and one shared world-time origin serve every
// zone: saves/loads stay FIFO across zones and world events fire everywhere
// at the same instant.
//
// Env knobs:
//   VORDAR_DB=path     character database (default vordar.db in cwd)
//
// Run from the workspace root: prefabs load from content/ relative to cwd.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use vordar_game::chapter::ChapterRegistry;
use vordar_server::db::DbWorker;
use vordar_server::net::ShutdownFlag;
use vordar_server::{build_zone_app, join_zone_threads, supervise_zone, TICK_HZ};

/// Every chapter this binary can host. Linking a new chapter crate +
/// one line here is the entire integration.
fn chapters() -> ChapterRegistry {
    ChapterRegistry::new(vec![chapter_01::module(), chapter_02::module()])
}

fn main() {
    let base_addr: SocketAddr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:5151".into())
        .parse()
        .expect("usage: vordar-server [ip:port]");

    let zones = vordar_game::zones::load_zones("content/zones/zones.ron");
    vordar_game::zones::validate_zones(&zones).expect("invalid zone topology");

    let directory: HashMap<String, SocketAddr> = zones
        .zones
        .iter()
        .enumerate()
        .map(|(i, z)| (z.name.clone(), SocketAddr::new(base_addr.ip(), base_addr.port() + i as u16)))
        .collect();

    let db_path = std::env::var("VORDAR_DB").unwrap_or_else(|_| "vordar.db".into());
    let db = DbWorker::spawn(&db_path).unwrap_or_else(|e| panic!("failed to open db '{db_path}': {e}"));
    let world_origin = Instant::now();

    // SIGINT/SIGTERM (Unix) or Ctrl+C/console-close (Windows): every zone
    // observes the same flag via its ShutdownFlag resource (finding 3) and
    // drains itself in-simulation. A second signal force-exits for the
    // stuck-shutdown case.
    let shutdown = Arc::new(AtomicBool::new(false));
    let signal_shutdown = shutdown.clone();
    let already_signaled = AtomicBool::new(false);
    ctrlc::set_handler(move || {
        if already_signaled.swap(true, Ordering::Relaxed) {
            log::error!("second shutdown signal received — forcing exit");
            std::process::exit(1);
        }
        log::info!("shutdown signal received — draining zones");
        signal_shutdown.store(true, Ordering::Relaxed);
    })
    .expect("failed to install signal handler");

    let handles: Vec<(String, _)> = zones
        .zones
        .into_iter()
        .map(|zone| {
            let addr = directory[&zone.name];
            let directory = directory.clone();
            let handle = db.handle();
            let name = zone.name.clone();
            let zone_shutdown = shutdown.clone();
            let join = std::thread::Builder::new()
                .name(format!("zone-{}", zone.name))
                .spawn(move || {
                    // Built on this thread: a built App cannot move (systems
                    // aren't Send). `supervise_zone` (rework 10) reruns this
                    // closure from scratch on the same address after a panic,
                    // so every capture below must be safe to rebuild from —
                    // `zone`/`directory` are cloned per rebuild and `handle`
                    // mints a fresh `DbHandle` via `fork()` so a rebuilt App
                    // never inherits a dead App's in-flight reply channel.
                    let watchdog_name = zone.name.clone();
                    // A fresh clone kept separate from `zone_shutdown`
                    // itself: the supervisor borrows `zone_shutdown` for the
                    // whole call below, so the rebuildable closure must move
                    // a clone into itself, not the borrowed original.
                    let app_shutdown = zone_shutdown.clone();
                    supervise_zone(&watchdog_name, &zone_shutdown, move || {
                        let chapter = zone.chapter.clone();
                        let mut app =
                            build_zone_app(addr, handle.fork(), zone.clone(), directory.clone(), world_origin);
                        if let Some(name) = chapter.as_deref() {
                            chapters()
                                .install(name, &mut app)
                                .unwrap_or_else(|e| panic!("zones.ron: {e}"));
                        }
                        app.insert_resource(vordar_game::world::load_world_events("content/zones/events.ron"));
                        app.insert_resource(ShutdownFlag(app_shutdown.clone()));
                        log::info!("zone listening on {addr}");
                        app.run_headless(TICK_HZ, None);
                    });
                })
                .expect("spawn zone thread");
            (name, join)
        })
        .collect();

    // A panicked zone thread used to be silently swallowed here (networking
    // audit 2026-07-11, finding 18) — see `join_zone_threads`'s doc comment.
    join_zone_threads(handles);
}
