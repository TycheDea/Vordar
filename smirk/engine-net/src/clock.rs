// Clock-sync filter behind NetClient's published server-time offset —
// windowed-minimum RTT sample selection, a least-squares drift-rate
// estimate, and a slew limiter on the published offset. Pure and
// network-free: client.rs feeds it Pong samples and reads the offset.
// The sync cadence (initial burst, steady-state re-check) lives here
// with the filter it parameterizes; client.rs's pinger drives it.

use std::collections::VecDeque;
use std::time::Duration;

/// Initial sync burst: enough samples to find a low-RTT one fast.
pub(crate) const SYNC_BURST_PINGS: u32 = 8;
pub(crate) const SYNC_BURST_INTERVAL: Duration = Duration::from_millis(100);
/// Steady-state re-check (DESIGN.md §3: "re-checked occasionally").
pub(crate) const SYNC_INTERVAL: Duration = Duration::from_secs(10);

/// Sliding window over which the lowest-RTT sample is treated as the sync
/// anchor, instead of the connection's all-time best. Once an early
/// low-RTT sample ages out of this window, later samples resume driving the
/// offset instead of one lucky sample pinning it for the rest of the
/// session.
const SYNC_WINDOW: Duration = Duration::from_secs(90);
/// Maximum rate, in parts-per-million of elapsed local time, at which the
/// published offset may move toward a new target. A correction is always a
/// slew, never a step, so a reader mid-correction (telegraph countdowns,
/// intent deadlines) never observes a jump.
const MAX_SLEW_PPM: f64 = 2_000.0;

/// One clock-sync ping/pong round-trip.
struct ClockSample {
    /// Local (client-epoch) micros at which this sample was measured.
    t_local: u64,
    rtt: u64,
    /// Raw local→server offset implied by this sample alone.
    raw_offset: i64,
}

/// Clock-sync filter behind `NetClient`'s published offset: windowed-minimum
/// RTT selection (not all-time-best), a linear drift-rate estimate that
/// projects the chosen sample forward to "now", and a slew limiter on the
/// published offset. Pure and network-free, so it is unit-testable without a
/// live connection.
pub(crate) struct ClockSync {
    samples: VecDeque<ClockSample>,
    offset: i64,
    rtt: u64,
    synced: bool,
    last_local: u64,
}

impl ClockSync {
    pub(crate) fn new() -> Self {
        Self { samples: VecDeque::new(), offset: 0, rtt: 0, synced: false, last_local: 0 }
    }

    /// Feed one Pong sample. `t_local` is the local micros at which it was
    /// received/measured; `t_server` and `rtt` come straight off the wire.
    pub(crate) fn on_pong(&mut self, t_local: u64, t_server: u64, rtt: u64) {
        let raw_offset = (t_server as i64 + (rtt / 2) as i64) - t_local as i64;
        self.samples.push_back(ClockSample { t_local, rtt, raw_offset });
        let window_start = t_local.saturating_sub(SYNC_WINDOW.as_micros() as u64);
        while self.samples.front().is_some_and(|s| s.t_local < window_start) {
            self.samples.pop_front();
        }

        // Windowed minimum: the lowest-RTT sample still inside the window,
        // not the all-time best — an early lucky sample stops being
        // load-bearing forever once it ages out.
        let best = self.samples.iter().min_by_key(|s| s.rtt).expect("just pushed one");
        let best_raw_offset = best.raw_offset;
        let best_t_local = best.t_local;
        let best_rtt = best.rtt;

        // Linear drift-rate estimate (least-squares slope of raw_offset vs.
        // t_local across the window) projects the chosen sample's offset
        // forward to `t_local` instead of using it stale.
        let drift = self.drift_rate();
        let target = best_raw_offset + (drift * (t_local as f64 - best_t_local as f64)) as i64;

        if !self.synced {
            // Bootstrap: nothing to slew from yet, publish directly.
            self.offset = target;
            self.synced = true;
        } else {
            // Slew toward the target instead of stepping to it.
            let elapsed = t_local.saturating_sub(self.last_local) as f64;
            let max_step = ((elapsed * MAX_SLEW_PPM / 1_000_000.0) as i64).max(1);
            self.offset += (target - self.offset).clamp(-max_step, max_step);
        }
        self.rtt = best_rtt;
        self.last_local = t_local;
    }

    /// Least-squares slope of `raw_offset` against `t_local` across the
    /// current window — an estimate of clock drift in offset-µs per local-µs.
    fn drift_rate(&self) -> f64 {
        if self.samples.len() < 2 {
            return 0.0;
        }
        let base_t = self.samples.front().unwrap().t_local as f64;
        let n = self.samples.len() as f64;
        let (mut sum_x, mut sum_y, mut sum_xy, mut sum_xx) = (0.0, 0.0, 0.0, 0.0);
        for s in &self.samples {
            let x = s.t_local as f64 - base_t;
            let y = s.raw_offset as f64;
            sum_x += x;
            sum_y += y;
            sum_xy += x * y;
            sum_xx += x * x;
        }
        let denom = n * sum_xx - sum_x * sum_x;
        if denom.abs() < f64::EPSILON { 0.0 } else { (n * sum_xy - sum_x * sum_y) / denom }
    }

    pub(crate) fn offset(&self) -> Option<i64> {
        self.synced.then_some(self.offset)
    }

    pub(crate) fn rtt(&self) -> Option<u64> {
        self.synced.then_some(self.rtt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins that clock sync tracks drift instead of locking onto the
    /// all-time-best RTT sample. Simulates an hour-long session (360
    /// re-check pings at the real `SYNC_INTERVAL` cadence) under a steady 50
    /// ppm clock skew — worth 180 ms of drift per hour — plus one early,
    /// unusually good RTT sample. Under a "keep the lowest RTT ever seen"
    /// rule that first sample would pin the offset for the rest of the
    /// session; the windowed-minimum + drift-rate estimate must instead
    /// keep tracking the drift once that sample ages out of the window.
    #[test]
    fn windowed_minimum_tracks_drift_past_an_early_lucky_sample() {
        let mut sync = ClockSync::new();
        const DRIFT_PPM: f64 = 50.0 / 1_000_000.0; // 50 ppm worst-case skew
        const BASE_OFFSET: i64 = 1_000_000; // 1 s baseline local->server offset
        let true_offset = |t_local: u64| BASE_OFFSET + (DRIFT_PPM * t_local as f64) as i64;

        // An early, unusually good RTT sample — the kind that pins the
        // all-time-best rule forever.
        let lucky_rtt = 2_000u64; // 2 ms
        let t_server0 = (true_offset(0) - (lucky_rtt / 2) as i64) as u64;
        sync.on_pong(0, t_server0, lucky_rtt);
        assert_eq!(sync.offset(), Some(BASE_OFFSET), "bootstrap sample must publish directly");

        // An hour of steady 10 s re-check pings, all with an ordinary RTT —
        // much worse than the lucky first sample.
        let normal_rtt = 20_000u64; // 20 ms
        let mut t_local = 0u64;
        for _ in 0..360 {
            t_local += SYNC_INTERVAL.as_micros() as u64;
            let t_server = (t_local as i64 + true_offset(t_local) - (normal_rtt / 2) as i64) as u64;
            sync.on_pong(t_local, t_server, normal_rtt);
        }

        let final_offset = sync.offset().expect("synced");
        let expected = true_offset(t_local);
        // An all-time-best rule would leave this at BASE_OFFSET, 180 ms away
        // from `expected` — this assertion fails under that rule.
        assert!(
            (final_offset - BASE_OFFSET).abs() > 150_000,
            "offset never moved off the early lucky sample: {final_offset} (started at {BASE_OFFSET})"
        );
        assert!(
            (final_offset - expected).abs() < 20_000,
            "offset {final_offset} did not track true drift {expected} within 20 ms"
        );
    }

    /// A new sync target must be approached gradually, not stepped straight
    /// to it, so a reader mid-correction never observes a jump.
    #[test]
    fn offset_corrections_are_slewed_not_stepped() {
        let mut sync = ClockSync::new();
        sync.on_pong(0, 0, 0); // bootstrap: raw_offset = 0
        assert_eq!(sync.offset(), Some(0));

        // One second later, a sample with a much better RTT implies a
        // wildly different offset — becomes the new window minimum/target.
        let t_local = 1_000_000u64; // 1 s later
        let big_offset = 500_000i64; // 500 ms away from the current estimate
        let rtt = 100u64;
        let t_server = (big_offset + t_local as i64 - (rtt / 2) as i64) as u64;
        sync.on_pong(t_local, t_server, rtt);

        let max_step = (1_000_000.0 * MAX_SLEW_PPM / 1_000_000.0) as i64; // elapsed(1s) * ppm rate
        let offset = sync.offset().unwrap();
        assert!(offset > 0, "offset should move toward the new target, not stay put");
        assert!(
            offset <= max_step,
            "offset jumped to {offset} µs in one update, past the {max_step} µs slew cap — a step, not a slew"
        );
        assert!(offset < big_offset, "offset must not step straight to the new target");
    }
}
