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
use std::time::Instant;
use vordar_game::chapter::ChapterRegistry;
use vordar_server::db::DbWorker;
use vordar_server::{build_zone_app, TICK_HZ};

/// Every chapter this binary can host. Linking a new chapter crate +
/// one line here is the entire integration.
fn chapters() -> ChapterRegistry {
    ChapterRegistry::new(vec![chapter_01::module()])
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

    let handles: Vec<_> = zones
        .zones
        .into_iter()
        .map(|zone| {
            let addr = directory[&zone.name];
            let directory = directory.clone();
            let handle = db.handle();
            std::thread::Builder::new()
                .name(format!("zone-{}", zone.name))
                .spawn(move || {
                    // Built on this thread: a built App cannot move (systems
                    // aren't Send).
                    let chapter = zone.chapter.clone();
                    let mut app = build_zone_app(addr, handle, zone, directory, world_origin);
                    if let Some(name) = chapter.as_deref() {
                        chapters()
                            .install(name, &mut app)
                            .unwrap_or_else(|e| panic!("zones.ron: {e}"));
                    }
                    app.insert_resource(vordar_game::world::load_world_events("content/world/events.ron"));
                    log::info!("zone listening on {addr}");
                    app.run_headless(TICK_HZ, None);
                })
                .expect("spawn zone thread")
        })
        .collect();

    for handle in handles {
        let _ = handle.join();
    }
}
