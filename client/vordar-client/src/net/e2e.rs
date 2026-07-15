use crate::net::{
    interpolate::NetInterpolateSystem,
    lifecycle::NetReceiveSystem,
    prediction::{start_predicted_leap, NetCorrectionSystem, NetSendInputSystem, SNAP_DISTANCE},
    own_entity, reconnect_attempt, NetClientState,
};
use crate::world_time::WorldTime;
use engine_app::events::EventBus;
use engine_app::scheduler::System;
use engine_app::time::Time;
use engine_core::components::Transform;
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use engine_net::NetClient;
use glam::{Vec2, Vec3};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use test_support::{name_token, percentile, workspace_root, Bot};
use vordar_game::motion::MovementSystem;
use vordar_game::player::PlayerMovementSystem;
use vordar_game::Player;
use vordar_protocol::{PROTOCOL_VERSION, TICK_HZ};

const DT: f32 = 1.0 / 60.0;

/// Registers the game's core components and content prefabs into
/// `resources` — the identical registration list both the onslaught replay
/// test and the smoothness probe need to spawn real content prefabs
/// (`content/prefabs`). Shared here instead of duplicated in each test.
fn insert_game_prefabs(resources: &mut Resources) {
    let mut registry = engine_core::prefab::ComponentRegistry::new();
    engine_core::prefab::register_core_components(&mut registry);
    registry.register::<Player>("Player");
    registry.register::<vordar_game::enemies::Enemy>("Enemy");
    registry.register::<vordar_game::ContactDamage>("ContactDamage");
    registry.register::<vordar_game::CombatStats>("CombatStats");
    registry.register::<vordar_game::class::ClassId>("Class");
    registry.register::<vordar_game::class::RaceId>("Race");
    registry.register::<vordar_game::vfx::VfxTrail>("VfxTrail");
    let mut prefabs = engine_core::prefab::PrefabLibrary::new();
    prefabs.load_dir("content/prefabs");
    resources.insert(registry);
    resources.insert(prefabs);
}

/// Networking audit 2026-07-11, finding 7: "disconnect is a log line, no
/// reconnect, no teardown" — this drives a real server, a real
/// `NetClient` connection, and the real `NetReceiveSystem` (no
/// reimplemented logic). A second login under the same character name
/// makes the server's existing session-takeover kick the first
/// connection (mirrors `phase6_login_takeover` in vordar-server's e2e
/// suite) — a genuine, unannounced disconnect. engine-net's listening
/// socket is never released once bound (findings 13/14/18 — no shutdown
/// story), so an in-process bind/drop/rebind on the same port isn't
/// possible; kicking the connection is the closest headless equivalent
/// of "the server process restarted" that exercises the same
/// Disconnected → teardown → backoff-redial → relogin path. The victim
/// must notice, tear down, and relogin entirely on its own.
#[test]
fn kicked_connection_reconnects_and_relogs_in() {
    workspace_root();

    let addr: SocketAddr = "127.0.0.1:25400".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(1800));
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut world = World::new();
    let mut resources = Resources::new();
    resources.insert(DespawnQueue::new());
    resources.insert(Time::new());
    resources.insert(WorldTime { offset_micros: 0, synced: false });
    resources.insert(NetClientState::new(
        Some(NetClient::connect(addr, PROTOCOL_VERSION).expect("victim connect")),
        addr,
        "reconnect-victim".into(),
        name_token("reconnect-victim"),
        false,
        Duration::ZERO,
    ));

    let mut recv = NetReceiveSystem;
    let deadline = Instant::now() + Duration::from_secs(5);
    while resources.get::<NetClientState>().unwrap().own_id.is_none() {
        assert!(Instant::now() < deadline, "victim never received its first Welcome");
        recv.run(&mut world, &mut resources, DT);
        std::thread::sleep(Duration::from_millis(16));
    }
    let first_id = resources.get::<NetClientState>().unwrap().own_id.unwrap();

    // Kick it: a second login under the same name takes over the
    // session server-side, closing the victim's connection out from
    // under it. Wait for the kicker's own Welcome before dropping it —
    // `Connection::close` tears the connection down immediately,
    // without flushing pending stream writes, so dropping right after
    // `send` can race the Login frame right off the wire. `Bot` derives
    // the same token from the name, so this takeover satisfies the
    // token-match requirement (networking rework 1, finding 3)
    // automatically.
    let mut kicker = Bot::connect_as(addr, "reconnect-victim");
    kicker.wait_for("kicker Welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    drop(kicker);

    // The victim must notice on its own — no test code touches its
    // NetClientState between here and full reconnection.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        recv.run(&mut world, &mut resources, DT);
        if reconnect_attempt(&resources).is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "the kick was never detected as a disconnect");
        std::thread::sleep(Duration::from_millis(16));
    }
    // Torn down immediately, not left dangling — the frozen-world half
    // of this finding's gap.
    assert!(
        resources.get::<NetClientState>().unwrap().own_id.is_none(),
        "an unexpected disconnect must clear own_id right away"
    );

    // Entirely automatic from here: backoff, redial, Connected, Login, Welcome.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        recv.run(&mut world, &mut resources, DT);
        let state = resources.get::<NetClientState>().unwrap();
        if state.own_id.is_some() && state.reconnect.is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "the client never reconnected on its own");
        std::thread::sleep(Duration::from_millis(16));
    }
    let second_id = resources.get::<NetClientState>().unwrap().own_id.unwrap();
    assert_ne!(first_id, second_id, "reconnect must relogin into a fresh body");
}

/// Networking audit 2026-07-11, finding 11: "Prediction replay models
/// plain movement only — leaps ... produce snaps at real latency." A real
/// headless server plus a real predicting `NetClient` (150 ms simulated
/// RTT, the finding's own number) cast a real Onslaught (the Ravager's
/// gap-closer, `content/classes/ravager.ron`: 0.4 s dash, 8 s cooldown,
/// 12-unit range) and drive exactly the systems `NetClientPlugin`
/// registers for a predicting player (`NetReceiveSystem`,
/// `NetSendInputSystem`, `PlayerMovementSystem`, `LeapSystem`,
/// `MovementSystem`, `NetCorrectionSystem`) — no reimplemented logic.
///
/// `reconcile_own` (invoked from inside `NetReceiveSystem::run` via
/// `apply_snapshot`) snaps `transform.position` straight to `replayed`
/// whenever the reconciliation error exceeds SNAP_DISTANCE; every other
/// path (Trust, Smooth) leaves `transform.position` untouched. So a snap
/// shows up as a position jump bigger than SNAP_DISTANCE measured across a
/// single `NetReceiveSystem::run` call — that must never happen, during
/// the dash or right after it.
#[test]
fn onslaught_dash_replay_never_snaps_at_150ms_rtt() {
    workspace_root();

    let addr: SocketAddr = "127.0.0.1:25402".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(2400));
    });
    std::thread::sleep(Duration::from_millis(300));

    // Real ability data (cast time, range) instead of hardcoded numbers —
    // the same content the server and a real client both load.
    let mut classes = vordar_game::class::ClassLibrary::new();
    classes.load_dir("content/classes");
    let onslaught = classes.get("ravager", "onslaught").expect("ravager has onslaught");
    let (cast_secs, max_range) = match &onslaught.effect {
        vordar_game::skills::AbilityEffect::Leap { cast_micros, max_range, .. } => {
            (*cast_micros as f32 / 1e6, *max_range)
        }
        _ => panic!("onslaught must be a Leap effect"),
    };

    // Component/prefab setup mirrors engine_app::prefab_plugin::PrefabPlugin
    // + vordar_game::plugin::GameComponentsPlugin without a full App — this
    // test drives systems directly, same as `kicked_connection_reconnects_and_relogs_in` above.
    let mut world = World::new();
    let mut resources = Resources::new();
    insert_game_prefabs(&mut resources);
    resources.insert(DespawnQueue::new());
    resources.insert(Time::new());
    resources.insert(WorldTime { offset_micros: 0, synced: false });
    resources.insert(EventBus::new());
    resources.insert(engine_app::input::KeyboardState::new());
    resources.insert(NetClientState::new(
        Some(
            NetClient::connect_with_latency(addr, PROTOCOL_VERSION, Duration::from_millis(150))
                .expect("dasher connect"),
        ),
        addr,
        "onslaught-dasher".into(),
        name_token("onslaught-dasher"),
        true,
        Duration::from_millis(150),
    ));

    let mut recv = NetReceiveSystem;
    let mut send = NetSendInputSystem;
    let mut player_move = PlayerMovementSystem;
    let mut leap_sys = vordar_game::combat::leap::LeapSystem;
    let mut move_sys = MovementSystem;
    let mut correction_sys = NetCorrectionSystem;
    let mut max_recv_jump = 0.0f32;

    // Input-phase half of one tick (NetReceiveSystem, NetSendInputSystem)
    // — watches every NetReceiveSystem call for a snap-sized position jump.
    let mut run_input = |world: &mut World, resources: &mut Resources| {
        resources.get_mut::<EventBus>().unwrap().clear();
        let before = own_entity(resources).and_then(|e| world.get::<&Transform>(e).ok().map(|t| t.position));
        recv.run(world, resources, DT);
        if let Some(before) = before {
            if let Some(after) =
                own_entity(resources).and_then(|e| world.get::<&Transform>(e).ok().map(|t| t.position))
            {
                max_recv_jump = max_recv_jump.max((after - before).length());
            }
        }
        send.run(world, resources, DT);
    };
    // Update-phase half (PlayerMovementSystem, LeapSystem, MovementSystem,
    // NetCorrectionSystem) — same order NetClientPlugin registers them in.
    let mut run_update = |world: &mut World, resources: &mut Resources| {
        player_move.run(world, resources, DT);
        leap_sys.run(world, resources, DT);
        move_sys.run(world, resources, DT);
        correction_sys.run(world, resources, DT);
    };

    // Welcome + clock sync.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        run_input(&mut world, &mut resources);
        run_update(&mut world, &mut resources);
        let ready = {
            let state = resources.get::<NetClientState>().unwrap();
            state.own_id.is_some() && state.client.as_ref().unwrap().server_now_micros().is_some()
        };
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "never got Welcome + clock sync");
        std::thread::sleep(Duration::from_millis(16));
    }

    // Finding 1 of docs/reviews/networking/plan-networking-rework-1-2026-07-13.md:
    // cooldowns now persist as remainders instead of pessimistically
    // seeding full cooldown at spawn, so a fresh character's "onslaught"
    // is castable immediately — no cooldown-clearing wait needed. The
    // predicted entity itself is still created only once the first
    // Snapshot's `enters` list reaches this client (Welcome alone
    // doesn't spawn it), so pump until it exists.
    let entity_deadline = Instant::now() + Duration::from_secs(2);
    while own_entity(&resources).is_none() {
        run_input(&mut world, &mut resources);
        run_update(&mut world, &mut resources);
        assert!(Instant::now() < entity_deadline, "predicted entity never appeared after Welcome");
        std::thread::sleep(Duration::from_millis(16));
    }
    let entity = own_entity(&resources).expect("predicted entity must exist by now");
    let origin = world.get::<&Transform>(entity).unwrap().position;
    let target = origin + Vec3::new(max_range * 0.5, 0.0, 0.0);
    let cast_target = Vec2::new(target.x, target.z);
    let velocity = vordar_game::combat::leap::leap_velocity(origin, target, cast_secs);

    // The cast tick: Input phase first (the real wire CastIntent, so the
    // server mirrors the identical dash, same as NetSendInputSystem's own
    // MoveIntent send moments earlier), then the client-predicted
    // insertion `start_predicted_leap` — exactly what AbilityCastSystem
    // calls for a Leap ability, invoked directly here because
    // AbilityCastSystem itself needs a renderer (mouse-cursor ground
    // projection) this headless test has none of — then Update phase, so
    // the dash begins immediately this same tick like the real system.
    run_input(&mut world, &mut resources);
    assert!(resources.get_mut::<NetClientState>().unwrap().send_cast_intent("onslaught".into(), cast_target));
    start_predicted_leap(&mut world, &mut resources, entity, velocity, cast_secs);
    run_update(&mut world, &mut resources);

    // Run through the dash and settle afterward, watching every tick's
    // NetReceiveSystem call for a snap.
    let dash_deadline = Instant::now() + Duration::from_secs(4);
    let mut elapsed = 0.0f32;
    while elapsed < cast_secs + 1.0 {
        assert!(Instant::now() < dash_deadline, "test loop stalled mid-dash");
        std::thread::sleep(Duration::from_millis(16));
        run_input(&mut world, &mut resources);
        run_update(&mut world, &mut resources);
        elapsed += DT;
    }

    assert!(
        max_recv_jump < SNAP_DISTANCE,
        "reconciliation snapped {max_recv_jump:.2} units mid-dash — leap-aware replay must keep \
         corrections under SNAP_DISTANCE ({SNAP_DISTANCE})"
    );
}

/// Sleeps until the next precise 60 Hz tick boundary, then advances
/// `next` by exactly one tick. The connection-wait loops elsewhere in
/// this file pace themselves with a flat 16 ms sleep, which drifts ~4 %
/// fast against the true 16.667 ms tick — harmless for a coarse "did the
/// Welcome arrive yet" wait, but enough drift for an extra render call to
/// occasionally land inside the smoothness probe's degenerate
/// reversal-cancellation window (see `drive_mover`'s doc comment),
/// spuriously inflating its measured zero-motion run by one tick.
fn pace_tick(next: &mut Instant) {
    *next += Duration::from_secs_f64(1.0 / TICK_HZ as f64);
    let now = Instant::now();
    if *next > now {
        std::thread::sleep(*next - now);
    } else {
        *next = now; // fell behind — resync instead of trying to catch up
    }
}

/// Sends the mover's next `ClientMsg::MoveIntents` datagram — the same
/// last-3 redundancy ring `NetSendInputSystem` keeps — and reverses
/// `dir`'s X sign every ~2 s so the mover walks back and forth instead of
/// leaving the observer's AOI (networking rework 4, finding 3's
/// smoothness probe). 2170 ms, not a whole multiple of the 100 ms
/// `SNAPSHOT_HZ` cadence, so a reversal's phase against the sample
/// boundaries drifts instead of relocking to the same offset every cycle.
/// `dir`'s Z component is a small constant (never reversed): a pure ±X
/// flip can make one buffered sample nearly *identical* to its neighbor
/// whenever the flip lands near a sample's midpoint (out, then most of
/// the way back, within one 100 ms interval) — a real linear-
/// interpolation artifact, not a freeze regression, but one that (when it
/// coincides with a lost/late next sample) can extrapolate from that
/// near-zero velocity for longer than this probe's zero-motion gate
/// allows. The steady Z drift guarantees no two samples are ever exactly
/// equal, so the interpolated segment is always genuinely moving.
fn drive_mover(mover: &mut Bot, dir: &mut Vec2, last_reverse: &mut Instant) {
    const REVERSE_INTERVAL: Duration = Duration::from_millis(2170);
    if last_reverse.elapsed() >= REVERSE_INTERVAL {
        dir.x = -dir.x;
        *last_reverse = Instant::now();
    }
    mover.send_move(*dir);
    mover.pump();
}

/// Networking rework 4, finding 3
/// (`docs/reviews/networking/plan-networking-rework-4-2026-07-14.md`): the loss
/// probes (`server/vordar-server/tests/loss.rs`) measure arrival gaps and
/// intent-ack lag only — nothing measures what a player actually SEES: the
/// per-tick rendered motion of a remote entity under loss and jitter. A
/// real headless server, a real "mover" (a second `Bot`, the kicker
/// pattern above) streaming `MoveIntents` datagrams ±X at 6 u/s,
/// and a real WAN-impaired "observer" running the actual client systems
/// (`NetReceiveSystem` + `NetInterpolateSystem`, `predict: false` — a
/// non-predicting own player is buffered like any remote) prove the
/// fixed-delay playback buffer (findings 1-2) keeps the rendered path
/// continuous instead of freezing/warbling at every late or lost
/// snapshot. Permanent regression gates, run like the loss probes:
/// `cargo test -p vordar-client --release -- --ignored --nocapture`.
#[test]
#[ignore = "loss probe — run with --release --ignored --nocapture"]
fn remote_render_smoothness_under_loss_probe() {
    workspace_root();
    if cfg!(debug_assertions) {
        eprintln!("WARNING: loss probe running in debug — results will not be representative");
    }

    const SPEED: f32 = 6.0;
    const WINDOW: Duration = Duration::from_secs(20);
    const SETTLE: Duration = Duration::from_secs(2);

    let addr: SocketAddr = "127.0.0.1:25404".parse().unwrap();
    std::thread::spawn(move || {
        vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(60 * 60));
    });
    std::thread::sleep(Duration::from_millis(300));

    // The mover: a second Bot (the kicker pattern above) that logs in and
    // streams MoveIntents — unimpaired; only the observer's connection
    // below carries the WAN impairment.
    let mut mover = Bot::connect_as(addr, "smoothness-mover");
    mover.wait_for("mover Welcome", Duration::from_secs(5), |b| b.player_id.is_some());
    let mover_id = mover.player_id.unwrap();
    // Small constant Z drift alongside the ±X reversal — see
    // `drive_mover`'s doc comment for why this is needed.
    let mut mover_dir = Vec2::new(1.0, 0.1).normalize();
    let mut last_reverse = Instant::now();

    // The observer: the onslaught test's world verbatim (prefab registry
    // + real NetReceiveSystem/NetInterpolateSystem), connected WAN-
    // impaired (100 ms RTT, 30 ms jitter, 3 % downstream loss).
    let mut world = World::new();
    let mut resources = Resources::new();
    insert_game_prefabs(&mut resources);
    resources.insert(DespawnQueue::new());
    resources.insert(Time::new());
    resources.insert(WorldTime { offset_micros: 0, synced: false });
    resources.insert(NetClientState::new(
        Some(
            NetClient::connect_impaired(addr, PROTOCOL_VERSION, engine_net::Impairment {
                rtt: Duration::from_millis(100),
                jitter: Duration::from_millis(30),
                downstream_loss: 0.03,
                ..Default::default()
            })
            .expect("observer connect"),
        ),
        addr,
        "smoothness-observer".into(),
        name_token("smoothness-observer"),
        false,
        Duration::from_millis(100),
    ));

    let mut recv = NetReceiveSystem;
    let mut render_sys = NetInterpolateSystem;
    // Precise 60 Hz pacing (see `pace_tick`) — shared across every loop
    // below so the whole run stays on one continuous tick boundary
    // instead of drifting at each stage transition.
    let mut next_tick = Instant::now();

    // Welcome + clock sync, same wait the onslaught test uses.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        recv.run(&mut world, &mut resources, DT);
        let ready = {
            let state = resources.get::<NetClientState>().unwrap();
            state.own_id.is_some() && state.client.as_ref().unwrap().server_now_micros().is_some()
        };
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "observer never got Welcome + clock sync");
        pace_tick(&mut next_tick);
    }

    // Wait for the mover's own entity to enter the observer's AOI.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mover_entity = loop {
        drive_mover(&mut mover, &mut mover_dir, &mut last_reverse);
        recv.run(&mut world, &mut resources, DT);
        render_sys.run(&mut world, &mut resources, DT);
        if let Some(&e) = resources.get::<NetClientState>().unwrap().entities.get(&mover_id) {
            break e;
        }
        assert!(Instant::now() < deadline, "observer never saw the mover's entity (id {mover_id}) in its AOI");
        pace_tick(&mut next_tick);
    };

    // Let the buffer/playback cursor lock onto its steady-state delay
    // before recording (mirrors loss.rs's `common::settle`).
    let settle_deadline = Instant::now() + SETTLE;
    while Instant::now() < settle_deadline {
        drive_mover(&mut mover, &mut mover_dir, &mut last_reverse);
        recv.run(&mut world, &mut resources, DT);
        render_sys.run(&mut world, &mut resources, DT);
        pace_tick(&mut next_tick);
    }

    // The measurement window: record the mover entity's rendered
    // Transform.position after every Update tick.
    let mut prev_pos = world.get::<&Transform>(mover_entity).unwrap().position;
    let mut steps: Vec<f32> = Vec::new();
    let mut zero_run = 0u32;
    let mut max_zero_run = 0u32;
    let window_deadline = Instant::now() + WINDOW;
    while Instant::now() < window_deadline {
        drive_mover(&mut mover, &mut mover_dir, &mut last_reverse);
        recv.run(&mut world, &mut resources, DT);
        render_sys.run(&mut world, &mut resources, DT);

        let pos = world.get::<&Transform>(mover_entity).unwrap().position;
        let step = (pos - prev_pos).length();
        steps.push(step);
        if step < 1e-4 {
            zero_run += 1;
            max_zero_run = max_zero_run.max(zero_run);
        } else {
            zero_run = 0;
        }
        prev_pos = pos;

        pace_tick(&mut next_tick);
    }

    assert!(steps.len() > 500, "smoothness probe only recorded {} ticks — window too short", steps.len());
    let nominal = SPEED / 60.0;
    let mut steps64: Vec<f64> = steps.iter().map(|&s| s as f64).collect();
    let p50 = percentile(&mut steps64, 0.50);
    let p99 = percentile(&mut steps64, 0.99);
    let max = *steps64.last().unwrap();
    println!(
        "remote render smoothness: ticks={} step_u p50={:.4} p99={:.4} max={:.4} longest_zero_run={}",
        steps.len(),
        p50,
        p99,
        max,
        max_zero_run
    );
    // Permanent regression gates (networking rework 4, finding 3): the
    // pre-rework-4 client froze 10-18 ticks at every late/lost snapshot
    // and then caught up at ~2x steps — both margins are >=2x here.
    assert!(
        max_zero_run <= 5,
        "longest zero-motion run {max_zero_run} ticks exceeds the 5-tick (~83 ms) freeze gate"
    );
    assert!(
        p99 <= 1.5 * nominal as f64,
        "p99 per-tick step {p99:.4} exceeds 1.5x nominal ({:.4}) — catch-up warble regression",
        1.5 * nominal as f64
    );
}
