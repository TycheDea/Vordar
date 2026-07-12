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
    /// a proxy for how saturated the network thread is (networking audit
    /// 2026-07-11, finding 14 step 1).
    pub busy_micros: AtomicU64,
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
}