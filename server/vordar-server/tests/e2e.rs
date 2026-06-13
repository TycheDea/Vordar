// End-to-end smokes, fully headless: a server zone App on a loopback QUIC
// endpoint, bot clients speaking the real protocol (engine-net directly, no
// renderer).
//
//   phase1_end_to_end        — connect, clock sync, mutual visibility,
//                              movement replication, disconnect despawn
//   phase2_simulated_latency — prediction acks + intent validation at 150 ms
//   phase3_npc_replication   — chapter waves replicate NPCs to clients
//   phase3_aoi_border        — AOI enter/leave at the radius border + bandwidth
//   phase6_*                 — character persistence: reconnect restores
//                              position, damage reaches the DB, restart durability

mod common;

use common::{settle, temp_db, workspace_root, Bot, PopulateSystem};
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use hecs::Entity;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_protocol::{encode, ClientMsg};

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

    // ── Cast 1: B stands still inside the area → hit (caster A excluded). ──
    let b_pos = *a.last_snapshot.get(&b_id).unwrap();
    let target = glam::Vec2::new(b_pos.x, b_pos.z);
    a.send_cast("blast", target);
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
    a.send_cast("blast", glam::Vec2::new(b_pos.x, b_pos.z));
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
        skill: "blast".into(),
        target: glam::Vec2::ZERO,
    }));
    std::thread::sleep(Duration::from_millis(1500));
    b.pump();
    assert_eq!(b.mechanics.len(), count_before, "backdated cast must be rejected");
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
        app.add_system(PopulateSystem { done: false, positions }, Phase::PreUpdate, SystemOrder::First);
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

// Phase 7.5: the player's left-click projectile, end to end. A camp-resident
// grunt replicates into the bot's AOI; the bot fires "bolt" casts at it. The
// bolt entity itself must replicate (it's an ordinary prefab entity), and
// three hits (3 × 12 ≥ 30 HP) must kill the grunt — observed as an AOI leave
// while the bot stays alive (player_id never changes → no death re-Welcome).
#[test]
fn phase7_5_bolt_kills_camped_enemy() {
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

    // Fire at the grunt's latest known position until it dies. It will charge
    // us once provoked — aiming at its current spot still works because it
    // approaches along the firing line.
    let mut saw_bolt = false;
    let mut last_cast = Instant::now() - Duration::from_secs(1);
    let deadline = Instant::now() + Duration::from_secs(20);
    while bot.last_snapshot.contains_key(&grunt_id) {
        assert!(Instant::now() < deadline, "grunt survived 20 s of bolts");
        if last_cast.elapsed() > Duration::from_millis(700) {
            if let Some(pos) = bot.last_snapshot.get(&grunt_id).copied() {
                bot.send_cast("bolt", glam::Vec2::new(pos.x, pos.z));
                last_cast = Instant::now();
            }
        }
        bot.pump();
        saw_bolt |= bot.prefabs.values().any(|p| p == "bolt");
        std::thread::sleep(Duration::from_millis(16));
    }

    assert!(saw_bolt, "the bolt projectile must replicate like any entity");
    assert_eq!(bot.player_id, Some(original_body), "the bot must survive the fight");
}

// Phase 6: disconnect saves the character; reconnecting with the same name
// restores the saved position; a fresh name gets a ring spawn instead.
#[test]
fn phase6_reconnect_restores_position() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25158".parse().unwrap();
    let db = temp_db("reconnect");
    let server_db = db.clone();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, &server_db).run_headless(60.0, Some(2400));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut alice = Bot::connect_as(addr, "alice");
    alice.wait_for("alice welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    alice.walk_until("alice walks east", glam::Vec2::new(1.0, 0.0), Duration::from_secs(10), |b| {
        b.own_pos().is_some_and(|p| p.x > 6.0)
    });
    settle(&mut alice, Duration::from_millis(300));
    let saved = alice.own_pos().unwrap();
    drop(alice);
    // Give the server a moment to process the disconnect (it both saves the
    // character and frees the name for the next login).
    std::thread::sleep(Duration::from_millis(500));

    let mut alice = Bot::connect_as(addr, "alice");
    alice.wait_for("alice re-welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    alice.wait_for("alice snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    let restored = alice.own_pos().unwrap();
    assert!(
        restored.distance(saved) < 1.0,
        "reconnect must restore the saved position: saved {saved}, got {restored}"
    );

    // A name never seen before spawns on the ring near the origin.
    let mut bob = Bot::connect_as(addr, "bob");
    bob.wait_for("bob welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bob.wait_for("bob snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    let bob_pos = bob.own_pos().unwrap();
    assert!(bob_pos.length() < 4.0, "fresh character must ring-spawn, got {bob_pos}");
    assert!(bob_pos.distance(saved) > 2.0, "fresh character must not inherit alice's spot");
}

// Phase 6: health persists. Health never rides the wire, so the assertion
// reads the test database directly after the victim disconnects.
#[test]
fn phase6_health_persists_in_db() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25159".parse().unwrap();
    let db = temp_db("health");
    let server_db = db.clone();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, &server_db).run_headless(60.0, Some(2400));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut atk = Bot::connect_as(addr, "atk");
    atk.wait_for("atk welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    atk.wait_for("atk clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    let mut victim = Bot::connect_as(addr, "victim");
    victim.wait_for("victim welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    let victim_id = victim.player_id.unwrap();
    atk.wait_for("atk sees victim", Duration::from_secs(5), |b| b.last_snapshot.contains_key(&victim_id));

    // Blast the stationary victim (25 damage, resolves 2 s after the cast).
    let vp = *atk.last_snapshot.get(&victim_id).unwrap();
    atk.send_cast("blast", glam::Vec2::new(vp.x, vp.z));
    atk.wait_for("hit lands on victim", Duration::from_secs(6), |b| {
        b.hit_results.values().any(|hits| hits.contains(&victim_id))
    });
    // Keep the victim's connection pumping so the QUIC session stays healthy,
    // then disconnect — the server saves position + health.
    settle(&mut victim, Duration::from_millis(200));
    drop(victim);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let health: Option<i32> = rusqlite::Connection::open(&db).ok().and_then(|conn| {
            conn.query_row("SELECT health FROM characters WHERE name = 'victim'", [], |r| r.get(0)).ok()
        });
        if health.is_some_and(|h| h < 100) {
            assert_eq!(health, Some(75), "one blast = 25 damage");
            break;
        }
        assert!(Instant::now() < deadline, "victim's damaged health never reached the db: {health:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

// Phase 6: relogin while the old session still looks alive (crashed client,
// close frame lost, quick relaunch) must TAKE OVER, not hang: the new
// connection gets Welcome + the freshest saved state, the old body despawns,
// the old connection is kicked.
#[test]
fn phase6_login_takeover() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25162".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(2400));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut first = Bot::connect_as(addr, "dup");
    first.wait_for("first welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    first.walk_until("first walks east", glam::Vec2::new(1.0, 0.0), Duration::from_secs(10), |b| {
        b.own_pos().is_some_and(|p| p.x > 6.0)
    });
    settle(&mut first, Duration::from_millis(300));
    let saved = first.own_pos().unwrap();
    let old_body = first.player_id.unwrap();

    // Deliberately NOT dropped: the old session is still online when the
    // second login lands.
    let mut second = Bot::connect_as(addr, "dup");
    second.wait_for("takeover welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    second.wait_for("takeover snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    assert_ne!(second.player_id, Some(old_body), "takeover must spawn a new body");
    let restored = second.own_pos().unwrap();
    assert!(
        restored.distance(saved) < 1.0,
        "takeover must restore the old session's position: saved {saved}, got {restored}"
    );
    assert!(
        !second.last_snapshot.contains_key(&old_body),
        "the old body must be despawned"
    );
    drop(first);
}

// Phase 6: durability across a server restart. Server 1 shuts down cleanly
// (DbWorker drop flushes queued saves); server 2 opens the same database and
// must restore the character.
#[test]
fn phase6_restart_durability() {
    workspace_root();
    let addr1: SocketAddr = "127.0.0.1:25160".parse().unwrap();
    let addr2: SocketAddr = "127.0.0.1:25161".parse().unwrap();
    let db = temp_db("restart");

    let server_db = db.clone();
    let server1 = std::thread::spawn(move || {
        vordar_server::build_server_app(addr1, &server_db).run_headless(60.0, Some(600));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut carol = Bot::connect_as(addr1, "carol");
    carol.wait_for("carol welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    carol.walk_until("carol walks east", glam::Vec2::new(1.0, 0.0), Duration::from_secs(8), |b| {
        b.own_pos().is_some_and(|p| p.x > 6.0)
    });
    settle(&mut carol, Duration::from_millis(300));
    let saved = carol.own_pos().unwrap();
    drop(carol);
    // Wait out server 1's tick budget; App drop joins the DbWorker, which
    // drains the queued disconnect-save into the file.
    server1.join().unwrap();

    let server_db = db.clone();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr2, &server_db).run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut carol = Bot::connect_as(addr2, "carol");
    carol.wait_for("carol re-welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    carol.wait_for("carol snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    let restored = carol.own_pos().unwrap();
    assert!(
        restored.distance(saved) < 1.0,
        "restart must restore the saved position: saved {saved}, got {restored}"
    );
}
