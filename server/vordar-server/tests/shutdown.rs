// Finding 3 of docs/reviews/plan-networking-rework-8-2026-07-12.md: killing
// the process today loses up to a full autosave window (~30 s) of every
// online player's state — there is no code path that saves *all* connected
// players and stops the sim loop. The fix: a shared `ShutdownFlag` resource,
// observed each tick by `ShutdownSystem`, saves every connected player and
// sets `AppExit` within the same tick `run_headless` checks it, so the loop
// returns with the final saves already queued in the `DbWorker`.

mod common;

use common::{settle, temp_db, workspace_root, Bot};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vordar_server::net_plugin::ShutdownFlag;

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
