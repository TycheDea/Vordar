use std::time::Duration;

pub fn workspace_root() {
    // Prefabs load from content/ relative to cwd — run as if from workspace root.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(root).unwrap();
}

/// Percentile `p` (0.0-1.0) of `values`, sorted ascending in place.
pub fn percentile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    let idx = ((values.len() as f64 * p) as usize).min(values.len() - 1);
    values[idx]
}

/// Join `handle` on a helper thread so a hang past `timeout` fails the test
/// instead of blocking it forever; `label` names the joined thread in the
/// panic message.
pub fn join_with_deadline<T: Send + 'static>(handle: std::thread::JoinHandle<T>, timeout: Duration, label: &str) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(handle.join());
    });
    rx.recv_timeout(timeout)
        .unwrap_or_else(|_| panic!("{label} did not exit within the deadline"))
        .unwrap_or_else(|_| panic!("{label} panicked instead of shutting down cleanly"))
}

/// Deterministic LCG shared by the benchmarks' scenario builders and
/// server/vordar-server/tests/soak.rs's Wander.
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(2862933555777941757).wrapping_add(3037000493))
    }

    /// Advance the state and return it.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }

    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 33) as u32) as f32 / (u32::MAX as f32 + 1.0)
    }
}
