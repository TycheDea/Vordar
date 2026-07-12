// Phase 7 e2e: multi-zone server, portal transfer handoff, login routing,
// shared world clock, and snapshot throttling under crowd load.

mod common;

use common::{temp_db, test_zones, walk_into_portal, workspace_root, Bot, PopulateSystem};
use engine_app::scheduler::{Phase, SystemOrder};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_game::zones::validate_zones;
use vordar_server::db::DbWorker;
use vordar_server::build_zone_app;

/// Spawn one zone App per thread (built ON its thread — Apps don't move),
/// sharing one DB worker and one world-time origin, exactly like main.rs.
fn spawn_zone_server(start_addr: SocketAddr, east_addr: SocketAddr, db_path: &str, ticks: u64) {
    let directory: HashMap<String, SocketAddr> =
        HashMap::from([("start".to_owned(), start_addr), ("east".to_owned(), east_addr)]);
    let worker = DbWorker::spawn(db_path).expect("db open");
    let world_origin = Instant::now();
    for zone in test_zones() {
        let addr = directory[&zone.name];
        let directory = directory.clone();
        let handle = worker.handle();
        std::thread::spawn(move || {
            build_zone_app(addr, handle, zone, directory, world_origin).run_headless(60.0, Some(ticks));
        });
    }
    // The worker must outlive the zone threads; its Drop would block this
    // test until every zone burns through its tick budget. Saves are
    // processed promptly while running — only shutdown flushing is lost,
    // and no Phase 7 test depends on it.
    std::mem::forget(worker);
    std::thread::sleep(Duration::from_millis(300));
}

// The shipped topology must satisfy the same structural rules the test
// topology does — main.rs validates it at startup and panics otherwise.
#[test]
fn shipped_zone_content_is_valid() {
    workspace_root();
    let zones = vordar_game::zones::load_zones("content/zones/zones.ron");
    validate_zones(&zones).unwrap();
    assert_eq!(zones.zones[0].name, "start", "clients connect to the first zone");
}

// The roadmap's Verify line, headless: walk into start's portal → Redirect →
// reconnect to east at the arrival point → walk into east's portal → back in
// start at ITS arrival point.
#[test]
fn phase7_portal_round_trip() {
    workspace_root();
    let start_addr: SocketAddr = "127.0.0.1:25170".parse().unwrap();
    let east_addr: SocketAddr = "127.0.0.1:25171".parse().unwrap();
    spawn_zone_server(start_addr, east_addr, ":memory:", 3600);

    let mut bot = Bot::connect_as(start_addr, "walker");
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    walk_into_portal(&mut bot, Vec3::new(10.0, 0.0, 0.0), Duration::from_secs(10));
    let (zone, addr) = bot.redirect.clone().unwrap();
    assert_eq!(zone, "east");
    assert_eq!(addr, east_addr);

    bot.follow_redirect();
    bot.wait_for("welcome in east", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("snapshot in east", Duration::from_secs(5), |b| b.own_pos().is_some());
    let arrived = bot.own_pos().unwrap();
    assert!(
        arrived.distance(Vec3::new(-6.0, 0.0, 0.0)) < 1.0,
        "must arrive at east's portal target, got {arrived}"
    );

    walk_into_portal(&mut bot, Vec3::new(-10.0, 0.0, 0.0), Duration::from_secs(10));
    let (zone, addr) = bot.redirect.clone().unwrap();
    assert_eq!(zone, "start");
    assert_eq!(addr, start_addr);

    bot.follow_redirect();
    bot.wait_for("welcome back in start", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("snapshot back in start", Duration::from_secs(5), |b| b.own_pos().is_some());
    let back = bot.own_pos().unwrap();
    assert!(
        back.distance(Vec3::new(6.0, 0.0, 0.0)) < 1.0,
        "round trip must end at start's arrival point, got {back}"
    );
}

// The impolite path (lessons.md): client killed right after the Redirect,
// never reaching the target zone. The transfer save must already be durable
// (zone + arrival pos in the DB), the late Disconnected must not clobber it,
// and a relogin to the WRONG zone routes to the right one without a Welcome.
#[test]
fn phase7_login_routes_to_saved_zone() {
    workspace_root();
    let start_addr: SocketAddr = "127.0.0.1:25172".parse().unwrap();
    let east_addr: SocketAddr = "127.0.0.1:25173".parse().unwrap();
    let db = temp_db("zone-routing");
    spawn_zone_server(start_addr, east_addr, &db, 3600);

    let mut ghost = Bot::connect_as(start_addr, "ghost");
    ghost.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    ghost.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    walk_into_portal(&mut ghost, Vec3::new(10.0, 0.0, 0.0), Duration::from_secs(10));
    // Killed mid-transfer: never connects to east.
    drop(ghost);

    // The transfer save reaches the file, and the abrupt disconnect that
    // follows must not overwrite it (no PlayerConn left in start).
    std::thread::sleep(Duration::from_millis(700));
    let (zone, x): (String, f64) = rusqlite::Connection::open(&db)
        .unwrap()
        .query_row("SELECT zone, pos_x FROM characters WHERE name = 'ghost'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(zone, "east", "transfer must persist the target zone");
    assert!((x - -6.0).abs() < 0.01, "transfer must persist the arrival point, got x={x}");

    // Relaunch hits start (clients always dial the base address): start
    // doesn't own the character, so it must redirect WITHOUT spawning.
    let mut ghost = Bot::connect_as(start_addr, "ghost");
    ghost.wait_for("routing redirect", Duration::from_secs(5), |b| b.redirect.is_some());
    assert_eq!(ghost.player_id, None, "wrong zone must not Welcome");
    assert_eq!(ghost.redirect.as_ref().unwrap().0, "east");

    ghost.follow_redirect();
    ghost.wait_for("welcome in east", Duration::from_secs(5), |b| b.player_id.is_some());
    ghost.wait_for("snapshot in east", Duration::from_secs(5), |b| b.own_pos().is_some());
    let pos = ghost.own_pos().unwrap();
    assert!(
        pos.distance(Vec3::new(-6.0, 0.0, 0.0)) < 1.0,
        "must spawn at the persisted transfer position, got {pos}"
    );
}

// Chapter 2's town (shipped zones.ron: east = chapter02): buildings and
// villagers replicate as ordinary prefab entities, a villager can't be hit
// by an AOE centered on it (no Health = immune by construction, the pin for
// that design), and the monster camps prowl outside town.
#[test]
fn town_zone_replicates_and_villagers_are_unhittable() {
    workspace_root();
    let start_addr: SocketAddr = "127.0.0.1:25177".parse().unwrap();
    let east_addr: SocketAddr = "127.0.0.1:25178".parse().unwrap();

    // Like spawn_zone_server, but east runs the town chapter — installed the
    // same way main.rs installs zone chapters.
    let mut zones = test_zones();
    zones[1].chapter = Some("chapter02".into());
    let directory: HashMap<String, SocketAddr> =
        HashMap::from([("start".to_owned(), start_addr), ("east".to_owned(), east_addr)]);
    let worker = DbWorker::spawn(":memory:").expect("db open");
    let world_origin = Instant::now();
    for zone in zones {
        let addr = directory[&zone.name];
        let directory = directory.clone();
        let handle = worker.handle();
        std::thread::spawn(move || {
            let chapter = zone.chapter.clone();
            let mut app = build_zone_app(addr, handle, zone, directory, world_origin);
            if let Some(name) = chapter.as_deref() {
                vordar_game::chapter::ChapterRegistry::new(vec![chapter_01::module(), chapter_02::module()])
                    .install(name, &mut app)
                    .unwrap();
            }
            app.run_headless(60.0, Some(3600));
        });
    }
    std::mem::forget(worker);
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect_as(start_addr, "traveler");
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    walk_into_portal(&mut bot, Vec3::new(10.0, 0.0, 0.0), Duration::from_secs(10));
    bot.follow_redirect();
    bot.wait_for("welcome in east", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("clock sync in east", Duration::from_secs(5), |b| {
        b.client.server_offset_micros().is_some()
    });
    bot.wait_for("the town replicates", Duration::from_secs(10), |b| {
        let has = |p: &str| b.prefabs.values().any(|v| v == p);
        has("town_hall") && has("npc_villager") && has("npc_elder") && has("cottage")
    });
    bot.wait_for("camps replicate outside town", Duration::from_secs(10), |b| {
        let has = |p: &str| b.prefabs.values().any(|v| v == p);
        has("grunt") && has("brigand")
    });

    // Walk into cleave range of a villager and drop the AOE on its head.
    let npc_id = *bot.prefabs.iter().find(|(_, p)| *p == "npc_villager").unwrap().0;
    let npc = *bot.last_snapshot.get(&npc_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(Instant::now() < deadline, "never reached the villager");
        let own = bot.own_pos().unwrap();
        let offset = glam::Vec2::new(npc.x - own.x, npc.z - own.z);
        if offset.length() < 6.0 {
            bot.send_move(glam::Vec2::ZERO);
            break;
        }
        bot.send_move(offset.normalize());
        bot.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    // Finding 8 of docs/reviews/audit-networking-2026-07-11.md: the zone
    // transfer re-spawned this connection's PlayerConn, which now
    // pessimistically starts abilities on full cooldown — clear it first.
    std::thread::sleep(Duration::from_millis(3200));
    bot.send_cast("cleave", glam::Vec2::new(npc.x, npc.z));
    bot.wait_for("cleave schedules", Duration::from_secs(3), |b| !b.mechanics.is_empty());
    let (mech, _) = *bot.mechanics.last().unwrap();
    bot.wait_for("cleave resolves", Duration::from_secs(4), |b| b.hit_results.contains_key(&mech));
    assert!(
        !bot.hit_results[&mech].contains(&npc_id),
        "villagers have no Health and must be unhittable by construction"
    );
    assert!(bot.last_snapshot.contains_key(&npc_id), "the villager still stands");
}

// Every zone shares one world-time origin: two clients in two different
// zones must compute the same absolute world time (their per-zone server
// clocks differ; the WorldClock mapping absorbs that).
#[test]
fn phase7_world_clock_shared_across_zones() {
    workspace_root();
    let start_addr: SocketAddr = "127.0.0.1:25174".parse().unwrap();
    let east_addr: SocketAddr = "127.0.0.1:25175".parse().unwrap();
    spawn_zone_server(start_addr, east_addr, ":memory:", 3600);

    let mut a = Bot::connect_as(start_addr, "stay");
    a.wait_for("A welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    a.wait_for("A world clock", Duration::from_secs(5), |b| b.world_offset.is_some());
    a.wait_for("A clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());

    // B reaches east the honest way: through the portal.
    let mut b = Bot::connect_as(start_addr, "rover");
    b.wait_for("B welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    b.wait_for("B first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    walk_into_portal(&mut b, Vec3::new(10.0, 0.0, 0.0), Duration::from_secs(10));
    b.follow_redirect();
    b.wait_for("B welcome in east", Duration::from_secs(5), |b| b.player_id.is_some());
    b.wait_for("B world clock", Duration::from_secs(5), |b| b.world_offset.is_some());
    b.wait_for("B clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());

    let world_a = a.client.server_now_micros().unwrap() as i64 + a.world_offset.unwrap();
    let world_b = b.client.server_now_micros().unwrap() as i64 + b.world_offset.unwrap();
    let skew = (world_a - world_b).abs();
    assert!(
        skew < 100_000,
        "zones disagree on world time by {} ms",
        skew / 1000
    );
}

// Crowd throttling: with ~150 entities in the AOI, per-snapshot `states` is
// capped — bandwidth stays bounded — while the round-robin still refreshes
// every entity within a short window.
#[test]
fn phase7_snapshot_throttle() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25176".parse().unwrap();

    // 150 stationary NPCs in a grid, all well inside the 40-unit AOI.
    let mut positions: Vec<Vec3> = Vec::new();
    for i in 0..15 {
        for j in 0..10 {
            positions.push(Vec3::new(-21.0 + 3.0 * i as f32, 0.0, -13.5 + 3.0 * j as f32));
        }
    }

    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        app.add_system(PopulateSystem { done: false, positions }, Phase::PreUpdate, SystemOrder::First);
        app.run_headless(60.0, Some(1500));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("crowd visible", Duration::from_secs(5), |b| b.last_snapshot.len() >= 151);

    // Bandwidth AND freshness over the same 2 s window. Untrottled, 151
    // entities at 10 Hz would be ~51 KB; the 64-state cap keeps it under 25
    // KB/s. Freshness: every known id must appear in `states` within the
    // window (the round-robin covers the 119-entity pool in ~4 snapshots).
    bot.bytes = 0;
    let mut refreshed: HashSet<u64> = HashSet::new();
    let measure_until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < measure_until {
        bot.pump();
        refreshed.extend(bot.last_states.iter().copied());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(bot.bytes < 50_000, "throttled bandwidth over budget: {} bytes in 2 s", bot.bytes);
    let known: HashSet<u64> = bot.last_snapshot.keys().copied().collect();
    let stale: Vec<u64> = known.difference(&refreshed).copied().collect();
    assert!(stale.is_empty(), "{} entities never refreshed in 2 s", stale.len());
}
