// Loss probe (gap C — the WEAKPOINTS #4 evidence): snapshots ride one
// reliable QUIC stream, so a single lost datagram stalls every later frame
// on that stream until the retransmit lands (head-of-line blocking). An
// observer with simulated 50 ms and 200 ms RTT (LAN and WAN paths) and
// receive-side datagram loss (below QUIC — see engine-net's
// `connect_impaired`) measures inter-snapshot arrival gaps at 0/1/3/5 % loss
// for each RTT.
//
// This is a probe, not a budget test: it prints gap p50/p99/max per rate.
// Decision gate for the datagram snapshot path (WEAKPOINTS #4): p99 gap
// > 250 ms or max > 500 ms at 1–5 % loss confirms the stream freezes.
//
//   cargo test -p vordar-server --release --test loss -- --ignored --nocapture

mod common;

use common::{percentile, Bot};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Measurement window per loss rate.
const WINDOW: Duration = Duration::from_secs(30);
/// Simulated RTTs probed — a realistic LAN path (continuity with the
/// original single-RTT baseline) and a realistic WAN path, where one
/// retransmit cycle costs a much larger fraction of the 100 ms snapshot
/// period.
const WAN_RTTS: [Duration; 2] = [Duration::from_millis(50), Duration::from_millis(200)];

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
        // 10 min of sim — comfortably outlives the eight 30 s windows (2 RTTs × 4 loss rates).
        app.run_headless(60.0, Some(60 * 600));
    });
    std::thread::sleep(Duration::from_millis(300));

    // One mover keeps the world changing so snapshots carry real state.
    let mut mover = Bot::connect(addr);
    mover.wait_for("mover welcomed", Duration::from_secs(10), |b| b.player_id.is_some());

    println!("loss probe: {} s window per rate", WINDOW.as_secs());
    for rtt in WAN_RTTS {
        println!("-- simulated rtt {} ms --", rtt.as_millis());
        for loss in [0.0f32, 0.01, 0.03, 0.05] {
            let name = format!("observer-{}-{}", rtt.as_millis(), (loss * 100.0).round() as u32);
            let mut observer = Bot::connect_impaired_as(addr, &name, rtt, loss);
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
            assert!(
                gaps.len() > 50,
                "observer at rtt={}ms loss={loss} saw only {} snapshots",
                rtt.as_millis(),
                gaps.len() + 1
            );
            let p99 = percentile(&mut gaps, 0.99);
            println!(
                "rtt={:>3}ms loss={:>2.0}%  snapshots={}  gap_ms p50={:.0} p99={:.0} max={:.0}",
                rtt.as_millis(),
                loss * 100.0,
                gaps.len() + 1,
                percentile(&mut gaps, 0.50),
                p99,
                gaps.last().unwrap(),
            );
            // Decision gate for the datagram snapshot path (rework 3): a lost
            // datagram is skipped, not retransmitted, so gaps should bound to
            // cadence multiples regardless of RTT — unlike the pre-datagram
            // stream baseline (BASELINE.md), which stayed under the gate only
            // by margin at 200 ms RTT.
            assert!(
                p99 <= 250.0,
                "rtt={}ms loss={loss} p99 gap {p99:.0}ms exceeds the 250ms datagram-era gate",
                rtt.as_millis()
            );
        }
    }
}

/// Upstream (client→server) counterpart of the probe above. MoveIntents now
/// ride an unreliable QUIC datagram with last-3 redundancy (protocol v15,
/// networking rework 3 finding 5) — CastIntent and Login stay on the
/// reliable stream. This probe sends a steady stream of MoveIntents and
/// measures how far the server's applied-intent ack
/// (`Snapshot::last_processed_seq`, mirrored client-side as `Bot::last_ack`)
/// falls behind the bot's own send counter (`Bot::seq`) — redundancy's
/// resilience under upstream loss made observable purely through the real
/// protocol, no server-internal hook required.
///
/// Pre-redundancy calibration run (see git history of this comment, captured
/// by this plan's finding 1 before finding 5 landed, when MoveIntent was a
/// single per-tick message on the one reliable stream) measured p50/p99/max
/// lag of roughly 6/9/10 ticks at realistic WAN loss (0-5%) versus 8/13/15 at
/// 30% and 17/27/29 at 60%: the stream stalled visibly under sustained heavy
/// loss. Post-redundancy, the same `EXTREME_LOSS` rate measures only ~15
/// ticks max (vs a ~9-tick 0%-loss baseline) at 50 ms RTT — last-3 redundancy
/// absorbs all but the rarest 3-in-a-row datagram loss, so lag stays close to
/// baseline instead of compounding into a growing backlog. `EXTREME_LOSS`
/// still proves the impairment mechanism reaches the transport (lag must
/// move at all) while also proving redundancy bounds the damage (lag must
/// not blow up the way the pre-redundancy stream did); 0/1/3/5% are printed
/// for realistic-WAN reference, same as the downstream probe above.
const UPSTREAM_WINDOW: Duration = Duration::from_secs(8);
/// Below the server's 60 Hz per-tick intent-apply rate, so at 0% loss the
/// applied ack tracks the send counter almost exactly — any lag beyond that
/// baseline under loss is the real stall, not queueing backlog.
const UPSTREAM_SEND_INTERVAL: Duration = Duration::from_millis(20);
const EXTREME_LOSS: f32 = 0.6;

#[test]
#[ignore = "loss probe — run with --release --ignored"]
fn loss_probe_upstream_intent_lag() {
    common::workspace_root();
    if cfg!(debug_assertions) {
        eprintln!("WARNING: loss probe running in debug — results will not be representative");
    }
    let addr: SocketAddr = "127.0.0.1:25182".parse().unwrap();
    std::thread::spawn(move || {
        let mut app = vordar_server::build_server_app(addr, ":memory:");
        // 400 s of sim — comfortably outlives 2 RTTs × 5 loss rates × 8 s windows plus settle.
        app.run_headless(60.0, Some(60 * 400));
    });
    std::thread::sleep(Duration::from_millis(300));

    println!("upstream loss probe: {} s window per rate", UPSTREAM_WINDOW.as_secs());
    for rtt in WAN_RTTS {
        println!("-- simulated rtt {} ms --", rtt.as_millis());
        let mut baseline_max: Option<u32> = None;
        for loss in [0.0f32, 0.01, 0.03, 0.05, EXTREME_LOSS] {
            let name = format!("upstreamer-{}-{}", rtt.as_millis(), (loss * 100.0).round() as u32);
            let mut bot = Bot::connect_upstream_impaired_as(addr, &name, rtt, loss);
            bot.wait_for("bot welcomed", Duration::from_secs(30), |b| b.player_id.is_some());
            common::settle(&mut bot, Duration::from_secs(2));

            let mut dir = glam::Vec2::X;
            let mut lags: Vec<u32> = Vec::new();
            let end = Instant::now() + UPSTREAM_WINDOW;
            let mut ticks = 0u32;
            while Instant::now() < end {
                ticks += 1;
                if ticks % 100 == 0 {
                    dir = -dir;
                }
                bot.send_move(dir);
                bot.pump();
                lags.push(bot.seq.saturating_sub(bot.last_ack));
                std::thread::sleep(UPSTREAM_SEND_INTERVAL);
            }

            assert!(
                lags.len() > 50,
                "bot at rtt={}ms loss={loss} upstream loss only sampled {} ticks",
                rtt.as_millis(),
                lags.len()
            );
            lags.sort_unstable();
            let p50 = lags[lags.len() / 2];
            let p99 = lags[(lags.len() * 99 / 100).min(lags.len() - 1)];
            let max = *lags.last().unwrap();
            println!(
                "rtt={:>3}ms upstream loss={:>2.0}%  sent={}  samples={}  lag p50={} p99={} max={}",
                rtt.as_millis(),
                loss * 100.0,
                bot.seq,
                lags.len(),
                p50,
                p99,
                max,
            );

            if loss == 0.0 {
                baseline_max = Some(max);
            }
            if loss == EXTREME_LOSS {
                let baseline = baseline_max.expect("0% loss rate must run first to establish a baseline");
                assert!(
                    max > baseline && max < baseline * 3,
                    "upstream loss={loss} at rtt={}ms should show some cost from move-intent loss \
                     (> {baseline}-tick 0%-loss baseline — proves try_send drop reaches the transport) \
                     but stay bounded by last-3 redundancy (< {} — proves redundancy contains the \
                     damage instead of stalling like the old single-send reliable stream) — got max={max}",
                    rtt.as_millis(),
                    baseline * 3
                );
            }
        }
    }
}
