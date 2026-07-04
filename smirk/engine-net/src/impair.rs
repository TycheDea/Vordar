// Network impairment for testing: a client endpoint whose UDP receive path
// drops datagrams BELOW QUIC. Dropping at this layer exercises the real
// retransmission machinery — a dropped stream frame stalls the stream until
// QUIC retransmits it, which is exactly the head-of-line phenomenon the loss
// probes measure. (Dropping frames above QUIC, after reliable delivery,
// cannot reproduce that.) Client-side receive drop == server→client loss.

use crate::NetError;
use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, Runtime, UdpPoller};
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

/// Client endpoint that drops received datagrams with probability `loss`.
pub(crate) fn lossy_client_endpoint(bind: SocketAddr, loss: f32) -> Result<quinn::Endpoint, NetError> {
    let socket = std::net::UdpSocket::bind(bind)?;
    let runtime = Arc::new(quinn::TokioRuntime);
    let inner = runtime.wrap_udp_socket(socket)?;
    quinn::Endpoint::new_with_abstract_socket(
        quinn::EndpointConfig::default(),
        None,
        Arc::new(LossySocket { inner, loss, rng: Mutex::new(0x9E37_79B9_7F4A_7C15) }),
        runtime,
    )
    .map_err(NetError::Io)
}

#[derive(Debug)]
struct LossySocket {
    inner: Arc<dyn AsyncUdpSocket>,
    loss: f32,
    /// Deterministic LCG state — same seed, same drop pattern.
    rng: Mutex<u64>,
}

impl LossySocket {
    fn drop_next(&self) -> bool {
        let mut s = self.rng.lock().unwrap();
        *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*s >> 40) as f32 / (1u64 << 24) as f32) < self.loss
    }
}

impl AsyncUdpSocket for LossySocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        self.inner.clone().create_io_poller()
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
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
        // the whole burst (bursty loss, still `loss` per receive event).
        loop {
            match self.inner.poll_recv(cx, &mut bufs[..1], &mut meta[..1]) {
                Poll::Ready(Ok(n)) if n >= 1 && self.drop_next() => continue,
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
