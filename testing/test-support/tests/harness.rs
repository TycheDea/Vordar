// Proves `SimDeadline` (via `Bot::wait_for`) fails on sim-tick budgets, not
// wall-clock ones, with an 8x wall backstop for a hung/silent sim.

use glam::Vec2;
use std::net::SocketAddr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};
use test_support::{spawn_server, workspace_root, Bot, MOVE_TOKEN_CAP};

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned())
}

#[test]
fn sim_budget_expires_against_a_live_server() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25501".parse().unwrap();
    spawn_server(addr, ":memory:", 1200);

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    let started = Instant::now();
    let result = catch_unwind(AssertUnwindSafe(|| {
        bot.wait_for("never", Duration::from_millis(500), |_| false);
    }));
    let elapsed = started.elapsed();

    let err = result.expect_err("wait_for must panic once the sim budget is exhausted");
    let text = panic_text(&*err);
    assert!(text.contains("sim budget exhausted"), "unexpected panic payload: {text}");
    // The 8x wall backstop for a 500ms budget is 4s; a sim-driven failure
    // must land well before that, proving the sim clock (not the wall)
    // expired it.
    assert!(elapsed < Duration::from_secs(2), "expired too late for a sim-budget failure: {elapsed:?}");
}

#[test]
fn wall_backstop_covers_a_silent_server() {
    workspace_root();
    // Nothing listens here: `Bot::connect` dials but no snapshot (or any
    // message) ever arrives, so `latest_state_tick` stays 0 and the sim
    // budget can never anchor.
    let addr: SocketAddr = "127.0.0.1:25555".parse().unwrap();
    let mut bot = Bot::connect(addr);

    let started = Instant::now();
    let result = catch_unwind(AssertUnwindSafe(|| {
        bot.wait_for("welcome", Duration::from_millis(200), |b| b.player_id.is_some());
    }));
    let elapsed = started.elapsed();

    let err = result.expect_err("wait_for must panic once the wall backstop is exceeded");
    let text = panic_text(&*err);
    assert!(text.contains("wall backstop exceeded"), "unexpected panic payload: {text}");
    // 8 x 200ms = 1.6s; it must not fire at the raw 200ms budget.
    assert!(elapsed >= Duration::from_millis(1600), "fired before the wall backstop: {elapsed:?}");
    assert!(elapsed < Duration::from_millis(2600), "fired too late past the wall backstop: {elapsed:?}");
}

#[test]
fn send_move_never_outruns_the_sim() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25502".parse().unwrap();
    spawn_server(addr, ":memory:", 1200);

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    let start_tick = bot.latest_state_tick;
    let loop_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < loop_deadline {
        bot.send_move(Vec2::X);
        bot.pump();
        std::thread::sleep(Duration::from_millis(2));
    }
    let elapsed_sim_ticks = bot.latest_state_tick - start_tick;

    assert!(
        bot.seq as u64 <= elapsed_sim_ticks + MOVE_TOKEN_CAP as u64,
        "seq {} outran the sim: elapsed_sim_ticks={} + MOVE_TOKEN_CAP={} = {}",
        bot.seq,
        elapsed_sim_ticks,
        MOVE_TOKEN_CAP,
        elapsed_sim_ticks + MOVE_TOKEN_CAP as u64
    );

    bot.send_move(Vec2::ZERO);
    bot.wait_for("full stream acked", Duration::from_secs(5), |b| b.last_ack == b.seq);
}
