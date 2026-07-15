// Network impairment for testing — the whole conditioner lives here:
// the `Impairment` knob set with named WAN profiles, the `Jitter` +
// `delay_reorder` delay/reorder pipeline stage client.rs inserts on each
// lane, `skewed_micros` clock scaling for the skew harness, and a client
// endpoint whose UDP send/receive paths drop datagrams BELOW QUIC.
// Dropping at the UDP layer exercises the real retransmission machinery —
// a dropped stream frame stalls the stream until QUIC retransmits it,
// which is exactly the head-of-line phenomenon the loss probes measure.
// (Dropping frames above QUIC, after reliable delivery, cannot reproduce
// that.) Client-side receive drop == server→client loss; client-side send
// drop == client→server loss.

use crate::NetError;
use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, Runtime, UdpPoller};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Both-direction network conditioner knobs: latency, independent
/// upstream/downstream datagram loss, jitter/reorder, and simulated clock
/// skew. Testing only; every field defaults to "no impairment".
#[derive(Clone, Copy, Debug, Default)]
pub struct Impairment {
    /// Simulated round-trip time; each direction is delayed `rtt / 2`.
    pub rtt: Duration,
    /// Probability (0.0–1.0) that a server→client datagram is dropped below
    /// QUIC, exercising real retransmission (see `lossy_client_endpoint`).
    pub downstream_loss: f32,
    /// Probability (0.0–1.0) that a client→server datagram is dropped below
    /// QUIC.
    pub upstream_loss: f32,
    /// Extra, per-frame random delay drawn uniformly from `[0, jitter]` and
    /// added on top of the fixed one-way latency, in both directions. Unlike
    /// the fixed latency, jitter can reorder frames relative to each other
    /// (see `delay_reorder`).
    pub jitter: Duration,
    /// Simulated client clock drift, in parts-per-million of real elapsed
    /// time. Feeds every "local now" this client reports or times against
    /// (`Ping.t_client`, `local_micros()`), so `ClockSync`'s drift-rate
    /// estimate has something real to track under a live connection instead
    /// of only in `ClockSync`'s own unit tests.
    pub clock_skew_ppm: f64,
}

impl Impairment {
    /// Fixed symmetric latency only — no loss, jitter, or skew.
    pub fn latency(rtt: Duration) -> Self {
        Self { rtt, ..Default::default() }
    }

    /// Named WAN profiles: representative combined latency/jitter/loss/skew
    /// shapes so client-feel and clock-sync claims have a headless test at
    /// recognizable real-world conditions, not just hand-picked individual
    /// numbers.
    pub fn wifi() -> Self {
        Self {
            rtt: Duration::from_millis(20),
            downstream_loss: 0.001,
            upstream_loss: 0.001,
            jitter: Duration::from_millis(5),
            clock_skew_ppm: 5.0,
        }
    }

    pub fn four_g() -> Self {
        Self {
            rtt: Duration::from_millis(70),
            downstream_loss: 0.01,
            upstream_loss: 0.01,
            jitter: Duration::from_millis(25),
            clock_skew_ppm: 20.0,
        }
    }

    pub fn satellite() -> Self {
        Self {
            rtt: Duration::from_millis(600),
            downstream_loss: 0.02,
            upstream_loss: 0.02,
            jitter: Duration::from_millis(40),
            clock_skew_ppm: 50.0,
        }
    }
}

/// Local-clock scaling for the `clock_skew_ppm` harness: a real client's
/// crystal doesn't tick at exactly the server's rate, by tens of ppm over a
/// long session. Scaling real elapsed time by `1 + skew_ppm/1e6` gives
/// clock-sync a genuine, growing offset to correct for instead of a step
/// outside the simulation entirely.
pub(crate) fn skewed_micros(elapsed: Duration, skew_ppm: f64) -> u64 {
    if skew_ppm == 0.0 {
        return elapsed.as_micros() as u64;
    }
    (elapsed.as_micros() as f64 * (1.0 + skew_ppm / 1_000_000.0)).max(0.0) as u64
}

/// Deterministic per-connection jitter source (same LCG technique as
/// `LossySocket`): draws an extra delay in `[0, max]` for each frame passing
/// through `delay_reorder`.
pub(crate) struct Jitter {
    rng: u64,
    max: Duration,
}

impl Jitter {
    /// A distinct seed per pipeline (writer app frames, writer ctrl frames,
    /// reader) keeps each independently deterministic without sharing a
    /// mutex-guarded RNG across tasks.
    pub(crate) fn with_seed(max: Duration, seed: u64) -> Self {
        Self { rng: seed, max }
    }

    pub(crate) fn sample(&mut self) -> Duration {
        if self.max.is_zero() {
            return Duration::ZERO;
        }
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let frac = (self.rng >> 40) as f64 / (1u64 << 24) as f64; // [0, 1)
        self.max.mul_f64(frac)
    }
}

/// One item in `delay_reorder`'s pending set: ordered by delivery deadline
/// only (ties broken by arrival order) so the payload itself need not be `Ord`.
struct Pending<T> {
    at: tokio::time::Instant,
    seq: u64,
    item: T,
}

impl<T> PartialEq for Pending<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.at, self.seq) == (other.at, other.seq)
    }
}
impl<T> Eq for Pending<T> {}
impl<T> PartialOrd for Pending<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Pending<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed: `BinaryHeap` is a max-heap, but delivery must pop the
        // *earliest* deadline first.
        (other.at, other.seq).cmp(&(self.at, self.seq))
    }
}

/// Delay pipeline stage for latency/jitter simulation: items arrive tagged
/// with their intended delivery instant and are released in *deadline*
/// order, not enqueue order. That distinction is what makes jitter able to
/// reorder frames — under a plain delayed FIFO channel, a later item
/// drawing a smaller delay than an earlier one still waits behind it; here
/// it legitimately overtakes, the way real jitter reorders packets on the
/// wire.
pub(crate) async fn delay_reorder<T: Send + 'static>(
    mut rx: UnboundedReceiver<(tokio::time::Instant, T)>,
    tx: UnboundedSender<T>,
) {
    let mut pending: BinaryHeap<Pending<T>> = BinaryHeap::new();
    let mut next_seq = 0u64;
    loop {
        match pending.peek() {
            Some(next) => {
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep_until(next.at) => {
                        if let Some(p) = pending.pop() {
                            if tx.send(p.item).is_err() { return; }
                        }
                    }
                    maybe = rx.recv() => match maybe {
                        Some((at, item)) => { pending.push(Pending { at, seq: next_seq, item }); next_seq += 1; }
                        None => break,
                    },
                }
            }
            None => match rx.recv().await {
                Some((at, item)) => { pending.push(Pending { at, seq: next_seq, item }); next_seq += 1; }
                None => return,
            },
        }
    }
    // Channel closed: drain whatever is left, still in deadline order.
    while let Some(p) = pending.pop() {
        tokio::time::sleep_until(p.at).await;
        let _ = tx.send(p.item);
    }
}

/// Client endpoint that drops received datagrams with probability
/// `downstream_loss` (server→client) and sent datagrams with probability
/// `upstream_loss` (client→server).
pub(crate) fn lossy_client_endpoint(
    bind: SocketAddr,
    downstream_loss: f32,
    upstream_loss: f32,
) -> Result<quinn::Endpoint, NetError> {
    let socket = std::net::UdpSocket::bind(bind)?;
    let runtime = Arc::new(quinn::TokioRuntime);
    let inner = runtime.wrap_udp_socket(socket)?;
    quinn::Endpoint::new_with_abstract_socket(
        quinn::EndpointConfig::default(),
        None,
        Arc::new(LossySocket {
            inner,
            downstream_loss,
            upstream_loss,
            rng: Mutex::new(0x9E37_79B9_7F4A_7C15),
        }),
        runtime,
    )
    .map_err(NetError::Io)
}

#[derive(Debug)]
struct LossySocket {
    inner: Arc<dyn AsyncUdpSocket>,
    downstream_loss: f32,
    upstream_loss: f32,
    /// Deterministic LCG state — same seed, same drop pattern. Shared between
    /// the send and receive paths: still deterministic (single mutex, total
    /// order of draws is whatever order the two paths happen to call in),
    /// just not independently replayable per direction.
    rng: Mutex<u64>,
}

impl LossySocket {
    fn roll(&self, p: f32) -> bool {
        if p <= 0.0 {
            return false;
        }
        let mut s = self.rng.lock().unwrap();
        *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*s >> 40) as f32 / (1u64 << 24) as f32) < p
    }
}

impl AsyncUdpSocket for LossySocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        self.inner.clone().create_io_poller()
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        if self.roll(self.upstream_loss) {
            // Pretend it was sent: a real lost datagram never errors back to
            // the sender either. QUIC's loss detection/retransmission on the
            // other end handles the rest, exactly like the receive-side drop.
            return Ok(());
        }
        self.inner.try_send(transmit)
    }

    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        // One receive per poll: dropping from the middle of a batch would
        // mean compacting buffer contents. Loss simulation is a test path —
        // simplicity over batching throughput. Note a single RecvMeta may
        // still carry several GRO-coalesced datagrams; a drop then discards
        // the whole burst (bursty loss, still `downstream_loss` per receive event).
        loop {
            match self.inner.poll_recv(cx, &mut bufs[..1], &mut meta[..1]) {
                Poll::Ready(Ok(n)) if n >= 1 && self.roll(self.downstream_loss) => continue,
                other => return other,
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal fake socket that just counts how many datagrams actually
    /// reached it — isolates `LossySocket::try_send`'s drop decision from
    /// real QUIC/retransmission timing, which is too noisy for a fast,
    /// deterministic test.
    #[derive(Debug, Default)]
    struct CountingSocket {
        sends: AtomicUsize,
    }

    impl AsyncUdpSocket for CountingSocket {
        fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
            unimplemented!("not exercised by try_send")
        }

        fn try_send(&self, _transmit: &Transmit) -> io::Result<()> {
            self.sends.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn poll_recv(
            &self,
            _cx: &mut Context,
            _bufs: &mut [IoSliceMut<'_>],
            _meta: &mut [RecvMeta],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }

        fn local_addr(&self) -> io::Result<SocketAddr> {
            Ok("127.0.0.1:0".parse().unwrap())
        }
    }

    fn make_socket(upstream_loss: f32, inner: Arc<CountingSocket>) -> LossySocket {
        LossySocket { inner, downstream_loss: 0.0, upstream_loss, rng: Mutex::new(0x9E37_79B9_7F4A_7C15) }
    }

    fn dummy_transmit(contents: &[u8]) -> Transmit<'_> {
        Transmit {
            destination: "127.0.0.1:1".parse().unwrap(),
            ecn: None,
            contents,
            segment_size: None,
            src_ip: None,
        }
    }

    /// `upstream_loss = 1.0` must drop every send below the real socket.
    #[test]
    fn upstream_loss_of_one_drops_every_send() {
        let inner = Arc::new(CountingSocket::default());
        let socket = make_socket(1.0, inner.clone());
        let payload = [0u8; 16];
        for _ in 0..200 {
            socket.try_send(&dummy_transmit(&payload)).expect("try_send must not error");
        }
        assert_eq!(inner.sends.load(Ordering::Relaxed), 0, "loss=1.0 must never reach the real socket");
    }

    /// `upstream_loss = 0.0` must forward every send unchanged.
    #[test]
    fn upstream_loss_of_zero_forwards_every_send() {
        let inner = Arc::new(CountingSocket::default());
        let socket = make_socket(0.0, inner.clone());
        let payload = [0u8; 16];
        for _ in 0..200 {
            socket.try_send(&dummy_transmit(&payload)).expect("try_send must not error");
        }
        assert_eq!(inner.sends.load(Ordering::Relaxed), 200, "loss=0.0 must forward every send");
    }

    /// A mid-range probability must drop *some* but not *all* sends, and land
    /// close to the configured rate over enough draws — proof `roll` (the
    /// same LCG technique the receive-side drop uses) is wired correctly for
    /// the send path, not just always-true/always-false.
    #[test]
    fn upstream_loss_of_half_drops_roughly_half() {
        let inner = Arc::new(CountingSocket::default());
        let socket = make_socket(0.5, inner.clone());
        let payload = [0u8; 16];
        const N: usize = 20_000;
        for _ in 0..N {
            socket.try_send(&dummy_transmit(&payload)).expect("try_send must not error");
        }
        let sent = inner.sends.load(Ordering::Relaxed);
        let rate = sent as f64 / N as f64;
        assert!(
            (rate - 0.5).abs() < 0.05,
            "loss=0.5 over {N} draws should forward close to half, forwarded {sent} ({rate:.3})"
        );
    }
}
