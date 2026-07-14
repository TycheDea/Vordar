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
