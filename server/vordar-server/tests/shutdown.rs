// Finding 3 of docs/reviews/networking/plan-networking-rework-8-2026-07-12.md: killing
// the process today loses up to a full autosave window (~30 s) of every
// online player's state — there is no code path that saves *all* connected
// players and stops the sim loop. The fix: a shared `ShutdownFlag` resource,
// observed each tick by `ShutdownSystem`, saves every connected player and
// sets `AppExit` within the same tick `run_headless` checks it, so the loop
// returns with the final saves already queued in the `DbWorker`.

mod common;

use common::{settle, temp_db, test_zones, walk_into_portal, workspace_root, Bot};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vordar_server::build_zone_app;
use vordar_server::db::DbWorker;
use vordar_server::net::ShutdownFlag;

#[test]
fn shutdown_flag_saves_all_players_and_returns_from_run_headless() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25201".parse().unwrap();
    let db = temp_db("shutdown-flag");
    let flag = Arc::new(AtomicBool::new(false));

    let server_flag = flag.clone();
    let server_db = db.clone();
    let server_thread = std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, &server_db);
        app.insert_resource(ShutdownFlag(server_flag));
        // No tick budget — only the shutdown flag (via AppExit) can end this
        // loop. Before the fix, nothing ever sets it and the thread never
        // returns.
        app.run_headless(60.0, None);
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect_as(addr, "walker");
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    // Walk east for ~1 s so the saved position is well off the spawn ring.
    let run_until = Instant::now() + Duration::from_millis(1000);
    while Instant::now() < run_until {
        bot.send_move(glam::Vec2::new(1.0, 0.0));
        bot.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    bot.send_move(glam::Vec2::ZERO);
    settle(&mut bot, Duration::from_millis(300));
    let last_pos = bot.own_pos().unwrap();

    // Flip the shared flag — within one tick, ShutdownSystem must save every
    // connected player and request AppExit.
    flag.store(true, Ordering::Relaxed);

    // Join with a deadline: before the fix, run_headless(_, None) never
    // returns and this would hang forever.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(server_thread.join());
    });
    rx.recv_timeout(Duration::from_secs(10))
        .expect("server thread did not exit within the deadline after the shutdown flag was set")
        .expect("server thread panicked");

    // The client must observe the connection close (NetServer's Drop, wired
    // by finding 1, fires when the App drops moments after run_headless
    // returns).
    bot.wait_for("disconnected", Duration::from_secs(5), |b| b.disconnected);

    // The final save must have landed — proving ShutdownSystem's save, not a
    // disconnect-save (the bot never initiated a disconnect before the flag
    // was set) or the periodic autosave (its ~30 s window is far longer than
    // this test runs).
    let conn = rusqlite::Connection::open(&db).unwrap();
    let saved_x: f64 = conn
        .query_row("SELECT pos_x FROM characters WHERE name = 'walker'", [], |r| r.get(0))
        .unwrap();
    assert!(
        (saved_x - last_pos.x as f64).abs() < 1.0,
        "shutdown save must persist the last observed position: expected ~{}, got {saved_x}",
        last_pos.x
    );
}

// Finding 4: nothing wires main's OS signal to the per-zone ShutdownFlag
// across a REAL multi-zone topology, and tests/zones.rs's own harness leaks
// its DbWorker (`std::mem::forget`) precisely because zone threads have had
// no way to be told to stop. This test builds the exact topology `main` runs
// (two zones, one shared DbWorker, one shared world-time origin) but keeps
// the zone JoinHandles and wires one shared ShutdownFlag into both apps —
// the harness this test builds IS the wiring `main` needs, minus the OS
// signal itself.
#[test]
fn shared_flag_drains_both_zones_and_worker_drop_returns() {
    workspace_root();
    let start_addr: SocketAddr = "127.0.0.1:25210".parse().unwrap();
    let east_addr: SocketAddr = "127.0.0.1:25211".parse().unwrap();
    let db = temp_db("shutdown-multizone");
    let flag = Arc::new(AtomicBool::new(false));

    let directory: HashMap<String, SocketAddr> =
        HashMap::from([("start".to_owned(), start_addr), ("east".to_owned(), east_addr)]);
    let worker = DbWorker::spawn(&db).expect("db open");
    let world_origin = Instant::now();

    let handles: Vec<_> = test_zones()
        .into_iter()
        .map(|zone| {
            let addr = directory[&zone.name];
            let directory = directory.clone();
            let handle = worker.handle();
            let zone_flag = flag.clone();
            std::thread::spawn(move || {
                let mut app = build_zone_app(addr, handle, zone, directory, world_origin);
                app.insert_resource(ShutdownFlag(zone_flag));
                // No tick budget — only the shared shutdown flag can end
                // either zone's loop.
                app.run_headless(60.0, None);
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(300));

    // Stays in start, walking away from start's portal (x=10) so it never
    // transfers — its final save must be a genuine start-zone position.
    let mut stay = Bot::connect_as(start_addr, "stay");
    stay.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    stay.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    let run_until = Instant::now() + Duration::from_millis(800);
    while Instant::now() < run_until {
        stay.send_move(glam::Vec2::new(-1.0, 0.0));
        stay.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    stay.send_move(glam::Vec2::ZERO);
    settle(&mut stay, Duration::from_millis(300));
    let stay_pos = stay.own_pos().unwrap();

    // Transfers to east through the real portal (the honest way, like
    // zones.rs's phase7_portal_round_trip), then walks away from east's own
    // portal back to start (x=-10) so it doesn't bounce back mid-test.
    let mut rover = Bot::connect_as(start_addr, "rover");
    rover.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    rover.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    walk_into_portal(&mut rover, glam::Vec3::new(10.0, 0.0, 0.0), Duration::from_secs(10));
    rover.follow_redirect();
    rover.wait_for("welcome in east", Duration::from_secs(5), |b| b.player_id.is_some());
    rover.wait_for("snapshot in east", Duration::from_secs(5), |b| b.own_pos().is_some());
    let run_until = Instant::now() + Duration::from_millis(800);
    while Instant::now() < run_until {
        rover.send_move(glam::Vec2::new(1.0, 0.0));
        rover.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    rover.send_move(glam::Vec2::ZERO);
    settle(&mut rover, Duration::from_millis(300));
    let rover_pos = rover.own_pos().unwrap();

    // Flip the ONE shared flag — both zone threads must drain within a tick
    // and return from run_headless.
    flag.store(true, Ordering::Relaxed);

    let per_zone_deadline = Duration::from_secs(10);
    for handle in handles {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(handle.join());
        });
        rx.recv_timeout(per_zone_deadline)
            .expect("zone thread did not exit within the deadline after the shutdown flag was set")
            .expect("zone thread panicked");
    }

    // No mem::forget (unlike tests/zones.rs's harness, which leaks the
    // worker precisely because zone threads previously had no way to stop):
    // every zone App has now dropped, taking its DbHandle down with it, so
    // the request channel is genuinely closed and this Drop must return
    // instead of hanging.
    drop(worker);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let stay_x: f64 = conn
        .query_row("SELECT pos_x FROM characters WHERE name = 'stay'", [], |r| r.get(0))
        .unwrap();
    let rover_x: f64 = conn
        .query_row("SELECT pos_x FROM characters WHERE name = 'rover'", [], |r| r.get(0))
        .unwrap();
    assert!(
        (stay_x - stay_pos.x as f64).abs() < 1.0,
        "start-zone shutdown save must persist the last observed position: expected ~{}, got {stay_x}",
        stay_pos.x
    );
    assert!(
        (rover_x - rover_pos.x as f64).abs() < 1.0,
        "east-zone shutdown save must persist the last observed position: expected ~{}, got {rover_x}",
        rover_pos.x
    );
}
