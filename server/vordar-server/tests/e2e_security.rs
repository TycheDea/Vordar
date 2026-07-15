// Security and metrics tests: login validation, rate limiting, reject counting.
// Isolated from connectivity, combat, and persistence concerns.

use test_support::{name_token, raw_login_probe, spawn_server, spawn_server_with, workspace_root, Bot, MetricMirror};
use engine_app::scheduler::{Phase, SystemOrder};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vordar_protocol::{encode, ClientMsg, LoginDenyReason, MoveIntentEntry};

// Finding 18 of docs/reviews/networking/audit-networking-2026-07-11.md: validate_intent's
// callers only `log::debug!`/`log::warn!` a rejected intent — nothing fed the
// `NetMetrics` the operational-blindness fix (finding 3) claims to expose, so
// a client sending invalid intents was as invisible to metrics as one behaving
// normally. The fix records every validate_intent rejection into
// `NetMetrics::rejects`.
#[test]
fn invalid_intent_increments_reject_counter() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25168".parse().unwrap();
    let rejects: Arc<AtomicU64> = Arc::default();
    {
        let rejects = rejects.clone();
        spawn_server_with(addr, ":memory:", 1200, move |app| {
            app.add_system(
                MetricMirror { dest: rejects, select: |m| &m.rejects },
                Phase::Input,
                SystemOrder::Default,
            );
        });
    }

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());

    assert_eq!(rejects.load(Ordering::Relaxed), 0, "reject counter must start at zero");

    // A far-future timestamp is a guaranteed, deterministic reject with no
    // need for any prior legitimate traffic to go stale first — unlike
    // seq=0 (PlayerConn::last_seq's sentinel), which since protocol v15
    // (networking rework 3 finding 5) is silently skipped as expected
    // last-3-redundancy overlap rather than rejected when it arrives inside
    // a `ClientMsg::MoveIntents` batch (`seq <= pc.last_seq`, and last_seq
    // starts at 0). seq=1 is newer than the sentinel, so only its timestamp
    // — stamped ~10 s in the future, far past FUTURE_SLACK_MICROS — trips
    // validate_intent.
    let t_server_micros = bot.client.server_now_micros().expect("clock synced");
    bot.client.send_datagram(encode(&ClientMsg::MoveIntents {
        intents: vec![MoveIntentEntry { seq: 1, t_server_micros: t_server_micros + 10_000_000, dir: glam::Vec2::ZERO }],
    }));

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && rejects.load(Ordering::Relaxed) == 0 {
        bot.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    assert!(rejects.load(Ordering::Relaxed) >= 1, "an invalid intent must be counted in NetMetrics::rejects");
}

// Finding 3 of docs/reviews/networking/plan-networking-rework-1-2026-07-13.md: `Login`
// used to carry only a bare name, so anyone who knew a character's name could
// take over — or kick — its session (`phase6_login_takeover` in persistence tests
// exercises the LEGITIMATE version of this same mechanism). A same-name login must now
// also present the token the session claimed the name with; a mismatch is
// denied (`LoginDenied(BadCredentials)`, connection left open — the CLIENT
// closes) and the connected victim is never touched: no kick, no DB
// roundtrip, no interruption to its snapshots.
#[test]
fn wrong_token_cannot_kick_or_impersonate() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25169".parse().unwrap();
    spawn_server(addr, ":memory:", 2400);

    let mut guarded = Bot::connect_as(addr, "guarded");
    guarded.wait_for("guarded welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    guarded.walk_until("guarded walks a little", glam::Vec2::new(1.0, 0.0), Duration::from_secs(3), |b| {
        b.own_pos().is_some_and(|p| p.x > 1.0)
    });

    // Attacker: same name, a token that does NOT match "guarded"'s.
    let mut wrong_token = name_token("guarded");
    wrong_token[0] ^= 0xFF;
    let denied = raw_login_probe(addr, "guarded", wrong_token, Duration::from_secs(5), || {});
    assert_eq!(
        denied,
        LoginDenyReason::BadCredentials,
        "the attacker must be denied a Welcome, not silently ignored or granted"
    );

    // The victim must be completely untouched: still online, still getting
    // snapshots, never kicked.
    for _ in 0..20 {
        guarded.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    assert!(!guarded.disconnected, "a mismatched-token login must never kick the connected victim");
    assert!(guarded.own_pos().is_some(), "the victim must keep receiving snapshots throughout");
}

// Finding 4 of docs/reviews/networking/plan-networking-rework-1-2026-07-13.md: nothing
// throttled repeated bad-credential login attempts — a client could probe
// names/tokens as fast as the message token bucket allowed. Failed logins
// (here: token mismatches) now count against a per-IP budget; once
// exhausted, further attempts from that IP are denied `RateLimited` instead
// of running credential verification again, while the CONNECTED victim whose
// name is being probed is never touched.
#[test]
fn login_failures_are_rate_limited() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25170".parse().unwrap();
    spawn_server(addr, ":memory:", 2400);

    let mut keeper = Bot::connect_as(addr, "keeper");
    keeper.wait_for("keeper welcome", Duration::from_secs(5), |b| b.player_id.is_some());

    let mut wrong_token = name_token("keeper");
    wrong_token[0] ^= 0xFF;

    // Six raw (non-Bot) connections in sequence, each presenting the same
    // wrong token for "keeper" — the first five are credential failures, the
    // sixth must be turned away on the rate-limit gate alone. Each probe
    // keeps the victim's connection alive and pumped throughout, not just
    // checked at the very end.
    let mut reasons: Vec<LoginDenyReason> = Vec::new();
    for _ in 0..6 {
        reasons.push(raw_login_probe(addr, "keeper", wrong_token, Duration::from_secs(5), || keeper.pump()));
    }

    assert_eq!(
        &reasons[..5],
        &[LoginDenyReason::BadCredentials; 5],
        "the first five bad-token attempts must each be denied BadCredentials: {reasons:?}"
    );
    assert_eq!(
        reasons[5],
        LoginDenyReason::RateLimited,
        "the sixth attempt within the failure window must be denied RateLimited: {reasons:?}"
    );

    assert!(!keeper.disconnected, "rate-limited probing of another name must never touch the connected victim");
    assert!(keeper.own_pos().is_some(), "the victim must keep receiving snapshots throughout");
}
