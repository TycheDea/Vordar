// Phase 7 soak: 200 bot clients in one zone must not break the tick budget —
// movement holds 60 Hz, snapshots hold 10 Hz, throttled bandwidth stays
// bounded, and a walking bot still covers the distance its intents demand.
//
// Ignored by default (heavy, wall-clock ~50 s). Run in RELEASE:
//
//   cargo test -p vordar-server --release --test soak -- --ignored --nocapture

mod common;

use common::Bot;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::traits::Resources;
use engine_core::World;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Bot count — override with VORDAR_SOAK_BOTS (default 200) for scaling runs.
fn soak_bots() -> usize {
    std::env::var("VORDAR_SOAK_BOTS").ok().and_then(|s| s.parse().ok()).unwrap_or(200)
}

const SAMPLED: usize = 5;
const WINDOW: Duration = Duration::from_secs(30);
const PLAYER_SPEED: f32 = 6.0; // content/prefabs/player.ron

/// Test-local instrumentation (the determinism discipline binds game systems,
/// not meters): records the interval between consecutive runs of its phase
/// while `recording` is set.
struct PhaseMeter {
    last: Option<Instant>,
    recording: Arc<AtomicBool>,
    intervals: Arc<Mutex<Vec<f64>>>,
}

impl System for PhaseMeter {
    fn run(&mut self, _world: &mut World, _resources: &mut Resources, _delta: f32) {
        if !self.recording.load(Ordering::Relaxed) {
            self.last = None;
            return;
        }
        let now = Instant::now();
        if let Some(prev) = self.last {
            self.intervals.lock().unwrap().push((now - prev).as_secs_f64());
        }
        self.last = Some(now);
    }
}

fn p99(intervals: &mut [f64]) -> f64 {
    intervals.sort_by(|a, b| a.total_cmp(b));
    intervals[(intervals.len() * 99) / 100 - 1]
}

/// Deterministic wander: direction from a per-bot LCG, re-rolled every 30
/// sends; beyond r=30 the bot is pulled back toward the origin so the crowd
/// stays mutually in-AOI (the worst case for snapshot fan-out).
struct Wander {
    rng: u64,
    dir: glam::Vec2,
    sends: u32,
}

impl Wander {
    fn new(seed: u64) -> Self {
        Self { rng: seed.wrapping_mul(2862933555777941757).wrapping_add(3037000493), dir: glam::Vec2::X, sends: 30 }
    }

    fn next_dir(&mut self, pos: Option<glam::Vec3>) -> glam::Vec2 {
        if let Some(p) = pos {
            if p.length() > 30.0 {
                return glam::Vec2::new(-p.x, -p.z).normalize();
            }
        }
        self.sends += 1;
        if self.sends >= 30 {
            self.sends = 0;
            self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let angle = (self.rng >> 33) as f32 / (u32::MAX >> 1) as f32 * std::f32::consts::TAU;
            self.dir = glam::Vec2::new(angle.cos(), angle.sin());
        }
        self.dir
    }
}

/// Drive a set of bots at ~60 Hz until `stop`.
fn drive(mut bots: Vec<(Bot, Wander)>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            for (bot, wander) in bots.iter_mut() {
                let dir = wander.next_dir(bot.own_pos());
                bot.send_move(dir);
                bot.pump();
            }
            std::thread::sleep(Duration::from_millis(16));
        }
    })
}

#[test]
#[ignore = "soak — run with --release --ignored"]
fn phase7_soak_200_bots_hold_tick_budget() {
    common::workspace_root();
    if cfg!(debug_assertions) {
        eprintln!("WARNING: soak running in debug — results will not be representative");
    }
    let total_bots = soak_bots();
    let addr: SocketAddr = "127.0.0.1:25180".parse().unwrap();

    let recording = Arc::new(AtomicBool::new(false));
    let input_intervals: Arc<Mutex<Vec<f64>>> = Arc::default();
    let post_intervals: Arc<Mutex<Vec<f64>>> = Arc::default();
    {
        let recording = recording.clone();
        let input_intervals = input_intervals.clone();
        let post_intervals = post_intervals.clone();
        std::thread::spawn(move || {
            let mut app = vordar_server::build_server_app(addr, ":memory:");
            app.add_system(
                PhaseMeter { last: None, recording: recording.clone(), intervals: input_intervals },
                Phase::Input,
                SystemOrder::First,
            );
            app.add_system(
                PhaseMeter { last: None, recording, intervals: post_intervals },
                Phase::PostUpdate,
                SystemOrder::Last,
            );
            // ≥90 s of sim at 60 Hz — covers ramp-up + window + walk + slack;
            // scaled with the bot count so bigger runs get a longer ramp.
            app.run_headless(60.0, Some(5400.max(total_bots as u64 * 27)));
        });
    }
    std::thread::sleep(Duration::from_millis(300));

    // ── Ramp up: bots in batches of 20 every 250 ms. ──
    let mut bots: Vec<Bot> = Vec::with_capacity(total_bots);
    while bots.len() < total_bots {
        for _ in 0..20.min(total_bots - bots.len()) {
            bots.push(Bot::connect(addr));
        }
        for bot in bots.iter_mut() {
            bot.pump();
        }
        std::thread::sleep(Duration::from_millis(250));
        eprintln!("connected {}", bots.len());
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        for bot in bots.iter_mut() {
            bot.pump();
        }
        let welcomed = bots.iter().filter(|b| b.player_id.is_some()).count();
        if welcomed == total_bots {
            break;
        }
        assert!(Instant::now() < deadline, "only {welcomed}/{total_bots} bots welcomed in 60 s");
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("all {total_bots} bots welcomed");

    // ── Split: 5 sampled bots stay on this thread (stats + the walker), the
    // rest spread over 4 driver threads as pure load. ──
    let mut sampled: Vec<Bot> = bots.drain(..SAMPLED).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let mut drivers = Vec::new();
    let chunk = bots.len().div_ceil(4);
    let mut seed = SAMPLED as u64;
    while !bots.is_empty() {
        let take = chunk.min(bots.len());
        let group: Vec<(Bot, Wander)> = bots
            .drain(..take)
            .map(|b| {
                seed += 1;
                (b, Wander::new(seed))
            })
            .collect();
        drivers.push(drive(group, stop.clone()));
    }

    // Let the crowd mix for a moment, then open the measurement window.
    std::thread::sleep(Duration::from_secs(3));
    for (i, bot) in sampled.iter_mut().enumerate() {
        bot.pump();
        bot.bytes = 0;
        bot.snapshot_ticks.clear();
        // Sampled bots wander too; seeds disjoint from the drivers'.
        let _ = i;
    }
    recording.store(true, Ordering::Relaxed);
    let mut wanders: Vec<Wander> = (0..SAMPLED).map(|i| Wander::new(1000 + i as u64)).collect();
    let window_end = Instant::now() + WINDOW;
    while Instant::now() < window_end {
        for (bot, wander) in sampled.iter_mut().zip(wanders.iter_mut()) {
            let dir = wander.next_dir(bot.own_pos());
            bot.send_move(dir);
            bot.pump();
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    recording.store(false, Ordering::Relaxed);

    // ── Stats first (a scaling probe past the budget must still report),
    // then the budget assertions. ──
    let mut input = input_intervals.lock().unwrap().clone();
    let input_hz = input.len() as f64 / WINDOW.as_secs_f64();
    eprintln!("input: {} runs ({input_hz:.1} Hz), p99 interval {:.1} ms", input.len(), p99(&mut input) * 1e3);

    let mut post = post_intervals.lock().unwrap().clone();
    let post_hz = post.len() as f64 / WINDOW.as_secs_f64();
    eprintln!("postupdate: {} runs ({post_hz:.1} Hz), p99 interval {:.1} ms", post.len(), p99(&mut post) * 1e3);

    // Machine-readable summary for docs/benchmarks/BASELINE.md.
    let avg_kb_s = sampled.iter().map(|b| b.bytes as f64).sum::<f64>()
        / sampled.len() as f64 / WINDOW.as_secs_f64() / 1024.0;
    println!(
        "soak: bots={total_bots} input_hz={input_hz:.1} input_p99_ms={:.2} post_hz={post_hz:.1} post_p99_ms={:.2} kb_s_per_client={avg_kb_s:.1}",
        p99(&mut input) * 1e3,
        p99(&mut post) * 1e3,
    );

    assert!(
        (58.0..=62.0).contains(&input_hz),
        "movement tick rate out of budget: {input_hz:.1} Hz"
    );
    assert!(p99(&mut input) < 0.025, "input p99 interval {:.1} ms ≥ 25 ms", p99(&mut input) * 1e3);
    assert!(post_hz >= 9.0, "snapshot phase rate out of budget: {post_hz:.1} Hz");

    for (i, bot) in sampled.iter().enumerate() {
        let snap_hz = bot.snapshot_ticks.len() as f64 / WINDOW.as_secs_f64();
        let kb_s = bot.bytes as f64 / WINDOW.as_secs_f64() / 1024.0;
        eprintln!("sampled bot {i}: {snap_hz:.1} snapshots/s, {kb_s:.1} KB/s");
        assert!(snap_hz >= 9.0, "bot {i} saw only {snap_hz:.1} snapshots/s");
        assert!(kb_s < 25.0, "bot {i} bandwidth {kb_s:.1} KB/s over the 25 KB/s budget");
    }

    // ── Movement integrity: a bot walks straight east for 5 s while the
    // crowd churns. "No intents dropped under load" is pinned by the ack
    // stream catching up to the last sent seq (the Phase 2 property, now at
    // 200× the load); displacement only sanity-checks that movement happens
    // at all — SeparationSystem drag through a 200-bot crowd legitimately
    // costs about half the free-path distance. ──
    let walker = &mut sampled[0];
    walker.pump();
    let start = walker.own_pos().expect("walker has a position");
    let walk_end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < walk_end {
        walker.send_move(glam::Vec2::X);
        walker.pump();
        std::thread::sleep(Duration::from_millis(16));
    }
    walker.send_move(glam::Vec2::ZERO);
    let final_seq = walker.seq;
    walker.wait_for("server acks the full intent stream under load", Duration::from_secs(5), |b| {
        b.last_ack == final_seq
    });
    let moved = walker.own_pos().unwrap().x - start.x;
    let free_path = PLAYER_SPEED * 5.0;
    eprintln!("walker covered {moved:.1} (free path {free_path:.1}), full stream acked");
    assert!(
        moved > free_path * 0.3,
        "walker covered only {moved:.1} of a {free_path:.1} free path — movement starved"
    );

    stop.store(true, Ordering::Relaxed);
    for driver in drivers {
        let _ = driver.join();
    }
}
