// Loss probe (gap C — the WEAKPOINTS #4 evidence): snapshots ride one
// reliable QUIC stream, so a single lost datagram stalls every later frame
// on that stream until the retransmit lands (head-of-line blocking). An
// observer with simulated 50 ms RTT and receive-side datagram loss (below
// QUIC — see engine-net's `connect_impaired`) measures inter-snapshot
// arrival gaps at 0/1/3/5 % loss.
//
// This is a probe, not a budget test: it prints gap p50/p99/max per rate.
// Decision gate for the datagram snapshot path (WEAKPOINTS #4): p99 gap
// > 250 ms or max > 500 ms at 1–5 % loss confirms the stream freezes.
//
//   cargo test -p vordar-server --release --test loss -- --ignored --nocapture

mod common;

use common::Bot;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Measurement window per loss rate.
const WINDOW: Duration = Duration::from_secs(30);
/// Simulated observer RTT — a realistic WAN path, and enough that a
/// retransmit costs a visible fraction of the 100 ms snapshot period.
const RTT: Duration = Duration::from_millis(50);

fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)]
}

#[test]
#[ignore = "loss probe — run with --release --ignored"]
fn loss_probe_inter_snapshot_gaps() {
    common::workspace_root();
    if cfg!(debug_assertions) {
        eprintln!("WARNING: loss probe running in debug — results will not be representative");
    }
    let addr: SocketAddr = "127.0.0.1:25181".parse().unwrap();
    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        // 10 min of sim — comfortably outlives the four 30 s windows.
        app.run_headless(60.0, Some(60 * 600));
    });
    std::thread::sleep(Duration::from_millis(300));

    // One mover keeps the world changing so snapshots carry real state.
    let mut mover = Bot::connect(addr);
    mover.wait_for("mover welcomed", Duration::from_secs(10), |b| b.player_id.is_some());

    println!("loss probe: {} s window per rate, simulated rtt {} ms", WINDOW.as_secs(), RTT.as_millis());
    for loss in [0.0f32, 0.01, 0.03, 0.05] {
        let name = format!("observer-{}", (loss * 100.0).round() as u32);
        let mut observer = Bot::connect_impaired_as(addr, &name, RTT, loss);
        observer.wait_for("observer welcomed", Duration::from_secs(30), |b| b.player_id.is_some());
        common::settle(&mut observer, Duration::from_secs(2));
        observer.snapshot_at.clear();

        // The mover oscillates near spawn (stays inside the observer's AOI).
        let mut dir = glam::Vec2::X;
        let mut pumps = 0u32;
        let end = Instant::now() + WINDOW;
        while Instant::now() < end {
            pumps += 1;
            if pumps % 120 == 0 {
                dir = -dir;
            }
            mover.send_move(dir);
            mover.pump();
            observer.pump();
            std::thread::sleep(Duration::from_millis(4));
        }

        let mut gaps: Vec<f64> = observer
            .snapshot_at
            .windows(2)
            .map(|w| (w[1] - w[0]).as_secs_f64() * 1e3)
            .collect();
        assert!(gaps.len() > 50, "observer at {loss} loss saw only {} snapshots", gaps.len() + 1);
        gaps.sort_by(|a, b| a.total_cmp(b));
        println!(
            "loss={:>2.0}%  snapshots={}  gap_ms p50={:.0} p99={:.0} max={:.0}",
            loss * 100.0,
            gaps.len() + 1,
            pct(&gaps, 0.50),
            pct(&gaps, 0.99),
            gaps.last().unwrap(),
        );
    }
}
