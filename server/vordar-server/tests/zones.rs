// Phase 7 e2e: multi-zone server, portal transfer handoff, login routing,
// shared world clock, and snapshot throttling under crowd load.

mod common;

use common::{temp_db, workspace_root, Bot, PopulateSystem};
use engine_app::scheduler::{Phase, SystemOrder};
use glam::{Vec2, Vec3};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_game::zones::{validate_zones, PortalDef, ZoneDef, ZonesDef};
use vordar_server::db::DbWorker;
use vordar_server::build_zone_app;

/// Compact two-zone topology so walks stay short: start's portal at x=10
/// drops you at x=-6 in east; east's portal at x=-10 sends you back to x=6.
fn test_zones() -> Vec<ZoneDef> {
    let zones = vec![
        ZoneDef {
            name: "start".into(),
            chapter: None,
            portals: vec![PortalDef {
                pos: Vec3::new(10.0, 0.0, 0.0),
                radius: 2.0,
                target_zone: "east".into(),
                target_pos: Vec3::new(-6.0, 0.0, 0.0),
            }],
        },
        ZoneDef {
            name: "east".into(),
            chapter: None,
            portals: vec![PortalDef {
                pos: Vec3::new(-10.0, 0.0, 0.0),
                radius: 2.0,
                target_zone: "start".into(),
                target_pos: Vec3::new(6.0, 0.0, 0.0),
            }],
        },
    ];
    validate_zones(&ZonesDef { zones: zones.clone() }).unwrap();
    zones
}

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

/// Steer toward `portal` (spawn points sit on a ring, so a straight east
/// walk can miss the 2-unit radius) until the server redirects us.
fn walk_into_portal(bot: &mut Bot, portal: Vec3, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        bot.pump();
        if bot.redirect.is_some() {
            return;
        }
        if let Some(pos) = bot.own_pos() {
            let d = portal - pos;
            let dir = Vec2::new(d.x, d.z);
            if dir.length_squared() > 1e-6 {
                bot.send_move(dir.normalize());
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    panic!("timed out walking into the portal");
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
