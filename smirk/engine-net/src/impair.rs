// Network impairment for testing: a client endpoint whose UDP send/receive
// paths drop datagrams BELOW QUIC. Dropping at this layer exercises the real
// retransmission machinery — a dropped stream frame stalls the stream until
// QUIC retransmits it, which is exactly the head-of-line phenomenon the loss
// probes measure. (Dropping frames above QUIC, after reliable delivery,
// cannot reproduce that.) Client-side receive drop == server→client loss;
// client-side send drop == client→server loss (networking audit 2026-07-11,
// finding 17 — the send-drop half was missing entirely).

use crate::NetError;
use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, Runtime, UdpPoller};
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

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
    /// deterministic test (networking audit 2026-07-11, finding 17, path
    /// step 1: "drop probability on `try_send`").
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

    /// `upstream_loss = 1.0` must drop every send below the real socket — the
    /// client→server direction that, before finding 17, had no drop path at
    /// all (`try_send` only ever forwarded).
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

    /// `upstream_loss = 0.0` must forward every send unchanged — the
    /// pre-finding-17 behavior for the direction that had no loss knob.
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
    /// same LCG technique the pre-existing receive-side drop already used)
    /// is wired correctly for the send path, not just always-true/always-false.
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
