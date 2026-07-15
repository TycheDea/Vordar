// NetClient — connecting side. Mirrors NetServer's thread/channel layout
// and drives the clock-sync filter (clock.rs) from its ping/pong tasks.

use crate::clock::{ClockSync, SYNC_BURST_INTERVAL, SYNC_BURST_PINGS, SYNC_INTERVAL};
use crate::common::{
    client_crypto, decode_ctrl, decode_datagram, encode_ctrl, encode_datagram, read_frame_out,
    write_frame, Ctrl, TAG_APP, TAG_CTRL,
};
use crate::impair::{delay_reorder, skewed_micros, Impairment, Jitter};
use crate::metrics::NetMetrics;
use crate::NetError;
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub enum ClientEvent {
    Connected,
    Disconnected,
    Message(Vec<u8>),
    /// The server rejected the handshake with a reason (e.g. version
    /// mismatch) instead of just closing the connection silently.
    /// `Disconnected` still follows.
    Rejected(String),
}

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct NetClient {
    events: UnboundedReceiver<ClientEvent>,
    out: UnboundedSender<Vec<u8>>,
    /// Outbound datagram lane — a separate channel from `out` above:
    /// datagrams never touch the reliable stream's writer queue, so a
    /// stalled stream can never delay them.
    out_datagram: UnboundedSender<Vec<u8>>,
    clock: Arc<Mutex<ClockSync>>,
    epoch: Instant,
    /// Simulated clock drift applied to every reading of `epoch` — zero for
    /// every non-impaired connection.
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

    /// Full network conditioner: latency, both-direction datagram loss,
    /// jitter/reorder, and simulated clock skew — see [`Impairment`].
    /// Testing only.
    pub fn connect_impaired(addr: SocketAddr, version: u8, impairment: Impairment) -> Result<Self, NetError> {
        let one_way = impairment.rtt / 2;
        let epoch = Instant::now();
        let (event_tx, event_rx) = unbounded_channel();
        let (out_tx, out_rx) = unbounded_channel();
        let (out_datagram_tx, out_datagram_rx) = unbounded_channel();
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
                        addr, version, epoch, event_tx.clone(), out_rx, out_datagram_rx, thread_clock,
                        one_way, impairment, thread_metrics,
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
            out_datagram: out_datagram_tx,
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

    /// Send `data` to the server via an unreliable QUIC datagram instead of
    /// the reliable ordered stream: a lost datagram is simply gone — no
    /// retransmit, no stream fallback — so callers must only route messages
    /// here that tolerate loss/reorder. Tagged `TAG_APP` so the server's
    /// datagram lane surfaces it as the same `ServerEvent::Message` the
    /// stream uses.
    pub fn send_datagram(&self, data: Vec<u8>) {
        let _ = self.out_datagram.send(data);
    }

    /// Microseconds since this client started — the local monotonic clock
    /// (skewed by `clock_skew_ppm` under the impairment harness).
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
    mut out_rx_datagram: UnboundedReceiver<Vec<u8>>,
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

    // Writer task — merges app sends only; sole owner of the stream. Clock
    // pings ride the datagram lane below so they never queue behind app
    // frames here. Frames carry a delivery deadline (enqueue time + one_way,
    // plus a jitter draw); `delay_reorder` releases them in deadline order
    // rather than enqueue order, so under jitter a frame can legitimately
    // overtake one queued ahead of it.
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

    // Datagram outbound pipeline: outbound datagrams get their own
    // delay_reorder stage (one_way + jitter, a fresh seed) but never touch
    // the stream's writer queue at all — `connection.send_datagram` is
    // fire-and-forget, so a lost datagram is truly gone instead of
    // retransmitted.
    let (dgram_write_tx, dgram_write_rx) = unbounded_channel::<(tokio::time::Instant, (u8, Vec<u8>))>();
    let (dgram_ordered_tx, mut dgram_ordered_rx) = unbounded_channel::<(u8, Vec<u8>)>();
    tokio::spawn(delay_reorder(dgram_write_rx, dgram_ordered_tx));
    let dgram_sender_metrics = metrics.clone();
    let dgram_conn_for_send = connection.clone();
    let dgram_sender = tokio::spawn(async move {
        while let Some((tag, payload)) = dgram_ordered_rx.recv().await {
            let bytes = Bytes::from(encode_datagram(tag, &payload));
            match dgram_conn_for_send.send_datagram(bytes) {
                Ok(()) => dgram_sender_metrics.record_datagram_out(),
                Err(_) => dgram_sender_metrics.record_datagram_send_failure(),
            }
        }
    });
    let dgram_app_tx = dgram_write_tx.clone();
    let dgram_forward = tokio::spawn(async move {
        let mut dgram_jitter = Jitter::with_seed(jitter, 0x5EED_C0DE_1357_9BDF);
        while let Some(data) = out_rx_datagram.recv().await {
            let at = tokio::time::Instant::now() + one_way + dgram_jitter.sample();
            if dgram_app_tx.send((at, (TAG_APP, data))).is_err() { break; }
        }
    });

    // Clock-sync pinger: a fast burst, then occasional re-checks. Pings ride
    // the datagram lane — never `write_tx` — so a retransmitting stream can
    // never inflate an RTT sample with queueing delay that has nothing to do
    // with the path. A lost ping/pong datagram costs one sample; the burst
    // and recheck cadence absorb it.
    let ping_tx = dgram_write_tx.clone();
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

    // Datagram inbound task: stamps arrivals into the SAME in_tx channel the
    // stream reader (below) uses, so the one delay_reorder / ordered_in_rx
    // consumer loop handles both lanes identically — a datagram Ctrl::Pong
    // or app message is indistinguishable from its stream counterpart by
    // the time it reaches that loop. Never sends `Err`: only the stream
    // reader owns connection-teardown signaling.
    let dgram_in_tx = in_tx.clone();
    let dgram_conn_for_recv = connection.clone();
    let dgram_reader_metrics = metrics.clone();
    let dgram_reader = tokio::spawn(async move {
        let mut dgram_read_jitter = Jitter::with_seed(jitter, 0xFEED_BEEF_0BAD_C0DE);
        while let Ok(bytes) = dgram_conn_for_recv.read_datagram().await {
            let Some((tag, payload)) = decode_datagram(&bytes) else { continue };
            dgram_reader_metrics.record_datagram_in();
            let at = tokio::time::Instant::now() + one_way + dgram_read_jitter.sample();
            if dgram_in_tx.send((at, Ok((tag, payload.to_vec())))).is_err() {
                break;
            }
        }
    });

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
    dgram_reader.abort();
    dgram_forward.abort();
    dgram_sender.abort();
    result
}
