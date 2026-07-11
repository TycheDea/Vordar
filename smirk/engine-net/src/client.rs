// NetClient — connecting side. Mirrors NetServer's thread/channel layout and
// owns the clock-sync state machine.

use crate::common::{
    client_crypto, decode_ctrl, encode_ctrl, read_frame_out, write_frame, Ctrl, TAG_APP, TAG_CTRL,
};
use crate::metrics::NetMetrics;
use crate::NetError;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub enum ClientEvent {
    Connected,
    Disconnected,
    Message(Vec<u8>),
    /// The server rejected the handshake with a reason (e.g. version
    /// mismatch) instead of just closing the connection silently (networking
    /// audit 2026-07-11, finding 16). `Disconnected` still follows.
    Rejected(String),
}

/// Initial sync burst: enough samples to find a low-RTT one fast.
const SYNC_BURST_PINGS: u32 = 8;
const SYNC_BURST_INTERVAL: Duration = Duration::from_millis(100);
/// Steady-state re-check (DESIGN.md §3: "re-checked occasionally").
const SYNC_INTERVAL: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Sliding window over which the lowest-RTT sample is treated as the sync
/// anchor, instead of the connection's all-time best (networking audit
/// 2026-07-11, finding 6). Once an early low-RTT sample ages out of this
/// window, later samples resume driving the offset instead of one lucky
/// sample pinning it for the rest of the session.
const SYNC_WINDOW: Duration = Duration::from_secs(90);
/// Maximum rate, in parts-per-million of elapsed local time, at which the
/// published offset may move toward a new target. A correction is always a
/// slew, never a step, so a reader mid-correction (telegraph countdowns,
/// intent deadlines) never observes a jump.
const MAX_SLEW_PPM: f64 = 2_000.0;

/// Both-direction network conditioner knobs (networking audit 2026-07-11,
/// finding 17): before this, the only impairment was a fixed symmetric
/// latency plus receive-side (server→client) datagram loss — no
/// client→server loss, no jitter/reorder, no simulated clock skew. Testing
/// only; every field defaults to "no impairment".
#[derive(Clone, Copy, Debug, Default)]
pub struct Impairment {
    /// Simulated round-trip time; each direction is delayed `rtt / 2`.
    pub rtt: Duration,
    /// Probability (0.0–1.0) that a server→client datagram is dropped below
    /// QUIC, exercising real retransmission (see `impair.rs`).
    pub downstream_loss: f32,
    /// Probability (0.0–1.0) that a client→server datagram is dropped below
    /// QUIC — the direction that used to be untestable entirely.
    pub upstream_loss: f32,
    /// Extra, per-frame random delay drawn uniformly from `[0, jitter]` and
    /// added on top of the fixed one-way latency, in both directions. Unlike
    /// the fixed latency, jitter can reorder frames relative to each other
    /// (see `delay_reorder`).
    pub jitter: Duration,
    /// Simulated client clock drift, in parts-per-million of real elapsed
    /// time. Feeds every "local now" this client reports or times against
    /// (`Ping.t_client`, `local_micros()`), so `ClockSync`'s drift-rate
    /// estimate (finding 6) has something real to track under a live
    /// connection instead of only in `ClockSync`'s own unit tests.
    pub clock_skew_ppm: f64,
}

impl Impairment {
    /// Fixed symmetric latency only — no loss, jitter, or skew.
    pub fn latency(rtt: Duration) -> Self {
        Self { rtt, ..Default::default() }
    }

    /// Named WAN profiles (finding 17, path step 5): representative combined
    /// latency/jitter/loss/skew shapes so client-feel and clock-sync claims
    /// have a headless test at recognizable real-world conditions, not just
    /// hand-picked individual numbers.
    pub fn wifi() -> Self {
        Self {
            rtt: Duration::from_millis(20),
            downstream_loss: 0.001,
            upstream_loss: 0.001,
            jitter: Duration::from_millis(5),
            clock_skew_ppm: 5.0,
        }
    }

    pub fn four_g() -> Self {
        Self {
            rtt: Duration::from_millis(70),
            downstream_loss: 0.01,
            upstream_loss: 0.01,
            jitter: Duration::from_millis(25),
            clock_skew_ppm: 20.0,
        }
    }

    pub fn satellite() -> Self {
        Self {
            rtt: Duration::from_millis(600),
            downstream_loss: 0.02,
            upstream_loss: 0.02,
            jitter: Duration::from_millis(40),
            clock_skew_ppm: 50.0,
        }
    }
}

/// Local-clock scaling for the `clock_skew_ppm` harness: a real client's
/// crystal doesn't tick at exactly the server's rate, by tens of ppm over a
/// long session (finding 6's evidence). Scaling real elapsed time by
/// `1 + skew_ppm/1e6` gives clock-sync a genuine, growing offset to correct
/// for instead of a step outside the simulation entirely.
fn skewed_micros(elapsed: Duration, skew_ppm: f64) -> u64 {
    if skew_ppm == 0.0 {
        return elapsed.as_micros() as u64;
    }
    (elapsed.as_micros() as f64 * (1.0 + skew_ppm / 1_000_000.0)).max(0.0) as u64
}

/// Deterministic per-connection jitter source (same LCG technique as
/// `impair.rs`): draws an extra delay in `[0, max]` for each frame passing
/// through `delay_reorder`.
struct Jitter {
    rng: u64,
    max: Duration,
}

impl Jitter {
    /// A distinct seed per pipeline (writer app frames, writer ctrl frames,
    /// reader) keeps each independently deterministic without sharing a
    /// mutex-guarded RNG across tasks.
    fn with_seed(max: Duration, seed: u64) -> Self {
        Self { rng: seed, max }
    }

    fn sample(&mut self) -> Duration {
        if self.max.is_zero() {
            return Duration::ZERO;
        }
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let frac = (self.rng >> 40) as f64 / (1u64 << 24) as f64; // [0, 1)
        self.max.mul_f64(frac)
    }
}

/// One item in `delay_reorder`'s pending set: ordered by delivery deadline
/// only (ties broken by arrival order) so the payload itself need not be `Ord`.
struct Pending<T> {
    at: tokio::time::Instant,
    seq: u64,
    item: T,
}

impl<T> PartialEq for Pending<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.at, self.seq) == (other.at, other.seq)
    }
}
impl<T> Eq for Pending<T> {}
impl<T> PartialOrd for Pending<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Pending<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed: `BinaryHeap` is a max-heap, but delivery must pop the
        // *earliest* deadline first.
        (other.at, other.seq).cmp(&(self.at, self.seq))
    }
}

/// Delay pipeline stage for latency/jitter simulation (finding 17): items
/// arrive tagged with their intended delivery instant and are released in
/// *deadline* order, not enqueue order. That distinction is what makes jitter
/// able to reorder frames — under a plain delayed FIFO channel (the pre-
/// finding-17 behavior), a later item drawing a smaller delay than an
/// earlier one still waits behind it; here it legitimately overtakes, the
/// way real jitter reorders packets on the wire.
async fn delay_reorder<T: Send + 'static>(
    mut rx: UnboundedReceiver<(tokio::time::Instant, T)>,
    tx: UnboundedSender<T>,
) {
    let mut pending: BinaryHeap<Pending<T>> = BinaryHeap::new();
    let mut next_seq = 0u64;
    loop {
        match pending.peek() {
            Some(next) => {
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep_until(next.at) => {
                        if let Some(p) = pending.pop() {
                            if tx.send(p.item).is_err() { return; }
                        }
                    }
                    maybe = rx.recv() => match maybe {
                        Some((at, item)) => { pending.push(Pending { at, seq: next_seq, item }); next_seq += 1; }
                        None => break,
                    },
                }
            }
            None => match rx.recv().await {
                Some((at, item)) => { pending.push(Pending { at, seq: next_seq, item }); next_seq += 1; }
                None => return,
            },
        }
    }
    // Channel closed: drain whatever is left, still in deadline order.
    while let Some(p) = pending.pop() {
        tokio::time::sleep_until(p.at).await;
        let _ = tx.send(p.item);
    }
}

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
/// live connection (networking audit 2026-07-11, finding 6).
struct ClockSync {
    samples: VecDeque<ClockSample>,
    offset: i64,
    rtt: u64,
    synced: bool,
    last_local: u64,
}

impl ClockSync {
    fn new() -> Self {
        Self { samples: VecDeque::new(), offset: 0, rtt: 0, synced: false, last_local: 0 }
    }

    /// Feed one Pong sample. `t_local` is the local micros at which it was
    /// received/measured; `t_server` and `rtt` come straight off the wire.
    fn on_pong(&mut self, t_local: u64, t_server: u64, rtt: u64) {
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

    fn offset(&self) -> Option<i64> {
        self.synced.then_some(self.offset)
    }

    fn rtt(&self) -> Option<u64> {
        self.synced.then_some(self.rtt)
    }
}

pub struct NetClient {
    events: UnboundedReceiver<ClientEvent>,
    out: UnboundedSender<Vec<u8>>,
    clock: Arc<Mutex<ClockSync>>,
    epoch: Instant,
    /// Simulated clock drift applied to every reading of `epoch` (finding 17's
    /// clock-skew harness) — zero for every non-impaired connection.
    clock_skew_ppm: f64,
    metrics: Arc<NetMetrics>,
}

impl NetClient {
    /// Connect to a server and start the network thread. Returns once the
    /// connection attempt is underway; `ClientEvent::Connected` confirms it.
    pub fn connect(addr: SocketAddr, version: u8) -> Result<Self, NetError> {
        Self::connect_impaired(addr, version, Impairment::default())
    }

    /// Like [`connect`](Self::connect), but artificially delays every frame
    /// after the handshake by `simulated_rtt / 2` in each direction — a testing
    /// knob for latency-sensitive features (prediction, lag compensation).
    /// Clock-sync pings are delayed too, so the measured RTT includes the
    /// simulated latency while the synced offset stays correct (the added
    /// delay is symmetric).
    pub fn connect_with_latency(
        addr: SocketAddr,
        version: u8,
        simulated_rtt: Duration,
    ) -> Result<Self, NetError> {
        Self::connect_impaired(addr, version, Impairment::latency(simulated_rtt))
    }

    /// Full network conditioner (finding 17): latency, both-direction datagram
    /// loss, jitter/reorder, and simulated clock skew — see [`Impairment`].
    /// Testing only.
    pub fn connect_impaired(addr: SocketAddr, version: u8, impairment: Impairment) -> Result<Self, NetError> {
        let one_way = impairment.rtt / 2;
        let epoch = Instant::now();
        let (event_tx, event_rx) = unbounded_channel();
        let (out_tx, out_rx) = unbounded_channel();
        let clock = Arc::new(Mutex::new(ClockSync::new()));
        let metrics = NetMetrics::new();

        let thread_clock = clock.clone();
        let thread_metrics = metrics.clone();
        std::thread::Builder::new()
            .name("engine-net-client".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => { log::error!("net: tokio runtime failed: {e}"); return; }
                };
                rt.block_on(async move {
                    match client_main(
                        addr, version, epoch, event_tx.clone(), out_rx, thread_clock, one_way, impairment,
                        thread_metrics,
                    ).await {
                        Ok(()) => log::info!("net: connection closed"),
                        Err(e) => log::warn!("net: connection ended: {e}"),
                    }
                    let _ = event_tx.send(ClientEvent::Disconnected);
                });
            })
            .map_err(NetError::Io)?;

        Ok(Self {
            events: event_rx,
            out: out_tx,
            clock,
            epoch,
            clock_skew_ppm: impairment.clock_skew_ppm,
            metrics,
        })
    }

    /// Drain all pending network events. Call once per Input tick.
    pub fn poll(&mut self) -> Vec<ClientEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
        out
    }

    pub fn send(&self, data: Vec<u8>) {
        let _ = self.out.send(data);
    }

    /// Microseconds since this client started — the local monotonic clock
    /// (skewed by `clock_skew_ppm` under the finding-17 harness).
    pub fn local_micros(&self) -> u64 {
        skewed_micros(self.epoch.elapsed(), self.clock_skew_ppm)
    }

    /// local → server clock offset, once at least one sync sample landed.
    pub fn server_offset_micros(&self) -> Option<i64> {
        self.clock.lock().unwrap().offset()
    }

    /// Estimated current server time. The anchor for intent timestamps and
    /// telegraph countdowns.
    pub fn server_now_micros(&self) -> Option<u64> {
        self.server_offset_micros()
            .map(|off| (self.local_micros() as i64 + off).max(0) as u64)
    }

    /// RTT of the best (lowest) clock-sync sample so far.
    pub fn rtt_micros(&self) -> Option<u64> {
        self.clock.lock().unwrap().rtt()
    }

    /// Frame/byte counters for this connection (observability only).
    pub fn metrics(&self) -> Arc<NetMetrics> {
        self.metrics.clone()
    }
}

#[allow(clippy::too_many_arguments)]
async fn client_main(
    addr: SocketAddr,
    version: u8,
    epoch: Instant,
    events: UnboundedSender<ClientEvent>,
    mut out_rx: UnboundedReceiver<Vec<u8>>,
    clock: Arc<Mutex<ClockSync>>,
    one_way: Duration,
    impairment: Impairment,
    metrics: Arc<NetMetrics>,
) -> Result<(), NetError> {
    let jitter = impairment.jitter;
    let skew_ppm = impairment.clock_skew_ppm;
    let bind: SocketAddr = if addr.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
    let mut endpoint = if impairment.downstream_loss > 0.0 || impairment.upstream_loss > 0.0 {
        crate::impair::lossy_client_endpoint(bind, impairment.downstream_loss, impairment.upstream_loss)?
    } else {
        quinn::Endpoint::client(bind)?
    };
    let mut config = client_crypto()?;
    // Keep idle connections alive — a player standing still must stay connected.
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(5)));
    config.transport_config(Arc::new(transport));
    endpoint.set_default_client_config(config);

    let connection = endpoint
        .connect(addr, "localhost")
        .map_err(|e| NetError::Handshake(e.to_string()))?
        .await
        .map_err(|e| NetError::Handshake(e.to_string()))?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| NetError::Handshake(e.to_string()))?;

    // Handshake: Hello → HelloAck (bounded wait).
    write_frame(&mut send, TAG_CTRL, &encode_ctrl(&Ctrl::Hello { version }))
        .await
        .map_err(|e| NetError::Handshake(e.to_string()))?;
    let ack = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame_out(&mut recv))
        .await
        .map_err(|_| NetError::Handshake("timed out waiting for HelloAck".into()))??;
    match (ack.0, decode_ctrl(&ack.1)) {
        (TAG_CTRL, Some(Ctrl::HelloAck)) => {}
        (TAG_CTRL, Some(Ctrl::Reject { reason })) => {
            let _ = events.send(ClientEvent::Rejected(reason.clone()));
            return Err(NetError::Handshake(format!("rejected by server: {reason}")));
        }
        _ => return Err(NetError::Handshake("expected HelloAck".into())),
    }
    let _ = events.send(ClientEvent::Connected);
    log::info!("net: connected to {addr}");

    // Writer task — merges app sends and clock pings; sole owner of the
    // stream. Frames carry a delivery deadline (enqueue time + one_way, plus
    // a jitter draw); `delay_reorder` releases them in deadline order rather
    // than enqueue order, so under jitter a frame can legitimately overtake
    // one queued ahead of it (finding 17 — previously a strictly monotonic
    // FIFO delay, which could never reorder anything).
    let (write_tx, write_rx) = unbounded_channel::<(tokio::time::Instant, (u8, Vec<u8>))>();
    let (ordered_tx, mut ordered_rx) = unbounded_channel::<(u8, Vec<u8>)>();
    tokio::spawn(delay_reorder(write_rx, ordered_tx));
    let writer_metrics = metrics.clone();
    let writer = tokio::spawn(async move {
        while let Some((tag, payload)) = ordered_rx.recv().await {
            if write_frame(&mut send, tag, &payload).await.is_err() {
                break;
            }
            writer_metrics.record_frame_out(payload.len());
        }
    });
    let app_tx = write_tx.clone();
    let conn_for_forward = connection.clone();
    let forward = tokio::spawn(async move {
        let mut app_jitter = Jitter::with_seed(jitter, 0xD1B5_4A32_D192_ED03);
        while let Some(data) = out_rx.recv().await {
            let at = tokio::time::Instant::now() + one_way + app_jitter.sample();
            if app_tx.send((at, (TAG_APP, data))).is_err() { break; }
        }
        // The simulation dropped its NetClient — close so the server notices.
        conn_for_forward.close(0u32.into(), b"client closed");
    });

    // Clock-sync pinger: a fast burst, then occasional re-checks.
    let ping_tx = write_tx.clone();
    let pinger = tokio::spawn(async move {
        let mut ping_jitter = Jitter::with_seed(jitter, 0xA5A5_5A5A_1234_5678);
        for _ in 0..SYNC_BURST_PINGS {
            let ping = Ctrl::Ping { t_client: skewed_micros(epoch.elapsed(), skew_ppm) };
            let at = tokio::time::Instant::now() + one_way + ping_jitter.sample();
            if ping_tx.send((at, (TAG_CTRL, encode_ctrl(&ping)))).is_err() { return; }
            tokio::time::sleep(SYNC_BURST_INTERVAL).await;
        }
        loop {
            tokio::time::sleep(SYNC_INTERVAL).await;
            let ping = Ctrl::Ping { t_client: skewed_micros(epoch.elapsed(), skew_ppm) };
            let at = tokio::time::Instant::now() + one_way + ping_jitter.sample();
            if ping_tx.send((at, (TAG_CTRL, encode_ctrl(&ping)))).is_err() { return; }
        }
    });

    // Raw reader stamps each frame on arrival; delay_reorder releases it
    // one_way (+ jitter) later, possibly out of arrival order.
    let (in_tx, in_rx) =
        unbounded_channel::<(tokio::time::Instant, Result<(u8, Vec<u8>), NetError>)>();
    let (ordered_in_tx, mut ordered_in_rx) = unbounded_channel::<Result<(u8, Vec<u8>), NetError>>();
    tokio::spawn(delay_reorder(in_rx, ordered_in_tx));
    let reader_metrics = metrics.clone();
    let reader = tokio::spawn(async move {
        let mut read_jitter = Jitter::with_seed(jitter, 0x1234_5678_9ABC_DEF0);
        loop {
            let frame = read_frame_out(&mut recv).await;
            if let Ok((_, ref payload)) = frame {
                reader_metrics.record_frame_in(payload.len());
            }
            let failed = frame.is_err();
            let at = tokio::time::Instant::now() + one_way + read_jitter.sample();
            if in_tx.send((at, frame)).is_err() || failed {
                break;
            }
        }
    });

    let result = loop {
        let Some(frame) = ordered_in_rx.recv().await else { break Err(NetError::Closed) };
        match frame {
            Ok((TAG_CTRL, payload)) => {
                if let Some(Ctrl::Pong { t_client, t_server }) = decode_ctrl(&payload) {
                    let now = skewed_micros(epoch.elapsed(), skew_ppm);
                    let rtt = now.saturating_sub(t_client);
                    let mut c = clock.lock().unwrap();
                    c.on_pong(now, t_server, rtt);
                    if let (Some(offset), Some(rtt)) = (c.offset(), c.rtt()) {
                        log::debug!("net: clock sync — offset {offset} µs, rtt {rtt} µs (windowed)");
                    }
                }
            }
            Ok((TAG_APP, data)) => {
                let _ = events.send(ClientEvent::Message(data));
            }
            Ok((tag, _)) => break Err(NetError::Handshake(format!("unknown frame tag {tag}"))),
            Err(e) => break Err(e),
        }
    };

    reader.abort();
    pinger.abort();
    forward.abort();
    writer.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for "clock sync locks onto the all-time-best RTT
    /// sample" (networking audit 2026-07-11, finding 6). Simulates an
    /// hour-long session (360 re-check pings at the real `SYNC_INTERVAL`
    /// cadence) under a steady 50 ppm clock skew — the worst case the finding
    /// cites, worth 180 ms of drift per hour — plus one early, unusually good
    /// RTT sample. Under the old "keep the lowest RTT ever seen" rule that
    /// first sample would pin the offset for the rest of the session; the
    /// windowed-minimum + drift-rate estimate must instead keep tracking the
    /// drift once that sample ages out of the window.
    #[test]
    fn windowed_minimum_tracks_drift_past_an_early_lucky_sample() {
        let mut sync = ClockSync::new();
        const DRIFT_PPM: f64 = 50.0 / 1_000_000.0; // 50 ppm, the finding's worst case
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
        // Old (all-time-best) behavior would leave this at BASE_OFFSET, 180 ms
        // away from `expected` — proof the fix actually moved off it.
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
    /// to it, so a reader mid-correction never observes a jump (finding 6).
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
