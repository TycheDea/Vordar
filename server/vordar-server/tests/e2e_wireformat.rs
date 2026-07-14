// Wire format and protocol tests: replication IDs, hp encoding, prefab table,
// datagram budgets, intent redundancy. Isolated from connectivity, combat, and
// persistence concerns.

use test_support::{settle, workspace_root, Bot, PopulateSystem};
use engine_app::scheduler::{Phase, SystemOrder};
use std::net::SocketAddr;
use std::time::Duration;
use engine_net::Impairment;

// Finding 1 of docs/reviews/networking/plan-networking-rework-5-2026-07-13.md: every
// wire entity id used to be raw hecs `Entity` bits (`entity.to_bits().get()`
// at the server net module's Welcome/HitResult/EntityDied/snapshot-gather sites) —
// always >= 2^32 because of the generation bits packed into the upper half,
// hence a 5+ byte postcard varint on every single reference. A zone-local
// `ReplIds` allocator now hands out small, monotonic `u32` ids instead. This
// documents the compactness contract directly: against the pre-fix wire
// format every one of these ids is in the billions and this test fails;
// after the fix every id is small by construction.
#[test]
fn replication_ids_are_compact() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25179".parse().unwrap();

    let positions: Vec<glam::Vec3> = (0..10).map(|i| glam::Vec3::new(i as f32 * 2.0, 0.0, 0.0)).collect();
    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        app.add_system(PopulateSystem { done: false, positions, prefab: "player".into() }, Phase::PreUpdate, SystemOrder::First);
        app.run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("the 10 NPCs replicate", Duration::from_secs(5), |b| b.prefabs.len() >= 10);

    let player_id = bot.player_id.unwrap();
    assert!((player_id as u64) < 100_000, "Welcome's player_id {player_id} is not a compact zone-local id");
    for &id in bot.prefabs.keys() {
        assert!((id as u64) < 100_000, "an AOI-enter id ({id}) is not a compact zone-local id");
    }
    for &id in bot.last_snapshot.keys() {
        assert!((id as u64) < 100_000, "an enters/states id ({id}) is not a compact zone-local id");
    }
}

// Finding 3 of docs/reviews/networking/plan-networking-rework-5-2026-07-13.md: hp used
// to flatten to a plain `i32` with 0 doing double duty for "no Health
// component" and "dead at 0 HP" (the server's old
// `hp.map(|h| h.current).unwrap_or(0)`). A Health-less replicated entity (the
// "bolt" prefab: Transform+Hitbox+PrefabId, no Health) must be indistinguishable
// on the wire from *absent hp*, not from "hp is 0" — which the old i32 format
// could not express. Against the pre-fix format this test fails: the bolt's
// flattened hp of 0 lands in `last_hp` just like any other reading, so it is
// NOT absent. After the fix, hp rides as `Option<i32>` and only `Some` readings
// reach `last_hp`.
#[test]
fn hp_none_distinguishes_health_less_entities() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25190".parse().unwrap();

    let positions = vec![glam::Vec3::new(5.0, 0.0, 0.0)];
    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        app.add_system(
            PopulateSystem { done: false, positions, prefab: "bolt".into() },
            Phase::PreUpdate,
            SystemOrder::First,
        );
        app.run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    bot.wait_for("the bolt replicates", Duration::from_secs(5), |b| {
        b.prefabs.values().any(|p| p == "bolt")
    });
    // Let a few more snapshots land so the bolt shows up in `states` too, not
    // only in its AOI-enter.
    settle(&mut bot, Duration::from_millis(500));

    let player_id = bot.player_id.unwrap();
    let bolt_id = *bot.prefabs.iter().find(|(_, p)| *p == "bolt").unwrap().0;

    assert!(bot.last_snapshot.contains_key(&bolt_id), "the bolt must still replicate a position");
    assert!(
        !bot.last_hp.contains_key(&bolt_id),
        "a Health-less entity must be ABSENT from last_hp (wire None), not present at 0"
    );
    assert_eq!(
        bot.last_hp.get(&player_id),
        Some(&100),
        "the player has a real Health component, so its hp rides as Some(100)"
    );
}

// Finding 4 of docs/reviews/networking/plan-networking-rework-5-2026-07-13.md:
// `EntityState.prefab` used to repeat the full prefab name string on every
// single AOI enter. A per-zone `ServerMsg::PrefabTable` is now sent once per
// connection immediately after `Welcome`, and `EntityState.prefab` rides as a
// `u16` index into it. This test references `Bot::prefab_names` directly — a
// field that does not exist before this finding lands — so it fails to
// compile against the pre-fix code rather than failing at runtime; once it
// compiles, `Bot::pump`'s index resolution (which panics on an unresolvable
// index) is the thing that would actually catch a broken wire-ordering
// regression.
#[test]
fn prefab_table_binds_u16_refs() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25191".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    // The bot's own AOI-enter can only have resolved to a name (below) if
    // `pump` already had a non-empty prefab table when it processed the
    // enter — Bot::pump panics on an unresolvable index instead of silently
    // dropping it, so reaching this wait_for's exit already proves the table
    // arrived before the first snapshot's enters (stream ordering).
    bot.wait_for("own enter resolves through the prefab table", Duration::from_secs(5), |b| {
        b.player_id.is_some_and(|id| b.prefabs.contains_key(&id))
    });

    assert!(!bot.prefab_names.is_empty(), "prefab table never arrived");
    assert!(
        bot.prefab_names.len() <= u16::MAX as usize + 1,
        "prefab table exceeds the u16 wire index space"
    );

    let player_id = bot.player_id.unwrap();
    assert_eq!(
        bot.prefabs.get(&player_id).map(String::as_str),
        Some("ravager"),
        "own enter must resolve to the PLAYER_PREFAB via the u16 table index"
    );
}

// Finding 5 of docs/reviews/networking/plan-networking-rework-5-2026-07-13.md: a
// permanent size gate on steady-state snapshot frames. Findings 1-4 compacted
// wire entity ids (u32, not raw hecs bits), quantized positions (WirePos),
// made hp an explicit Option (no more 0-as-"no Health"), and replaced a
// repeated prefab name string with a u16 table index — together meant to
// bring a full 64-entry `states` frame (the server's MAX_SNAPSHOT_STATES)
// comfortably under the ~1.2 KB QUIC datagram budget that rework 3
// (snapshots on datagrams) is physically blocked on today. Against the
// pre-rework wire format this exact scenario (a 100-entity crowd, steady
// state, the full 64-entry states budget) measures ~1.25 KB+ per frame and
// this test fails; it passes only because findings 1-4 landed first.
#[test]
fn crowd_snapshot_fits_datagram_budget() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25192".parse().unwrap();

    // 100 "player" NPCs on rings of radius 5-25 around the origin — all
    // comfortably inside the bot's AOI_RADIUS (40, the server's AOI_RADIUS).
    let mut positions: Vec<glam::Vec3> = Vec::new();
    for ring in 0..10 {
        let radius = 5.0 + ring as f32 * (20.0 / 9.0); // 10 rings, 5.0..=25.0
        for spot in 0..10 {
            let angle = (spot as f32 / 10.0) * std::f32::consts::TAU;
            positions.push(glam::Vec3::new(radius * angle.cos(), 0.0, radius * angle.sin()));
        }
    }

    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        app.add_system(PopulateSystem { done: false, positions, prefab: "player".into() }, Phase::PreUpdate, SystemOrder::First);
        app.run_headless(60.0, Some(2500));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut bot = Bot::connect(addr);
    bot.wait_for("welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    // MAX_SNAPSHOT_STATES = 64 (the server): with 101 entities in the AOI
    // (100 NPCs + the bot's own player), the crowd-throttle round-robin caps
    // `states` at the full budget — the worst case this gate must measure.
    bot.wait_for("crowd throttle budget reached", Duration::from_secs(10), |b| {
        b.last_states.len() == 64
    });

    // Let the initial `enters` wave (all 101 identities) finish landing
    // before measuring — a first-join identity wave rides `AoiDelta` on the
    // reliable stream (protocol v14, networking rework 3 finding 4), entirely
    // separate from the `Snapshot` datagram this gate measures, but settling
    // first still avoids racing the AOI-entry seeding against steady state.
    settle(&mut bot, Duration::from_secs(1));
    bot.snapshot_bytes.clear();
    settle(&mut bot, Duration::from_secs(1)); // ~10 snapshots at SNAPSHOT_HZ

    assert!(!bot.snapshot_bytes.is_empty(), "no steady-state snapshots were measured");
    assert_eq!(
        bot.last_states.len(),
        64,
        "the worst case (full states budget) must still be in effect when measured"
    );
    // Since rework 3 finding 4, `snapshot_bytes` measures only the datagram
    // `Snapshot { tick, last_processed_seq, states }` payload — identity
    // (enters/leaves) moved to the separate stream-only `AoiDelta` message, so
    // this 1100-byte gate is now literally the datagram-budget measurement
    // rework 3 needed, not an approximation of it.
    let max = *bot.snapshot_bytes.iter().max().unwrap();
    assert!(
        max <= 1100,
        "steady-state snapshot frame is {max} bytes, over the 1100-byte datagram-budget gate"
    );
}

// Finding 5 of docs/reviews/networking/plan-networking-rework-3-2026-07-13.md:
// `ClientMsg::MoveIntent` was replaced by `ClientMsg::MoveIntents`, a batch
// of up to 3 entries (this tick's plus the two previous) sent via datagram
// every Input tick — a lost datagram is fully recovered by the next tick's
// overlapping batch. This proves the recovery end-to-end under upstream
// loss: a bot walking +X at realistic WAN RTT (40 ms) with heavy upstream
// loss (30%) must still replicate most of the displacement an unimpaired
// control bot covers over the identical send cadence and window. Without
// redundancy only ~70% of intents would apply at this loss rate (each
// dropped datagram is a fully lost tick); with last-3 redundancy an intent
// is lost only if 3 consecutive datagrams all drop (0.3³ ≈ 2.7%), so ~97%+
// apply — comfortably above the 85% gate this test asserts.
#[test]
fn move_intents_redundancy_survives_upstream_loss() {
    workspace_root();
    let addr: SocketAddr = "127.0.0.1:25193".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(1200));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut control = Bot::connect_as(addr, "redundancy-control");
    control.wait_for("control welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    control.wait_for("control clock sync", Duration::from_secs(5), |b| b.client.server_offset_micros().is_some());
    control.wait_for("control first snapshot", Duration::from_secs(5), |b| b.own_pos().is_some());

    let mut impaired = Bot::connect_full_as(
        addr,
        "redundancy-impaired",
        Impairment { rtt: Duration::from_millis(40), upstream_loss: 0.3, ..Default::default() },
    );
    impaired.wait_for("impaired welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    impaired.wait_for("impaired clock sync", Duration::from_secs(10), |b| b.client.server_offset_micros().is_some());
    impaired.wait_for("impaired first snapshot", Duration::from_secs(10), |b| b.own_pos().is_some());

    let control_start = control.own_pos().expect("control bot has an initial position");
    let impaired_start = impaired.own_pos().expect("impaired bot has an initial position");

    // Both bots walk +X, sending one MoveIntents batch per 16 ms tick, for
    // the same ~3 s window.
    let dir = glam::Vec2::X;
    let end = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < end {
        control.send_move(dir);
        control.pump();
        impaired.send_move(dir);
        impaired.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    control.send_move(glam::Vec2::ZERO);
    impaired.send_move(glam::Vec2::ZERO);
    settle(&mut control, Duration::from_millis(500));
    settle(&mut impaired, Duration::from_millis(500));

    let control_disp = control.own_pos().unwrap().x - control_start.x;
    let impaired_disp = impaired.own_pos().unwrap().x - impaired_start.x;

    assert!(control_disp > 1.0, "control bot barely moved ({control_disp:.2}) — test setup is broken");
    assert!(
        impaired_disp >= 0.85 * control_disp,
        "impaired bot displaced {impaired_disp:.2} vs control's {control_disp:.2} \
         ({:.0}%) — last-3 redundancy should recover ~97%+ of intents at 30% upstream \
         loss, not fall below the 85% gate",
        100.0 * impaired_disp / control_disp
    );
}
