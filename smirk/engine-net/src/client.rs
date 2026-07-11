// NetClient — connecting side. Mirrors NetServer's thread/channel layout and
// owns the clock-sync state machine.

use crate::common::{
    client_crypto, decode_ctrl, encode_ctrl, read_frame_out, write_frame, Ctrl, TAG_APP, TAG_CTRL,
};
use crate::NetError;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub enum ClientEvent {
    Connected,
    Disconnected,
    Message(Vec<u8>),
}

/// Clock-sync state shared between the network thread (writer) and the
/// simulation thread (reader). `offset` maps local micros → server micros.
struct Clock {
    offset: AtomicI64,
    rtt: AtomicU64,
    best_rtt: AtomicU64,
    synced: AtomicBool,
}

/// Initial sync burst: enough samples to find a low-RTT one fast.
const SYNC_BURST_PINGS: u32 = 8;
const SYNC_BURST_INTERVAL: Duration = Duration::from_millis(100);
/// Steady-state re-check (DESIGN.md §3: "re-checked occasionally").
const SYNC_INTERVAL: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct NetClient {
    events: UnboundedReceiver<ClientEvent>,
    out: UnboundedSender<Vec<u8>>,
    clock: Arc<Clock>,
    epoch: Instant,
}

impl NetClient {
    /// Connect to a server and start the network thread. Returns once the
    /// connection attempt is underway; `ClientEvent::Connected` confirms it.
    pub fn connect(addr: SocketAddr, version: u8) -> Result<Self, NetError> {
        Self::connect_with_latency(addr, version, Duration::ZERO)
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
        Self::connect_impaired(addr, version, simulated_rtt, 0.0)
    }

    /// Like [`connect_with_latency`](Self::connect_with_latency), but also
    /// drops received datagrams below QUIC with probability `loss` — dropped
    /// stream frames stall until QUIC retransmits them, so head-of-line
    /// behavior under loss is the real thing (see `impair.rs`). Testing only.
    pub fn connect_impaired(
        addr: SocketAddr,
        version: u8,
        simulated_rtt: Duration,
        loss: f32,
    ) -> Result<Self, NetError> {
        let one_way = simulated_rtt / 2;
        let epoch = Instant::now();
        let (event_tx, event_rx) = unbounded_channel();
        let (out_tx, out_rx) = unbounded_channel();
        let clock = Arc::new(Clock {
            offset: AtomicI64::new(0),
            rtt: AtomicU64::new(0),
            best_rtt: AtomicU64::new(u64::MAX),
            synced: AtomicBool::new(false),
        });

        let thread_clock = clock.clone();
        std::thread::Builder::new()
            .name("engine-net-client".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => { log::error!("net: tokio runtime failed: {e}"); return; }
                };
                rt.block_on(async move {
                    match client_main(addr, version, epoch, event_tx.clone(), out_rx, thread_clock, one_way, loss).await {
                        Ok(()) => log::info!("net: connection closed"),
                        Err(e) => log::warn!("net: connection ended: {e}"),
                    }
                    let _ = event_tx.send(ClientEvent::Disconnected);
                });
            })
            .map_err(NetError::Io)?;

        Ok(Self { events: event_rx, out: out_tx, clock, epoch })
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

    /// Microseconds since this client started — the local monotonic clock.
    pub fn local_micros(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }

    /// local → server clock offset, once at least one sync sample landed.
    pub fn server_offset_micros(&self) -> Option<i64> {
        self.clock.synced.load(Ordering::Acquire).then(|| self.clock.offset.load(Ordering::Acquire))
    }

    /// Estimated current server time. The anchor for intent timestamps and
    /// telegraph countdowns.
    pub fn server_now_micros(&self) -> Option<u64> {
        self.server_offset_micros()
            .map(|off| (self.local_micros() as i64 + off).max(0) as u64)
    }

    /// RTT of the best (lowest) clock-sync sample so far.
    pub fn rtt_micros(&self) -> Option<u64> {
        self.clock.synced.load(Ordering::Acquire).then(|| self.clock.rtt.load(Ordering::Acquire))
    }
}

#[allow(clippy::too_many_arguments)]
async fn client_main(
    addr: SocketAddr,
    version: u8,
    epoch: Instant,
    events: UnboundedSender<ClientEvent>,
    mut out_rx: UnboundedReceiver<Vec<u8>>,
    clock: Arc<Clock>,
    one_way: Duration,
    loss: f32,
) -> Result<(), NetError> {
    let bind: SocketAddr = if addr.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
    let mut endpoint = if loss > 0.0 {
        crate::impair::lossy_client_endpoint(bind, loss)?
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
        _ => return Err(NetError::Handshake("expected HelloAck".into())),
    }
    let _ = events.send(ClientEvent::Connected);
    log::info!("net: connected to {addr}");

    // Writer task — merges app sends and clock pings; sole owner of the stream.
    // Frames carry a delivery deadline (enqueue time + one_way): deadlines are
    // monotonic, so FIFO delivery simulates latency without throttling
    // throughput (sleeping per frame inside a loop would compound the delay).
    let (write_tx, mut write_rx) = unbounded_channel::<(tokio::time::Instant, u8, Vec<u8>)>();
    let writer = tokio::spawn(async move {
        while let Some((at, tag, payload)) = write_rx.recv().await {
            tokio::time::sleep_until(at).await;
            if write_frame(&mut send, tag, &payload).await.is_err() {
                break;
            }
        }
    });
    let app_tx = write_tx.clone();
    let conn_for_forward = connection.clone();
    let forward = tokio::spawn(async move {
        while let Some(data) = out_rx.recv().await {
            if app_tx.send((tokio::time::Instant::now() + one_way, TAG_APP, data)).is_err() { break; }
        }
        // The simulation dropped its NetClient — close so the server notices.
        conn_for_forward.close(0u32.into(), b"client closed");
    });

    // Clock-sync pinger: a fast burst, then occasional re-checks.
    let ping_tx = write_tx.clone();
    let pinger = tokio::spawn(async move {
        for _ in 0..SYNC_BURST_PINGS {
            let ping = Ctrl::Ping { t_client: epoch.elapsed().as_micros() as u64 };
            if ping_tx.send((tokio::time::Instant::now() + one_way, TAG_CTRL, encode_ctrl(&ping))).is_err() { return; }
            tokio::time::sleep(SYNC_BURST_INTERVAL).await;
        }
        loop {
            tokio::time::sleep(SYNC_INTERVAL).await;
            let ping = Ctrl::Ping { t_client: epoch.elapsed().as_micros() as u64 };
            if ping_tx.send((tokio::time::Instant::now() + one_way, TAG_CTRL, encode_ctrl(&ping))).is_err() { return; }
        }
    });

    // Raw reader stamps each frame on arrival; processing happens one_way later.
    let (in_tx, mut in_rx) =
        unbounded_channel::<(tokio::time::Instant, Result<(u8, Vec<u8>), NetError>)>();
    let reader = tokio::spawn(async move {
        loop {
            let frame = read_frame_out(&mut recv).await;
            let failed = frame.is_err();
            if in_tx.send((tokio::time::Instant::now() + one_way, frame)).is_err() || failed {
                break;
            }
        }
    });

    let result = loop {
        let Some((at, frame)) = in_rx.recv().await else { break Err(NetError::Closed) };
        tokio::time::sleep_until(at).await;
        match frame {
            Ok((TAG_CTRL, payload)) => {
                if let Some(Ctrl::Pong { t_client, t_server }) = decode_ctrl(&payload) {
                    let now = epoch.elapsed().as_micros() as u64;
                    let rtt = now.saturating_sub(t_client);
                    // Keep the lowest-RTT sample — most symmetric, least error.
                    if rtt <= clock.best_rtt.load(Ordering::Acquire) {
                        let offset = (t_server as i64 + (rtt / 2) as i64) - now as i64;
                        clock.offset.store(offset, Ordering::Release);
                        clock.rtt.store(rtt, Ordering::Release);
                        clock.best_rtt.store(rtt, Ordering::Release);
                        clock.synced.store(true, Ordering::Release);
                        log::debug!("net: clock sync — offset {offset} µs, rtt {rtt} µs");
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
