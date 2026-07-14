// Combat mechanics tests: scheduled AOE, rend, onslaught. Isolated from
// connectivity, persistence, and wire-format concerns.

mod common;

use common::{settle, workspace_root, Bot};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_protocol::{encode, ClientMsg};

// Phase 4: scheduled-snapshot combat under 150 ms latency. One identical
// MechanicScheduled reaches every client; standing in the area at T is a hit;
// stepping out before T (by the defender's own synced clock) is a miss even
// though those packets arrive after T; backdated casts are rejected.
#[test]
fn phase4_scheduled_aoe() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25156".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(2400));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut a = Bot::connect(addr);
    a.wait_for("A welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    a.wait_for("A clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    let mut b = Bot::connect_with_latency(addr, Duration::from_millis(150));
    b.wait_for("B welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    b.wait_for("B clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    let a_id = a.player_id.unwrap();
    let b_id = b.player_id.unwrap();
    a.wait_for("A sees B", Duration::from_secs(5), |bot| bot.last_snapshot.contains_key(&b_id));

    // Finding 8 of docs/reviews/networking/audit-networking-2026-07-11.md: every fresh
    // spawn now pessimistically starts its abilities on full cooldown, so
    // "cleave" isn't castable in the instant after login — clear it first.
    std::thread::sleep(Duration::from_millis(3200));

    // ── Cast 1: B stands still inside the area → hit (caster A excluded). ──
    // "cleave" (the Ravager's heavy Scheduled hit) shares blast's choreography
    // numbers: radius 4.0, 2 s cast, 3 s cooldown.
    let b_pos = *a.last_snapshot.get(&b_id).unwrap();
    let target = glam::Vec2::new(b_pos.x, b_pos.z);
    a.send_cast("cleave", target);
    a.wait_for("A gets MechanicScheduled", Duration::from_secs(3), |bot| !bot.mechanics.is_empty());
    b.wait_for("B gets MechanicScheduled", Duration::from_secs(3), |bot| !bot.mechanics.is_empty());
    // The design's broadcast rule: every client gets the SAME schedule.
    assert_eq!(a.mechanics[0], b.mechanics[0], "schedule must be identical on all clients");
    let (mech1, _) = a.mechanics[0];

    a.wait_for("first hit result", Duration::from_secs(4), |bot| bot.hit_results.contains_key(&mech1));
    b.wait_for("B sees first hit result", Duration::from_secs(4), |bot| bot.hit_results.contains_key(&mech1));
    let hits = &a.hit_results[&mech1];
    assert!(hits.contains(&b_id), "B stood in the area at T and must be hit");
    assert!(!hits.contains(&a_id), "the caster is excluded from its own mechanic");

    // ── Cast 2: B steps out before T (on its own clock) → miss. ──
    std::thread::sleep(Duration::from_secs(2)); // clear the 3 s cooldown (cast 1 spent 2 s resolving)
    a.pump();
    b.pump();
    let b_pos = *a.last_snapshot.get(&b_id).unwrap();
    a.send_cast("cleave", glam::Vec2::new(b_pos.x, b_pos.z));
    b.wait_for("B gets second schedule", Duration::from_secs(3), |bot| bot.mechanics.len() >= 2);
    let (mech2, resolve_at) = *b.mechanics.last().unwrap();
    assert_ne!(mech1, mech2);

    // B walks east starting at T−800 ms by its own synced clock; it crosses
    // the radius-4 border ~T−130 ms. Its last pre-T intents arrive ~75 ms
    // late — the stamp-based rewind must still count them as before T.
    loop {
        let now = b.client.server_now_micros().unwrap();
        if now >= resolve_at + 400_000 {
            break;
        }
        if now + 800_000 >= resolve_at {
            b.send_move(glam::Vec2::new(1.0, 0.0));
        }
        a.pump();
        b.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    b.send_move(glam::Vec2::ZERO);

    a.wait_for("second hit result", Duration::from_secs(4), |bot| bot.hit_results.contains_key(&mech2));
    assert!(
        !a.hit_results[&mech2].contains(&b_id),
        "B stepped out before T — the rewound test must miss it"
    );

    // ── Backdated cast: rejected server-side, nothing gets scheduled. ──
    let count_before = b.mechanics.len();
    let now = b.client.server_now_micros().unwrap();
    b.seq += 1;
    b.client.send(encode(&ClientMsg::CastIntent {
        seq: b.seq,
        t_server_micros: now.saturating_sub(10_000_000),
        skill: "cleave".into(),
        target: glam::Vec2::ZERO,
    }));
    std::thread::sleep(Duration::from_millis(1500));
    b.pump();
    assert_eq!(b.mechanics.len(), count_before, "backdated cast must be rejected");
}

// Phase 7.5 (Ravager rework): the player's default attack, end to end. A
// camp-resident grunt replicates into the bot's AOI; the bot closes to melee
// and casts "rend" (fast Scheduled strike, 20 dmg with the Ravager's power)
// until the grunt's 30 HP run out — observed as an AOI leave while the bot
// stays alive (player_id never changes → no death re-Welcome).
#[test]
fn phase7_5_rend_kills_camped_enemy() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25163".parse().unwrap();
    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        app.add_plugin(chapter_01::Chapter01Plugin);
        app.run_headless(60.0, Some(2400));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    bot.wait_for("a grunt in the AOI", Duration::from_secs(5), |b| {
        b.prefabs.values().any(|p| p == "grunt")
    });
    let original_body = bot.player_id.unwrap();
    let grunt_id = *bot.prefabs.iter().find(|(_, p)| *p == "grunt").unwrap().0;

    // Fight at rend's edge: close to ~2.2, back off under 1.6 (the grunt's
    // 2.5 speed can't catch the bot's 6.0 — face-tanking a 10-per-contact
    // charger at the Ravager's zero defense loses). Two clean hits (16 + 4
    // power) beat 30 HP. Cast attempts go out every 250 ms once well inside
    // max_range (2.0 vs 2.5): the server silently drops out-of-range and
    // on-cooldown casts and its own 900 ms cooldown paces the real ones, so
    // a rejected attempt costs 250 ms — the old edge-of-range gate (2.4,
    // 0.1 slack at up to 8.5 u/s closing speed) burned a full second per
    // stale-snapshot miss and flaked past the deadline under parallel-test
    // CPU load.
    let mut last_cast = Instant::now() - Duration::from_secs(2);
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut hp_seen: Vec<i32> = Vec::new();
    while bot.last_snapshot.contains_key(&grunt_id) {
        assert!(Instant::now() < deadline, "grunt survived 25 s of rends");
        if let (Some(own), Some(grunt)) = (bot.own_pos(), bot.last_snapshot.get(&grunt_id).copied()) {
            let offset = glam::Vec2::new(grunt.x - own.x, grunt.z - own.z);
            let dist = offset.length();
            if dist > 2.2 {
                bot.send_move(offset.normalize());
            } else if dist < 1.6 {
                bot.send_move(-offset.normalize());
            } else {
                bot.send_move(glam::Vec2::ZERO);
            }
            if dist <= 2.0 && last_cast.elapsed() > Duration::from_millis(250) {
                bot.send_cast("rend", glam::Vec2::new(grunt.x, grunt.z));
                last_cast = Instant::now();
            }
        }
        bot.pump();
        if let Some(&hp) = bot.last_hp.get(&grunt_id) {
            if hp_seen.last() != Some(&hp) {
                hp_seen.push(hp);
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    // The death message may ride the tick after the AOI-leave snapshot.
    bot.wait_for("EntityDied for the grunt", Duration::from_secs(2), |b| {
        b.deaths.iter().any(|&(id, _)| id == grunt_id)
    });

    assert!(!bot.mechanics.is_empty(), "rend's strike schedule must replicate");
    assert_eq!(bot.player_id, Some(original_body), "the bot must survive the fight");
    // Protocol v8: hp rides in snapshots — the grunt's 30 HP visibly drops
    // before it dies, and its death is announced with a position.
    assert_eq!(hp_seen.first(), Some(&30), "grunt enters at full health, saw {hp_seen:?}");
    assert!(
        hp_seen.windows(2).all(|w| w[1] < w[0]),
        "replicated hp only decreases during the fight: {hp_seen:?}"
    );
    assert!(hp_seen.len() >= 2, "at least one damage tick replicated: {hp_seen:?}");
}

// The Ravager's gap-closer, end to end: one Onslaught cast must both dash the
// caster's replicated position to the target point and fire the arrival hit
// test there (hitting the bystander, never the caster).
#[test]
fn ravager_onslaught_dashes_and_resolves() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25166".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(2400));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut a = Bot::connect(addr);
    a.wait_for("A welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    a.wait_for("A clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    let mut b = Bot::connect_as(addr, "bystander");
    b.wait_for("B welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    let a_id = a.player_id.unwrap();
    let b_id = b.player_id.unwrap();
    a.wait_for("A sees B", Duration::from_secs(5), |bot| bot.last_snapshot.contains_key(&b_id));

    let start = a.own_pos().unwrap();
    let target = *a.last_snapshot.get(&b_id).unwrap();
    let start_dist = (glam::Vec2::new(start.x, start.z) - glam::Vec2::new(target.x, target.z)).length();

    // Finding 1 of docs/reviews/networking/plan-networking-rework-1-2026-07-13.md:
    // cooldowns now persist as remainders instead of pessimistically seeding
    // full cooldown at spawn, so a fresh character's "onslaught" is castable
    // immediately — no clearing wait needed.
    a.send_cast("onslaught", glam::Vec2::new(target.x, target.z));

    a.wait_for("dash schedule broadcast", Duration::from_secs(3), |bot| !bot.mechanics.is_empty());
    let (mech, _) = a.mechanics[0];
    a.wait_for("arrival hit result", Duration::from_secs(4), |bot| bot.hit_results.contains_key(&mech));
    let hits = &a.hit_results[&mech];
    assert!(hits.contains(&b_id), "the bystander stood at the arrival point and must be hit");
    assert!(!hits.contains(&a_id), "the caster is excluded from its own arrival strike");

    // The dash itself: A's replicated position must have closed on the target
    // (separation pushes the two solids apart after landing, so "close", not
    // "exact").
    settle(&mut a, Duration::from_millis(500));
    let end = a.own_pos().unwrap();
    let end_dist = (glam::Vec2::new(end.x, end.z) - glam::Vec2::new(target.x, target.z)).length();
    assert!(
        end_dist < 2.0 && end_dist < start_dist * 0.5,
        "the caster must dash to the target: start {start_dist:.2} → end {end_dist:.2}"
    );
}
