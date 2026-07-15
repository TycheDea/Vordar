use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

/// Failure window for the per-IP login rate limiter: failure timestamps
/// older than this are pruned before every check.
const LOGIN_FAIL_WINDOW_MICROS: u64 = 10_000_000;
/// Failures within the window before further logins from that IP are denied
/// `RateLimited`.
const MAX_LOGIN_FAILURES: usize = 5;

/// Failed-login ledger, per source IP: bounds credential brute-force /
/// name-probing without touching successful logins — every multi-bot test,
/// the 200-bot soak, and the dev single-player pack log in from 127.0.0.1, so
/// a limit on SUCCESSFUL logins would need config plumbing through every
/// server constructor just to keep the workspace green. Only failures count.
pub(super) struct LoginFailures {
    pub(super) by_ip: HashMap<IpAddr, VecDeque<u64>>,
}

impl LoginFailures {
    pub(super) fn new() -> Self {
        Self { by_ip: HashMap::new() }
    }

    /// Record a failed login attempt from `ip` at server time `now`.
    pub(super) fn record(&mut self, ip: IpAddr, now: u64) {
        self.by_ip.entry(ip).or_default().push_back(now);
    }

    /// Prune stamps older than `LOGIN_FAIL_WINDOW_MICROS` and report whether
    /// `ip` is currently over `MAX_LOGIN_FAILURES` within the window. An IP
    /// whose stamps all age out is dropped from the map entirely — pruning
    /// happens on every login attempt (the Login arm calls this before
    /// anything else), so the ledger cannot grow unboundedly across a long
    /// server lifetime.
    pub(super) fn is_limited(&mut self, ip: IpAddr, now: u64) -> bool {
        let Some(stamps) = self.by_ip.get_mut(&ip) else { return false };
        while stamps.front().is_some_and(|&t| now.saturating_sub(t) > LOGIN_FAIL_WINDOW_MICROS) {
            stamps.pop_front();
        }
        let limited = stamps.len() >= MAX_LOGIN_FAILURES;
        let empty = stamps.is_empty();
        if empty {
            self.by_ip.remove(&ip);
        }
        limited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tolerate `MAX_LOGIN_FAILURES - 1` failures, deny at
    /// `MAX_LOGIN_FAILURES` within the window, and forget the IP entirely
    /// once every stamp has aged out — a stale, empty ledger entry must not
    /// linger forever.
    #[test]
    fn login_failures_deny_at_five_and_forget_after_the_window_drains() {
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let mut failures = LoginFailures::new();
        let t0 = 1_000_000_000u64;

        for i in 0..4u64 {
            failures.record(ip, t0 + i);
        }
        assert!(!failures.is_limited(ip, t0 + 4), "4 failures within the window must not be limited");

        failures.record(ip, t0 + 4);
        assert!(failures.is_limited(ip, t0 + 4), "the 5th failure within the window must be limited");

        let after_window = t0 + 4 + LOGIN_FAIL_WINDOW_MICROS + 1;
        assert!(!failures.is_limited(ip, after_window), "failures aged out of the window must not still be limited");
        assert!(
            !failures.by_ip.contains_key(&ip),
            "an IP with no failures left in the window must be dropped, not merely zeroed"
        );
    }
}
