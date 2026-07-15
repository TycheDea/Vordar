use std::time::Duration;

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
