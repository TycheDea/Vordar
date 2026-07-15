// Per-connection and aggregate metrics for the network layer.
// Exposed via periodic snapshots; zero-cost when unused.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct NetMetrics {
    pub frames_in: AtomicU64,
    pub frames_out: AtomicU64,
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    pub rejects: AtomicU64,
    pub writer_queue_depth: AtomicU64,
    /// Cumulative microseconds the network thread's busy-time canary (see
    /// `server::server_main`) woke up late by — on the single-threaded
    /// runtime, lateness is time the thread spent running other tasks
    /// (handshakes, frame codec, accept loop) instead of idling, so this is
    /// a proxy for how saturated the network thread is.
    pub busy_micros: AtomicU64,
    /// Datagrams received on the unreliable lane — counted regardless of
    /// tag (ctrl ping or app payload).
    pub datagrams_in: AtomicU64,
    /// Datagrams successfully handed to `quinn::Connection::send_datagram`
    /// on the unreliable lane.
    pub datagrams_out: AtomicU64,
    /// A `send_datagram` call that failed (connection closing, payload too
    /// large) and was dropped instead of falling back to the stream —
    /// datagrams are best-effort by contract; the next cadence supersedes.
    pub datagram_send_failures: AtomicU64,
}

impl NetMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    #[inline]
    pub fn record_frame_in(&self, bytes: usize) {
        self.frames_in.fetch_add(1, Ordering::Relaxed);
        self.bytes_in.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_frame_out(&self, bytes: usize) {
        self.frames_out.fetch_add(1, Ordering::Relaxed);
        self.bytes_out.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_reject(&self) {
        self.rejects.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_datagram_in(&self) {
        self.datagrams_in.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_datagram_out(&self) {
        self.datagrams_out.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_datagram_send_failure(&self) {
        self.datagram_send_failures.fetch_add(1, Ordering::Relaxed);
    }
}