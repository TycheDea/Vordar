// Zone supervision and restart policy: catches panics from zone threads,
// restarts on the same address if the restart budget allows, and re-raises
// once the budget is exhausted or a shutdown signal is already set.

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Join every zone thread, logging loudly if one panicked instead of exiting
/// cleanly — silently discarding `handle.join()`'s `Result` would leave a
/// panicked zone dead with no trace at all. Every zone thread runs under
/// `supervise_zone`, so a panic is normally caught and the zone rebuilds on
/// the same address automatically — this log fires only once the restart
/// budget (`MAX_ZONE_RESTARTS`) is spent, or the shutdown flag was already
/// set when the panic was caught: the zone is then genuinely, permanently
/// down, its listener stays bound but dead, and every other zone keeps
/// redirecting players into that now-dead address until the process is
/// restarted.
pub fn join_zone_threads(handles: Vec<(String, std::thread::JoinHandle<()>)>) {
    for (name, handle) in handles {
        if let Err(payload) = handle.join() {
            log::error!(
                "zone '{name}' thread panicked and exited ({}); its listener is now dead — \
                 other zones will keep redirecting players into a stale address until the \
                 process is restarted",
                panic_message(&payload),
            );
        }
    }
}

/// Best-effort human-readable text from a caught panic payload — `panic!`
/// with a literal yields `&'static str`, `panic!("...{}...")` yields `String`;
/// anything else (a custom payload via `panic_any`) has no reliable `Display`.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned())
}

/// Max consecutive *fast* zone-panic restarts before the supervisor gives up
/// and re-raises the panic. A run that survives `HEALTHY_UPTIME` resets the
/// strike count, so this bounds hot crash loops
/// (bad content, corrupt state that panics on the first tick), not a
/// long-lived server's lifetime recovery budget.
pub const MAX_ZONE_RESTARTS: u32 = 3;

/// A zone run lasting at least this long is healthy and forgives any earlier
/// strikes before the next panic is counted.
const HEALTHY_UPTIME: Duration = Duration::from_secs(60);

/// Pure strike-accounting step, kept `Duration`-parameterized (rather than
/// reading a clock itself) so it is unit-testable without waiting 60 real
/// seconds: a run that survived `HEALTHY_UPTIME` resets the count to 1 (the
/// new panic itself always counts as one strike); otherwise the streak
/// continues.
fn next_strikes(prev: u32, ran_for: Duration) -> u32 {
    if ran_for >= HEALTHY_UPTIME {
        1
    } else {
        prev + 1
    }
}

/// Run `run_zone` under a restart-on-panic supervisor. A panic unwinding out
/// of `run_zone` drops everything it built — in
/// particular the App's `NetServer`, whose `Drop`
/// (`engine-net/src/server.rs:240-253`) closes the QUIC endpoint and joins
/// the network thread — so by the time `catch_unwind` returns it is safe to
/// call `run_zone` again and rebuild from scratch on the same address.
/// Consecutive fast failures (ones that don't survive `HEALTHY_UPTIME`) are
/// bounded by `MAX_ZONE_RESTARTS`; once that budget is spent, or `shutdown`
/// is already set when a panic is caught, the original panic payload is
/// re-raised via `resume_unwind` so `join_zone_threads` reports it exactly as
/// an unsupervised panic would — a clean return from `run_zone` (e.g. a
/// shutdown drain finishing) ends supervision with no restart.
pub fn supervise_zone(name: &str, shutdown: &AtomicBool, mut run_zone: impl FnMut()) {
    let mut strikes = 0u32;
    loop {
        let started = Instant::now();
        match catch_unwind(AssertUnwindSafe(&mut run_zone)) {
            Ok(()) => return,
            Err(payload) => {
                strikes = next_strikes(strikes, started.elapsed());
                if shutdown.load(Ordering::Relaxed) || strikes > MAX_ZONE_RESTARTS {
                    resume_unwind(payload);
                }
                log::error!(
                    "zone '{name}' panicked ({}); restarting (strike {strikes}/{MAX_ZONE_RESTARTS})",
                    panic_message(&payload)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn panic_message_extracts_string_literal_payload() {
        let handle = std::thread::spawn(|| panic!("zone exploded"));
        let payload = handle.join().unwrap_err();
        assert_eq!(panic_message(&payload), "zone exploded");
    }

    #[test]
    fn panic_message_extracts_formatted_string_payload() {
        let cause = "bad zones.ron";
        let handle = std::thread::spawn(move || panic!("startup failed: {cause}"));
        let payload = handle.join().unwrap_err();
        assert_eq!(panic_message(&payload), "startup failed: bad zones.ron");
    }

    /// A panicked zone thread must not prevent `join_zone_threads` from
    /// joining (and thus not silently dropping) the other zone threads
    /// behind it, and must not itself propagate the panic and take the whole
    /// process down with it.
    #[test]
    fn a_panicked_zone_does_not_stop_the_others_from_being_joined() {
        let joined = Arc::new(AtomicUsize::new(0));
        let healthy_before = {
            let joined = joined.clone();
            std::thread::Builder::new().spawn(move || { joined.fetch_add(1, Ordering::SeqCst); }).unwrap()
        };
        let panicking = std::thread::Builder::new().spawn(|| panic!("zone crashed")).unwrap();
        let healthy_after = {
            let joined = joined.clone();
            std::thread::Builder::new().spawn(move || { joined.fetch_add(1, Ordering::SeqCst); }).unwrap()
        };
        join_zone_threads(vec![
            ("start".to_owned(), healthy_before),
            ("east".to_owned(), panicking),
            ("west".to_owned(), healthy_after),
        ]);
        assert_eq!(joined.load(Ordering::SeqCst), 2, "both healthy zone threads must still be joined despite the panic in between");
    }

    /// One panic followed by a clean return must be fully absorbed by the
    /// supervisor — it retries after the panic and returns normally once
    /// `run_zone` succeeds, having called the closure exactly twice.
    #[test]
    fn supervise_zone_restarts_after_one_panic_then_returns() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_closure = calls.clone();
        let shutdown = AtomicBool::new(false);
        supervise_zone("t", &shutdown, move || {
            let n = calls_in_closure.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                panic!("first run fails");
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// A closure that always panics must be retried exactly
    /// `MAX_ZONE_RESTARTS` times past the first failure (`MAX_ZONE_RESTARTS +
    /// 1` total calls), then the supervisor must re-raise the original
    /// payload so the thread dies panicked with the same message an
    /// unsupervised caller would see.
    #[test]
    fn supervise_zone_gives_up_after_max_restarts_and_repanics_with_original_payload() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_closure = calls.clone();
        let handle = std::thread::spawn(move || {
            let shutdown = AtomicBool::new(false);
            supervise_zone("t", &shutdown, move || {
                calls_in_closure.fetch_add(1, Ordering::SeqCst);
                panic!("always fails");
            });
        });
        let payload = handle.join().unwrap_err();
        assert_eq!(panic_message(&payload), "always fails");
        assert_eq!(calls.load(Ordering::SeqCst), (MAX_ZONE_RESTARTS + 1) as usize);
    }

    /// If the shutdown flag is already set when the first panic is caught,
    /// the supervisor must not restart at all — one call, immediate re-raise.
    #[test]
    fn supervise_zone_does_not_restart_when_shutdown_flag_already_set() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_closure = calls.clone();
        let handle = std::thread::spawn(move || {
            let shutdown = AtomicBool::new(true);
            supervise_zone("t", &shutdown, move || {
                calls_in_closure.fetch_add(1, Ordering::SeqCst);
                panic!("boom");
            });
        });
        let payload = handle.join().unwrap_err();
        assert_eq!(panic_message(&payload), "boom");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// A run that survived `HEALTHY_UPTIME` resets the strike count to 1 for
    /// the new panic; a fast failure just accumulates.
    #[test]
    fn next_strikes_resets_after_healthy_uptime_but_accumulates_otherwise() {
        assert_eq!(next_strikes(3, Duration::from_secs(61)), 1);
        assert_eq!(next_strikes(1, Duration::from_millis(100)), 2);
    }
}
