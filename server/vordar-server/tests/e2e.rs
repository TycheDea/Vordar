// Connectivity and AOI tests: server/client handshake, movement replication,
// player visibility, NPC/entity replication, AOI radius, bandwidth budgeting.
// Isolated from combat, persistence, security, and wire-format concerns.
//
// Tests kept in this file:
//   phase1_end_to_end        — connect, clock sync, mutual visibility,
//                              movement replication, disconnect despawn
//   phase2_simulated_latency — prediction acks + intent validation at 150 ms
//   epsilon_over_unit_direction_still_moves_player — normalize noise tolerance
//   phase3_npc_replication   — chapter waves replicate NPCs to clients
//   phase3_respawn_after_death — player respawn after entity death
//   phase3_aoi_border        — AOI enter/leave at the radius border + bandwidth
//   phase5_world_clock_and_blood_moon — world time + scripted events
//   far_bot_never_sees_out_of_aoi_mechanic — AOI scope for damage telegraphs

use test_support::{settle, workspace_root, Bot, PopulateSystem};
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use hecs::Entity;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

#[test]
fn phase1_end_to_end() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25151".parse().unwrap();

    // Server runs ~20 s of simulation on its own thread, then winds down.
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut a = Bot::connect(addr);
    a.wait_for("bot A welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    a.wait_for("bot A clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    a.wait_for("bot A first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    let mut b = Bot::connect(addr);
    b.wait_for("bot B welcome", Duration::from_secs(5), |b| b.player_id.is_some());

    // Both players replicated to both clients.
    let a_id = a.player_id.unwrap();
    let b_id = b.player_id.unwrap();
    assert_ne!(a_id, b_id);
    a.wait_for("bot A sees both players", Duration::from_secs(5), |bot| {
        bot.last_snapshot.contains_key(&a_id) && bot.last_snapshot.contains_key(&b_id)
    });
    b.wait_for("bot B sees both players", Duration::from_secs(5), |bot| {
        bot.last_snapshot.contains_key(&a_id) && bot.last_snapshot.contains_key(&b_id)
    });

    // A runs east for ~1.5 s; both bots must observe the movement.
    let start = a.own_pos().unwrap();
    let run_until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < run_until {
        a.send_move(glam::Vec2::new(1.0, 0.0));
        a.pump();
        b.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    a.send_move(glam::Vec2::ZERO);
    a.wait_for("A's movement settles", Duration::from_secs(2), |bot| {
        bot.own_pos().unwrap().x - start.x > 4.0
    });
    b.wait_for("B observes A's movement", Duration::from_secs(2), |bot| {
        bot.last_snapshot.get(&a_id).unwrap().x - start.x > 4.0
    });

    // Disconnect B → its player must leave A's snapshots.
    drop(b);
    a.wait_for("bot B's player despawns", Duration::from_secs(5), |bot| {
        !bot.last_snapshot.contains_key(&b_id)
    });
}

// Phase 2: under 150 ms simulated round-trip latency, the clock sync measures
// the inflated RTT, delayed intents still pass the server's arrival-deadline
// validation and move the player, and snapshots acknowledge the full intent
// stream (`last_processed_seq` catches up to the last sent seq).
#[test]
fn phase2_simulated_latency() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25152".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect_with_latency(addr, Duration::from_millis(150));
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    bot.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    let rtt = bot.client.rtt_micros().unwrap();
    assert!(rtt >= 150_000, "simulated latency not measured by clock sync: rtt {rtt} µs");

    let start = bot.own_pos().unwrap();
    let run_until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < run_until {
        bot.send_move(glam::Vec2::new(1.0, 0.0));
        bot.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    bot.send_move(glam::Vec2::ZERO);
    let final_seq = bot.seq;

    bot.wait_for("movement observed despite latency", Duration::from_secs(5), |b| {
        b.own_pos().unwrap().x - start.x > 4.0
    });
    bot.wait_for("server acks the full intent stream", Duration::from_secs(5), |b| {
        b.last_ack == final_seq
    });
}

// Regression test for Finding 2 of docs/reviews/networking/audit-networking-2026-07-11.md:
// an honest client's direction that lands a few ULP over unit length (exactly
// what glam's f32 `normalize()` produces for ordinary camera-yaw inputs) must
// still move the player. A strict `> 1.0` reject silently drops every one of
// these intents (misprediction-causing rubber-banding); the fix tolerates
// epsilon-scale excess and clamps it, like the shared `movement_velocity` rule.
#[test]
fn epsilon_over_unit_direction_still_moves_player() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25167".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    bot.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    // A direction whose length² lands a few ULP over 1.0 — not malicious, just
    // ordinary f32 normalize() noise.
    let dir = glam::Vec2::new(1.0 + f32::EPSILON, 0.0);
    assert!(dir.length_squared() > 1.0, "test setup: dir must be over-unit");
    assert!(dir.length_squared() <= 1.0 + 1e-3, "test setup: dir must be within tolerance");

    let start = bot.own_pos().unwrap();
    let run_until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < run_until {
        bot.send_move(dir);
        bot.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    bot.send_move(glam::Vec2::ZERO);
    bot.wait_for("epsilon-over-unit direction moved the player", Duration::from_secs(2), |b| {
        b.own_pos().unwrap().x - start.x > 4.0
    });
}

// Phase 3: the server-side chapter (waves) spawns NPCs and they replicate to
// clients through AOI enters with their prefab identity.
#[test]
fn phase3_npc_replication() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25153".parse().unwrap();
    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        app.add_plugin(chapter_01::Chapter01Plugin);
        app.run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("grunts replicate", Duration::from_secs(5), |b| {
        b.prefabs.values().any(|p| p == "grunt")
    });
}

// Regression test for Finding 5 of docs/reviews/networking/audit-networking-2026-07-11.md:
// MechanicScheduled/HitResult used to `broadcast` to EVERY connection
// regardless of distance — a cheating client got a zone-wide radar off
// telegraph positions, and aggregate mechanic traffic scaled O(players ×
// casts). The fix scopes both sends to the same AOI_RADIUS the snapshot
// system already uses, so a bot far outside the caster's AOI must never
// receive either message for that mechanic.
#[test]
fn far_bot_never_sees_out_of_aoi_mechanic() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25164".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(2400));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut a = Bot::connect(addr);
    a.wait_for("A welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    a.wait_for("A clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    a.wait_for("A first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    let mut far = Bot::connect_as(addr, "far-observer");
    far.wait_for("far welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    far.wait_for("far clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    far.wait_for("far first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    // Both bots ring-spawn within 3 units of the origin; walk the observer
    // well past AOI_RADIUS (40) so it can no longer see A at all.
    far.walk_until("far bot clears A's AOI", glam::Vec2::new(1.0, 0.0), Duration::from_secs(15), |b| {
        b.own_pos().is_some_and(|p| p.x > 55.0)
    });

    // A casts "cleave" centered on itself — well inside A's own AOI, and far
    // outside the observer's.
    let a_pos = a.own_pos().unwrap();
    a.send_cast("cleave", glam::Vec2::new(a_pos.x, a_pos.z));
    a.wait_for("A gets its own mechanic schedule", Duration::from_secs(3), |bot| !bot.mechanics.is_empty());
    let (mech, _) = a.mechanics[0];
    a.wait_for("A gets the hit result", Duration::from_secs(4), |bot| bot.hit_results.contains_key(&mech));

    // The far bot must never receive either message for this mechanic.
    settle(&mut far, Duration::from_millis(500));
    assert!(far.mechanics.is_empty(), "far bot must not see an out-of-AOI MechanicScheduled");
    assert!(far.hit_results.is_empty(), "far bot must not see an out-of-AOI HitResult");
}

// Phase 5: world clock + scripted world event. The blood moon fires at world
// time 2 s; every connected client carries the same world-clock mapping; the
// event's spawns replicate to everyone, including a client that joins
// mid-event (state reconstruction = clock + AOI, by construction).
#[test]
fn phase5_world_clock_and_blood_moon() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25157".parse().unwrap();
    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        // The grunt prefab lives in chapter-01's prefab dir (registrations
        // only — no chapter waves in this test).
        app.add_plugin(chapter_01::Chapter01ContentPlugin);
        app.insert_resource(vordar_game::world::WorldEventsDef {
            day_seconds: 60.0,
            events: vec![vordar_game::world::WorldEventDef {
                name: "blood_moon".into(),
                start_seconds_of_day: 2.0,
                duration_seconds: 30.0,
                ambient: glam::Vec3::new(0.7, 0.08, 0.08),
                spawns: vec![vordar_game::world::WorldSpawn {
                    prefab: "grunt".into(),
                    positions: vec![
                        glam::Vec3::new(10.0, 0.0, 0.0),
                        glam::Vec3::new(0.0, 0.0, 10.0),
                        glam::Vec3::new(-10.0, 0.0, 0.0),
                    ],
                }],
            }],
        });
        app.run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let grunt_count = |b: &Bot| b.prefabs.values().filter(|p| *p == "grunt").count();

    let mut a = Bot::connect(addr);
    let mut b = Bot::connect(addr);
    a.wait_for("A world clock", Duration::from_secs(5), |b| b.world_offset.is_some());
    b.wait_for("B world clock", Duration::from_secs(5), |b| b.world_offset.is_some());
    // One authoritative clock: every client got the same mapping.
    assert_eq!(a.world_offset, b.world_offset);

    // The blood moon spawns reach both clients.
    a.wait_for("A sees blood-moon grunts", Duration::from_secs(8), |bot| grunt_count(bot) >= 3);
    b.wait_for("B sees blood-moon grunts", Duration::from_secs(8), |bot| grunt_count(bot) >= 3);

    // Mid-event joiner: clock mapping + already-spawned entities arrive at once.
    let mut c = Bot::connect(addr);
    c.wait_for("C world clock", Duration::from_secs(5), |b| b.world_offset.is_some());
    assert_eq!(c.world_offset, a.world_offset);
    c.wait_for("C sees the in-progress event's grunts", Duration::from_secs(8), |bot| {
        grunt_count(bot) >= 3
    });
}

/// Kills every player once at tick 120 (~2 s) — a deterministic stand-in for
/// dying to enemies.
struct KillPlayersSystem {
    ticks: u64,
    fired: bool,
}

impl System for KillPlayersSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        self.ticks += 1;
        if self.fired || self.ticks < 120 {
            return;
        }
        self.fired = true;
        let players: Vec<Entity> = world
            .query::<(Entity, &vordar_game::Player)>()
            .iter()
            .map(|(e, _)| e)
            .collect();
        for entity in players {
            resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
        }
    }
}

// Phase 3: a connection always owns a live player. When combat kills the
// entity, the server respawns it and re-Welcomes the client; the old body
// leaves the AOI stream.
#[test]
fn phase3_respawn_after_death() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25155".parse().unwrap();
    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        app.add_system(KillPlayersSystem { ticks: 0, fired: false }, Phase::Update, SystemOrder::Default);
        app.run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    let first_body = bot.player_id.unwrap();

    bot.wait_for("re-welcome after death", Duration::from_secs(10), |b| {
        b.player_id != Some(first_body)
    });
    bot.wait_for("respawned player replicates", Duration::from_secs(5), |b| b.own_pos().is_some());
    bot.wait_for("old body leaves the AOI", Duration::from_secs(5), |b| {
        !b.last_snapshot.contains_key(&first_body)
    });
}

// Phase 3: AOI border behavior + bandwidth. 100 NPCs sit within radius ~19 of
// the origin (always inside the bot's 40-unit AOI for this walk), one far NPC
// sits at x=58 — outside AOI from spawn, inside after walking east, outside
// again after walking back.
#[test]
fn phase3_aoi_border() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25154".parse().unwrap();

    let mut positions: Vec<glam::Vec3> = Vec::new();
    for i in 0..10 {
        for j in 0..10 {
            positions.push(glam::Vec3::new(-13.5 + 3.0 * i as f32, 0.0, -13.5 + 3.0 * j as f32));
        }
    }
    positions.push(glam::Vec3::new(58.0, 0.0, 0.0));

    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        app.add_system(PopulateSystem { done: false, positions, prefab: "player".into() }, Phase::PreUpdate, SystemOrder::First);
        app.run_headless(60.0, Some(1500));
    });
    std::thread::sleep(Duration::from_millis(300));

    let far_visible = |b: &Bot| b.last_snapshot.values().any(|p| p.x > 50.0);

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("nearby NPCs visible", Duration::from_secs(5), |b| {
        b.last_snapshot.len() >= 101 // 100 NPCs + own player
    });
    assert!(!far_visible(&bot), "far NPC must start outside the AOI");

    // Bandwidth with ~101 entities in the AOI at SNAPSHOT_HZ — budget 20 KB/s.
    bot.bytes = 0;
    let measure_until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < measure_until {
        bot.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(bot.bytes < 40_000, "bandwidth over budget: {} bytes in 2 s", bot.bytes);

    // Walk east — the far NPC must enter the AOI...
    bot.walk_until("far NPC enters AOI", glam::Vec2::new(1.0, 0.0), Duration::from_secs(10), |b| {
        b.own_pos().is_some_and(|p| p.x > 19.5) && far_visible(b)
    });
    // ...and leave it again on the way back.
    bot.walk_until("far NPC leaves AOI", glam::Vec2::new(-1.0, 0.0), Duration::from_secs(10), |b| {
        b.own_pos().is_some_and(|p| p.x < 5.0) && !far_visible(b)
    });
    // The near field never left.
    assert!(bot.last_snapshot.len() >= 101, "near NPCs flapped out of the AOI");
}
