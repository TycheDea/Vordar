// Proves the zone-thread watchdog's whole recovery loop: a zone panics
// mid-session, its connected player is disconnected, the watchdog rebuilds
// the zone on the SAME address (via `supervise_zone` and `DbHandle::fork`),
// a fresh connection to that address is Welcomed, a second zone is
// completely unaffected, and the shared shutdown flag still drains
// everything cleanly afterward.

use test_support::{join_with_deadline, spawn_zones, temp_db, test_zones, walk_into_portal, workspace_root, Bot};
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::traits::Resources;
use engine_core::World;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vordar_server::net::ShutdownFlag;
use vordar_server::{build_zone_app, supervise_zone};

/// Panics exactly once across every zone rebuild: `swap` only fires the
/// first time it observes `true`, and every rebuilt App registers a brand
/// new `PanicOnce` sharing the same flag — which by then reads `false` — so
/// the watchdog's own restart never re-triggers it.
struct PanicOnce(Arc<AtomicBool>);

impl System for PanicOnce {
    fn run(&mut self, _world: &mut World, _resources: &mut Resources, _delta: f32) {
        if self.0.swap(false, Ordering::SeqCst) {
            panic!("test-induced zone panic");
        }
    }
}

#[test]
fn a_panicked_zone_restarts_on_the_same_address_and_a_fresh_connection_succeeds() {
    workspace_root();
    let start_addr: SocketAddr = "127.0.0.1:25301".parse().unwrap();
    let east_addr: SocketAddr = "127.0.0.1:25302".parse().unwrap();
    let db = temp_db("watchdog");
    let flag = Arc::new(AtomicBool::new(false));
    let trigger = Arc::new(AtomicBool::new(false));

    let (handles, worker) = spawn_zones(test_zones(), start_addr, east_addr, &db, |addr, handle, zone, directory, world_origin| {
        let is_start = zone.name == "start";
        let zone_flag = flag.clone();
        let zone_trigger = trigger.clone();
        std::thread::spawn(move || {
            let watchdog_name = zone.name.clone();
            // A fresh clone kept separate from `zone_flag` itself: the
            // supervisor borrows `zone_flag` for the whole call below, so
            // the rebuildable closure must move a clone, not the borrowed
            // original.
            let app_flag = zone_flag.clone();
            supervise_zone(&watchdog_name, &zone_flag, move || {
                let mut app =
                    build_zone_app(addr, handle.fork(), zone.clone(), directory.clone(), world_origin);
                app.insert_resource(ShutdownFlag(app_flag.clone()));
                if is_start {
                    app.add_system(PanicOnce(zone_trigger.clone()), Phase::Update, SystemOrder::Default);
                }
                app.run_headless(60.0, None);
            });
        })
    });

    let mut victim = Bot::connect_as(start_addr, "victim");
    victim.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    victim.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    // A brand-new character always starts in the 'start' zone by DB default
    // (db.rs's `load_or_create`), so a fresh name dialing east directly would
    // just get redirected back to start — reaching east for real means
    // walking through the portal, exactly like shutdown.rs's "rover" bot.
    let mut easterner = Bot::connect_as(start_addr, "easterner");
    easterner.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    easterner.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    walk_into_portal(&mut easterner, glam::Vec3::new(10.0, 0.0, 0.0), Duration::from_secs(10));
    easterner.follow_redirect();
    easterner.wait_for("welcome in east", Duration::from_secs(5), |b| b.player_id.is_some());
    easterner.wait_for("first snapshot in east", Duration::from_secs(5), |b| b.own_pos().is_some());

    // Trigger the panic; the unwinding App's NetServer Drop closes the wire.
    trigger.store(true, Ordering::SeqCst);
    victim.wait_for("disconnected", Duration::from_secs(5), |b| b.disconnected);

    // Poll the SAME address until the watchdog's rebuild accepts connections
    // again — before the fix there is no supervisor, the zone thread stays
    // dead, and every retry times out. Pump east throughout: its zone must
    // stay completely unaffected by start's panic and restart.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut fresh = loop {
        easterner.pump();
        assert!(!easterner.disconnected, "east zone must stay unaffected by start's panic");
        if let Some(bot) = Bot::try_connect_as(start_addr, "victim") {
            break bot;
        }
        assert!(Instant::now() < deadline, "start zone never came back up after the panic");
        std::thread::sleep(Duration::from_millis(100));
    };
    fresh.wait_for("welcome after restart", Duration::from_secs(5), |b| b.player_id.is_some());
    fresh.wait_for("first snapshot after restart", Duration::from_secs(5), |b| b.own_pos().is_some());

    // Clean shutdown still drains both zones — supervision composes with the
    // shared shutdown flag: a clean return from run_headless ends
    // supervision with no restart.
    flag.store(true, Ordering::SeqCst);
    for handle in handles {
        join_with_deadline(handle, Duration::from_secs(10), "zone thread");
    }
    drop(worker);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM characters", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 2, "both characters must be persisted after the clean shutdown");
}
