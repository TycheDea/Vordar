//! `Impairment` simulates server→client loss, jitter/reorder, and client
//! clock skew — not just downstream loss.
//!
//! Send-side loss on `try_send` is covered by deterministic unit tests in
//! `impair.rs` itself (`LossySocket::try_send` against a fake socket) rather
//! than here: on a live QUIC connection the dropped datagram is
//! transparently retransmitted (it rides the one reliable stream), so the
//! *observable* effect is a timing stall too noisy for a fast test to
//! assert a hard bound on — that cadence effect is what the ignored,
//! `--release`-only probe in `vordar-server`'s `tests/loss.rs`
//! (`loss_probe_upstream_intent_lag`) measures instead.

use engine_net::{Impairment, NetClient, NetServer, ServerEvent};
use std::time::{Duration, Instant};

fn wait_connected(server: &mut NetServer) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if server.poll().into_iter().any(|ev| matches!(ev, ServerEvent::Connected(_))) {
            return;
        }
        assert!(Instant::now() < deadline, "client never connected");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A per-frame random extra delay must be able to reorder frames relative
/// to their send order. Sends a sequence of numbered frames well under the
/// token-bucket refill rate, with jitter several times larger than the send
/// spacing, and checks the server sees at least one inversion in arrival
/// order — real reordering through the actual `NetClient`/`NetServer`
/// pipeline, not a direct test of the private delay queue.
#[test]
fn jitter_reorders_frames_relative_to_send_order() {
    let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
    let impairment = Impairment { jitter: Duration::from_millis(30), ..Default::default() };
    let client = NetClient::connect_impaired(server.local_addr(), 1, impairment).expect("connect");
    wait_connected(&mut server);

    const N: u32 = 100;
    for i in 0..N {
        client.send(i.to_le_bytes().to_vec());
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut received: Vec<u32> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while received.len() < N as usize && Instant::now() < deadline {
        for ev in server.poll() {
            if let ServerEvent::Message { data, .. } = ev {
                received.push(u32::from_le_bytes(data.try_into().expect("4-byte payload")));
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(received.len(), N as usize, "all frames should arrive under loss-free jitter");

    let inversions = received.windows(2).filter(|w| w[0] > w[1]).count();
    assert!(
        inversions > 0,
        "jitter should have reordered at least one adjacent pair; arrival order was perfectly \
         monotonic: {received:?}"
    );
}

/// A simulated client clock running at a different rate than real elapsed
/// time: `local_micros()` (the basis for every Ping/`server_now_micros`
/// computation) must reflect the configured ppm skew against real wall time.
#[test]
fn clock_skew_harness_skews_reported_local_time() {
    let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
    // 10% fast clock — large enough to measure reliably over a short sleep.
    let impairment = Impairment { clock_skew_ppm: 100_000.0, ..Default::default() };
    let client = NetClient::connect_impaired(server.local_addr(), 1, impairment).expect("connect");
    wait_connected(&mut server);

    let wall_start = Instant::now();
    let reported_start = client.local_micros();
    std::thread::sleep(Duration::from_millis(300));
    let wall_elapsed_us = wall_start.elapsed().as_micros() as f64;
    let reported_elapsed_us = (client.local_micros() - reported_start) as f64;

    let ratio = reported_elapsed_us / wall_elapsed_us;
    assert!(
        (ratio - 1.10).abs() < 0.02,
        "10% clock_skew_ppm should make local_micros() run ~10% faster than real time, got ratio {ratio:.4}"
    );
}

