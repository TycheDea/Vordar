// Persistence tests: character saves, reconnection, restarts, cooldown remainders.
// Isolated from connectivity, combat, and wire-format concerns.

use test_support::{settle, spawn_server, temp_db, workspace_root, Bot};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

// Disconnect saves the character; reconnecting with the same name
// restores the saved position; a fresh name gets a ring spawn instead.
#[test]
fn reconnect_restores_position() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25158".parse().unwrap();
    let db = temp_db("reconnect");
    let server_db = db.clone();
    spawn_server(addr, &server_db, 2400);

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

// Health persists. Health never rides the wire, so the assertion
// reads the test database directly after the victim disconnects.
#[test]
fn health_persists_in_db() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25159".parse().unwrap();
    let db = temp_db("health");
    let server_db = db.clone();
    spawn_server(addr, &server_db, 2400);

    let mut atk = Bot::connect_as(addr, "atk");
    atk.wait_for("atk welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    atk.wait_for("atk clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    let mut victim = Bot::connect_as(addr, "victim");
    victim.wait_for("victim welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    let victim_id = victim.player_id.unwrap();
    atk.wait_for("atk sees victim", Duration::from_secs(5), |b| b.last_snapshot.contains_key(&victim_id));

    // Every fresh spawn pessimistically starts its abilities on full
    // cooldown, so "cleave" isn't castable in the instant after login —
    // clear it first.
    std::thread::sleep(Duration::from_millis(3200));

    // Cleave the stationary victim (30 base + the Ravager's power, possibly a
    // crit — resolves 2 s after the cast).
    let vp = *atk.last_snapshot.get(&victim_id).unwrap();
    atk.send_cast("cleave", glam::Vec2::new(vp.x, vp.z));
    atk.wait_for("hit lands on victim", Duration::from_secs(6), |b| {
        b.hit_results.values().any(|hits| hits.contains(&victim_id))
    });
    // Keep the victim's connection pumping so the QUIC session stays healthy,
    // then disconnect — the server saves position + health.
    settle(&mut victim, Duration::from_millis(200));
    drop(victim);

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let health: Option<i32> = rusqlite::Connection::open(&db).ok().and_then(|conn| {
            conn.query_row("SELECT health FROM characters WHERE name = 'victim'", [], |r| r.get(0)).ok()
        });
        if health.is_some_and(|h| h < 100) {
            // The point is persistence, not the formula (stats.rs unit-tests
            // that): damaged, alive, and the damage magnitude is one cleave's.
            let h = health.unwrap();
            assert!((100 - 54..100).contains(&h), "one cleave's damage expected, got {h}");
            break;
        }
        assert!(Instant::now() < deadline, "victim's damaged health never reached the db: {health:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

// A hostile or corrupt `characters.health` row must never grant a state
// better than a legitimate one, nor spawn the character already dead — the
// stored value clamps into the live prefab's own `[1, max]` bounds.
#[test]
fn tampered_health_clamps_into_prefab_bounds() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25185".parse().unwrap();
    let db = temp_db("tampered-health");
    let server_db = db.clone();
    spawn_server(addr, &server_db, 2400);

    // The row must exist (and pass the accounts FK) before it can be
    // tampered with directly, so log in once and disconnect first.
    let mut setup = Bot::connect_as(addr, "clampme");
    setup.wait_for("clampme welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    drop(setup);
    std::thread::sleep(Duration::from_millis(500));

    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE characters SET health = -50 WHERE name = 'clampme'", []).unwrap();
    }
    let mut low = Bot::connect_as(addr, "clampme");
    low.wait_for("clampme re-welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    let low_id = low.player_id.unwrap();
    low.wait_for("clampme hp replicates", Duration::from_secs(5), |b| b.last_hp.contains_key(&low_id));
    assert_eq!(
        low.last_hp.get(&low_id),
        Some(&1),
        "a negative stored health must clamp to the floor (1), not spawn already dead"
    );
    drop(low);
    std::thread::sleep(Duration::from_millis(500));

    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE characters SET health = 999999 WHERE name = 'clampme'", []).unwrap();
    }
    let mut high = Bot::connect_as(addr, "clampme");
    high.wait_for("clampme re-re-welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    let high_id = high.player_id.unwrap();
    high.wait_for("clampme hp replicates again", Duration::from_secs(5), |b| b.last_hp.contains_key(&high_id));
    assert_eq!(
        high.last_hp.get(&high_id),
        Some(&100),
        "a stored health above the prefab's max must clamp down to it, not overheal"
    );
}

// A corrupt `characters.cooldowns` blob loses its true remainders, so the
// spawn path must fail toward the attacker's disadvantage: every one of the
// class's abilities locks out for its full cooldown, rather than the parse
// error's empty map being read as "nothing on cooldown".
#[test]
fn corrupt_cooldown_blob_locks_out_every_ability_at_spawn() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25186".parse().unwrap();
    let db = temp_db("corrupt-cooldown");
    let server_db = db.clone();
    spawn_server(addr, &server_db, 2400);

    let mut setup = Bot::connect_as(addr, "corruptcd");
    setup.wait_for("corruptcd welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    drop(setup);
    std::thread::sleep(Duration::from_millis(500));

    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE characters SET cooldowns = 'not valid ron' WHERE name = 'corruptcd'", []).unwrap();
    }

    let mut bot = Bot::connect_as(addr, "corruptcd");
    bot.wait_for("corruptcd re-welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("corruptcd re-snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    let pos = bot.own_pos().unwrap();
    bot.send_cast("onslaught", glam::Vec2::new(pos.x, pos.z));

    // A fresh (non-corrupt) character can cast "onslaught" immediately after
    // spawn — see `relog_restores_exact_cooldown_remainder` — so an empty
    // window here is only explained by every ability starting locked out.
    let settle_until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < settle_until {
        bot.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        bot.mechanics.is_empty(),
        "a corrupt cooldown blob must lock every ability out at spawn, not reset it to castable"
    );
}

// Relogin while the old session still looks alive (crashed client,
// close frame lost, quick relaunch) must TAKE OVER, not hang: the new
// connection gets Welcome + the freshest saved state, the old body despawns,
// the old connection is kicked.
#[test]
fn login_takeover() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25162".parse().unwrap();
    spawn_server(addr, ":memory:", 2400);

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

// Durability across a server restart. Server 1 shuts down cleanly
// (DbWorker drop flushes queued saves); server 2 opens the same database and
// must restore the character.
#[test]
fn restart_durability() {
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

// Cooldowns persist as `ready_at` remainders, so a relog restores the EXACT
// remaining cooldown — neither a free reset (no persisted state at all) nor
// a full pessimistic reset (every ability back to full cooldown).
#[test]
fn relog_restores_exact_cooldown_remainder() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25165".parse().unwrap();
    let db = temp_db("cooldown-relog");
    let server_db = db.clone();
    spawn_server(addr, &server_db, 2400);

    let mut alice = Bot::connect_as(addr, "alice");
    alice.wait_for("alice welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    alice.wait_for("alice clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    alice.wait_for("alice first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    // A fresh character's cooldowns start empty — "onslaught" (8 s cooldown,
    // 12-unit range, target own position, well within range) is castable
    // immediately, no pessimistic wait needed.
    let pos = alice.own_pos().unwrap();
    alice.send_cast("onslaught", glam::Vec2::new(pos.x, pos.z));
    alice.wait_for("onslaught schedules", Duration::from_secs(2), |b| !b.mechanics.is_empty());

    // Stay connected for 4 s of the 8 s cooldown before disconnecting — the
    // true remainder to persist is ~4 s.
    settle(&mut alice, Duration::from_secs(4));
    drop(alice);
    // Give the server a moment to process the disconnect and save.
    std::thread::sleep(Duration::from_millis(500));

    let mut alice2 = Bot::connect_as(addr, "alice");
    alice2.wait_for("alice re-welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    alice2.wait_for("alice re-clock-sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    alice2.wait_for("alice re-snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());
    let pos2 = alice2.own_pos().unwrap();
    alice2.send_cast("onslaught", glam::Vec2::new(pos2.x, pos2.z));

    // (a) An immediate recast must be rejected: the persisted remainder is
    // still in effect — a relog must never reset the cooldown to zero.
    let settle_until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < settle_until {
        alice2.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        alice2.mechanics.is_empty(),
        "relog must not reset the cooldown: 'onslaught' was accepted immediately after relog"
    );

    // (b) The TRUE remaining cooldown is ~3-4 s (8 s minus the ~4 s already
    // elapsed before disconnect) — a pessimistic full-cooldown reset would
    // need the full 8 s from re-Welcome, so succeeding within 6 s proves the
    // exact remainder was restored, not a fresh full cooldown.
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        alice2.send_cast("onslaught", glam::Vec2::new(pos2.x, pos2.z));
        alice2.pump();
        if !alice2.mechanics.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "'onslaught' never became castable within 6 s of relog — the persisted remainder was not restored"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
