// NetServer — listening side. One tokio runtime on a dedicated thread; the
// simulation polls events and pushes sends through unbounded channels.

use crate::common::{
    decode_ctrl, decode_datagram, encode_ctrl, encode_datagram, read_frame_in, server_crypto,
    write_frame, Ctrl, TAG_APP, TAG_CTRL,
};
use crate::metrics::NetMetrics;
use crate::NetError;
use bytes::Bytes;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub type ConnId = u64;

pub enum ServerEvent {
    Connected(ConnId),
    Disconnected(ConnId),
    /// `recv_micros` is the server clock (see [`NetServer::now_micros`]) at the
    /// moment the network thread received the frame — the arrival-deadline anchor.
    Message { conn: ConnId, data: Vec<u8>, recv_micros: u64 },
}

/// Payloads travel as `Arc<Vec<u8>>` so a broadcast is one encode plus a
/// refcount bump per connection instead of a full clone per connection.
enum Outgoing {
    To(ConnId, Arc<Vec<u8>>),
    All(Arc<Vec<u8>>),
    Kick(ConnId),
    /// An unreliable datagram send — already tagged (`encode_datagram`
    /// applied in `NetServer::send_datagram`), so the router only has to
    /// hand the bytes to quinn.
    Datagram(ConnId, Vec<u8>),
}

/// Per-connection writer queue + the quinn handle (for server-side close) +
/// the queue's current backlog depth (items enqueued, not yet dequeued by the
/// writer task) — the budget `WRITER_QUEUE_CAP` is enforced against.
type ConnMap = Arc<
    Mutex<HashMap<ConnId, (UnboundedSender<(u8, Arc<Vec<u8>>)>, quinn::Connection, Arc<AtomicU64>)>>,
>;
/// Per-connection RTT: the latest smoothed sample plus an EWMA mean/variance
/// baseline folded from the same samples, owned directly by that
/// connection's reader task — writes never take the map lock. The map
/// itself is only touched at connect (insert) and disconnect (remove), so
/// the sim thread's reads essentially never contend with the network
/// thread.
type RttMap = Arc<Mutex<HashMap<ConnId, Arc<RttHandle>>>>;

/// Exponentially-weighted mean/variance of a connection's RTT samples — the
/// baseline a mechanic-window spike is measured against (DESIGN.md §3).
#[derive(Clone, Copy)]
struct RttEstimator {
    mean: f64,
    var: f64,
    warmed: bool,
}

impl RttEstimator {
    /// EWMA smoothing factor: recent samples dominate the baseline within a
    /// few seconds of RTT updates, without one outlier resetting it outright.
    const ALPHA: f64 = 0.1;

    fn new() -> Self {
        Self { mean: 0.0, var: 0.0, warmed: false }
    }

    /// Folds one RTT sample (micros) into the running mean/variance. The
    /// first sample seeds the mean with zero variance rather than averaging
    /// against an arbitrary starting point.
    fn update(&mut self, sample_micros: f64) {
        if !self.warmed {
            self.mean = sample_micros;
            self.warmed = true;
            return;
        }
        let delta = sample_micros - self.mean;
        self.mean += Self::ALPHA * delta;
        self.var = (1.0 - Self::ALPHA) * (self.var + Self::ALPHA * delta * delta);
    }
}

/// Owns one connection's live RTT reading and its EWMA baseline together, so
/// every sample updates both under a single call.
struct RttHandle {
    current: AtomicU64,
    stats: Mutex<RttEstimator>,
}

impl RttHandle {
    fn new() -> Self {
        Self { current: AtomicU64::new(0), stats: Mutex::new(RttEstimator::new()) }
    }

    fn record(&self, sample_micros: u64) {
        self.current.store(sample_micros, Ordering::Relaxed);
        self.stats.lock().unwrap().update(sample_micros as f64);
    }
}
/// Source IP per live connection: populated once at connect, removed once
/// at disconnect — the exact same lifecycle as `RttMap` above, so
/// `NetServer::peer_ip` can attribute a failed login to an address without
/// the sim tracking connection metadata of its own.
type PeerMap = Arc<Mutex<HashMap<ConnId, IpAddr>>>;
/// Live connection count per source IP — reserved the instant a connection
/// is accepted (before its handshake even completes) and released when it
/// ends, so `NetServer::MAX_CONNECTIONS_PER_IP` holds even against a burst of
/// near-simultaneous connection attempts from one address.
type IpCounts = Arc<Mutex<HashMap<IpAddr, usize>>>;

pub struct NetServer {
    events: UnboundedReceiver<ServerEvent>,
    out: UnboundedSender<Outgoing>,
    epoch: Instant,
    local_addr: SocketAddr,
    rtts: RttMap,
    peers: PeerMap,
    metrics: Arc<NetMetrics>,
    /// Signals `server_main`'s accept loop to stop; taken and sent on Drop.
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// The network thread's handle; taken and joined on Drop, bounded by
    /// `server_main`'s `wait_idle` timeout so Drop cannot hang.
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Bind-time connection-cap configuration. `NetServer::bind`'s hard-coded
/// caps and this struct's `Default` are the same hostile-client values —
/// production keeps them untouched. An embedder that models a real crowd of
/// distinct clients from one source IP (a soak harness, a stress CLI, a LAN
/// deployment) states that trust model explicitly through
/// `NetServer::bind_with_limits` instead of the transport weakening its own
/// default.
#[derive(Clone, Copy, Debug)]
pub struct NetLimits {
    pub max_connections: usize,
    pub max_connections_per_ip: usize,
}

impl Default for NetLimits {
    fn default() -> Self {
        Self {
            max_connections: NetServer::MAX_CONNECTIONS,
            max_connections_per_ip: NetServer::MAX_CONNECTIONS_PER_IP,
        }
    }
}

/// Enqueue a frame onto a connection's writer queue, tracking backlog depth.
/// A stalled reader (a client that stops draining its stream) otherwise grows
/// this queue without bound; once depth crosses `NetServer::WRITER_QUEUE_CAP`
/// the connection is kicked instead of buffering forever.
fn enqueue(
    tx: &UnboundedSender<(u8, Arc<Vec<u8>>)>,
    depth: &AtomicU64,
    metrics: &NetMetrics,
    connection: &quinn::Connection,
    tag: u8,
    payload: Arc<Vec<u8>>,
) {
    if tx.send((tag, payload)).is_ok() {
        let d = depth.fetch_add(1, Ordering::Relaxed) + 1;
        metrics.writer_queue_depth.fetch_add(1, Ordering::Relaxed);
        if d as usize > NetServer::WRITER_QUEUE_CAP {
            connection.close(0u32.into(), b"writer queue backlog");
        }
    }
}

impl NetServer {
    /// Bind a QUIC endpoint and start the network thread. `version` is the
    /// application protocol version checked during the handshake. Uses the
    /// hostile-client default connection caps — see [`NetServer::bind_with_limits`]
    /// to override them.
    pub fn bind(addr: SocketAddr, version: u8) -> Result<Self, NetError> {
        Self::bind_with_limits(addr, version, NetLimits::default())
    }

    /// Like [`NetServer::bind`], but with explicit connection-cap
    /// configuration instead of the transport's hostile-client defaults —
    /// e.g. a soak harness modeling many distinct clients from one source IP
    /// raises `max_connections_per_ip` to its bot count here rather than the
    /// transport weakening its own default.
    pub fn bind_with_limits(addr: SocketAddr, version: u8, limits: NetLimits) -> Result<Self, NetError> {
        let epoch = Instant::now();
        let (event_tx, event_rx) = unbounded_channel();
        let (out_tx, out_rx) = unbounded_channel();
        let conns: ConnMap = Arc::new(Mutex::new(HashMap::new()));
        let rtts: RttMap = Arc::new(Mutex::new(HashMap::new()));
        let peers: PeerMap = Arc::new(Mutex::new(HashMap::new()));
        let metrics = NetMetrics::new();

        // Report bind success/failure synchronously before the thread detaches.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<SocketAddr, NetError>>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let thread_conns = conns.clone();
        let thread_rtts = rtts.clone();
        let thread_peers = peers.clone();
        let thread_metrics = metrics.clone();
        let thread = std::thread::Builder::new()
            .name("engine-net-server".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => { let _ = ready_tx.send(Err(NetError::Io(e))); return; }
                };
                rt.block_on(server_main(
                    addr, version, epoch, event_tx, out_rx, thread_conns, thread_rtts, thread_peers,
                    thread_metrics, ready_tx, limits, shutdown_rx,
                ));
            })
            .map_err(NetError::Io)?;

        let local_addr = ready_rx
            .recv()
            .map_err(|_| NetError::Handshake("network thread died during bind".into()))??;

        Ok(Self {
            events: event_rx,
            out: out_tx,
            epoch,
            local_addr,
            rtts,
            peers,
            metrics,
            shutdown_tx: Some(shutdown_tx),
            thread: Some(thread),
        })
    }

    /// Drain all pending network events. Call once per Input tick.
    pub fn poll(&mut self) -> Vec<ServerEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
        out
    }

    pub fn send(&self, conn: ConnId, data: Vec<u8>) {
        let _ = self.out.send(Outgoing::To(conn, Arc::new(data)));
    }

    pub fn broadcast(&self, data: Vec<u8>) {
        let _ = self.out.send(Outgoing::All(Arc::new(data)));
    }

    /// Send `data` to one connection via an unreliable QUIC datagram instead
    /// of the reliable ordered stream: a lost datagram is simply gone — no
    /// retransmit, no stream fallback — so callers must only route messages
    /// here that tolerate loss/reorder (superseded state, latest-wins acks).
    /// Tagged `TAG_APP` so a receiver's datagram lane surfaces it as the
    /// same `ServerEvent::Message`/`ClientEvent::Message` the stream uses. A
    /// failed send (connection closing, payload too large) is counted in
    /// `NetMetrics::datagram_send_failures` and dropped — never falls back
    /// to the stream.
    pub fn send_datagram(&self, conn: ConnId, data: Vec<u8>) {
        let _ = self.out.send(Outgoing::Datagram(conn, encode_datagram(TAG_APP, &data)));
    }

    /// Bounded writer queue policy: drop connection when backlog exceeds this.
    pub const WRITER_QUEUE_CAP: usize = 128;

    /// Default hard cap on total simultaneous connections this endpoint will
    /// hold open — without it, a connection flood grows `ConnMap` (and every
    /// per-connection task/channel/queue) without bound. This is
    /// `NetLimits::default().max_connections`; override via
    /// `bind_with_limits`.
    pub const MAX_CONNECTIONS: usize = 4096;
    /// Default hard cap on simultaneous connections from a single source IP —
    /// bounds one hostile or misconfigured client from exhausting
    /// `MAX_CONNECTIONS` alone. This is
    /// `NetLimits::default().max_connections_per_ip`; an embedder modeling
    /// many distinct clients from one IP (a soak harness) overrides it via
    /// `bind_with_limits`.
    pub const MAX_CONNECTIONS_PER_IP: usize = 8;

    /// Reader-side token-bucket rate limit: the number of app frames a
    /// connection may have queued as burst headroom above its steady refill
    /// rate. Starts full so a legitimate opening burst isn't penalized.
    pub const MSG_BUCKET_CAPACITY: f64 = 128.0;
    /// Steady-state refill rate of the token bucket above, in tokens/sec.
    /// 2x the 60 Hz sim tick rate: generous headroom for a real client's
    /// intent stream (one message per tick plus occasional casts) while
    /// still bounding a flooding client's rate into the `ServerEvent`
    /// channel, which the simulation drains only once per Input tick.
    pub const MSG_REFILL_PER_SEC: f64 = 120.0;

    /// Close a connection from the server side (e.g. session takeover).
    /// Cleanup runs the normal path, so `Disconnected` still fires.
    pub fn disconnect(&self, conn: ConnId) {
        let _ = self.out.send(Outgoing::Kick(conn));
    }

    /// Microseconds since server start — the authoritative server clock.
    pub fn now_micros(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }

    /// Smoothed path RTT to a client (from QUIC), if connected.
    pub fn rtt_micros(&self, conn: ConnId) -> Option<u64> {
        self.rtts.lock().unwrap().get(&conn).map(|h| h.current.load(Ordering::Relaxed))
    }

    /// EWMA (mean, standard deviation) of `conn`'s RTT samples in
    /// microseconds, or `None` if the connection has no sample yet — the
    /// baseline a caller flags a current reading against.
    pub fn rtt_baseline(&self, conn: ConnId) -> Option<(f64, f64)> {
        let map = self.rtts.lock().unwrap();
        let stats = map.get(&conn)?.stats.lock().unwrap();
        stats.warmed.then(|| (stats.mean, stats.var.sqrt()))
    }

    /// Source IP of a connection, if still connected — the accessor the
    /// per-IP failed-login rate limiter reads to attribute a denied login
    /// to its source address.
    pub fn peer_ip(&self, conn: ConnId) -> Option<IpAddr> {
        self.peers.lock().unwrap().get(&conn).copied()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Frame/byte/backlog counters for this server (observability only).
    pub fn metrics(&self) -> Arc<NetMetrics> {
        self.metrics.clone()
    }
}

impl Drop for NetServer {
    /// Deterministically stops the accept loop, closes the QUIC endpoint
    /// (every connected client sees a close with reason "server shutdown"),
    /// and joins the network thread — bounded by `server_main`'s
    /// `wait_idle` timeout, so this cannot hang.
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn server_main(
    addr: SocketAddr,
    version: u8,
    epoch: Instant,
    events: UnboundedSender<ServerEvent>,
    mut out_rx: UnboundedReceiver<Outgoing>,
    conns: ConnMap,
    rtts: RttMap,
    peers: PeerMap,
    metrics: Arc<NetMetrics>,
    ready: std::sync::mpsc::Sender<Result<SocketAddr, NetError>>,
    limits: NetLimits,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let endpoint = match server_crypto()
        .and_then(|cfg| quinn::Endpoint::server(cfg, addr).map_err(NetError::Io))
    {
        Ok(ep) => {
            let local = ep.local_addr().expect("endpoint has a local addr");
            let _ = ready.send(Ok(local));
            ep
        }
        Err(e) => { let _ = ready.send(Err(e)); return; }
    };
    log::info!("net: listening on {}", endpoint.local_addr().unwrap());

    // Router: simulation → per-connection writer queues.
    let router_conns = conns.clone();
    let router_metrics = metrics.clone();
    tokio::spawn(async move {
        while let Some(out) = out_rx.recv().await {
            let map = router_conns.lock().unwrap();
            match out {
                Outgoing::To(id, data) => {
                    if let Some((tx, connection, depth)) = map.get(&id) {
                        enqueue(tx, depth, &router_metrics, connection, TAG_APP, data);
                    }
                }
                Outgoing::All(data) => {
                    // Arc clone: refcount bump, not a payload copy.
                    for (tx, connection, depth) in map.values() {
                        enqueue(tx, depth, &router_metrics, connection, TAG_APP, data.clone());
                    }
                }
                Outgoing::Kick(id) => {
                    if let Some((_, connection, _)) = map.get(&id) {
                        connection.close(0u32.into(), b"kicked");
                    }
                }
                Outgoing::Datagram(id, bytes) => {
                    if let Some((_, connection, _)) = map.get(&id) {
                        match connection.send_datagram(Bytes::from(bytes)) {
                            Ok(()) => router_metrics.record_datagram_out(),
                            Err(_) => router_metrics.record_datagram_send_failure(),
                        }
                    }
                }
            }
        }
    });

    // Busy-time instrumentation: a canary task wakes on a fixed cadence. On
    // this current-thread runtime nothing else can run while this task is
    // asleep, so any lateness on wakeup is exactly the time the thread spent
    // running other tasks (handshakes, frame codec, the accept loop) —
    // accumulated into `NetMetrics::busy_micros` as a busy-time proxy.
    {
        let metrics = metrics.clone();
        tokio::spawn(async move {
            const TICK: Duration = Duration::from_millis(20);
            loop {
                let before = Instant::now();
                tokio::time::sleep(TICK).await;
                let late = before.elapsed().saturating_sub(TICK);
                metrics.busy_micros.fetch_add(late.as_micros() as u64, Ordering::Relaxed);
            }
        });
    }

    let next_id = Arc::new(AtomicU64::new(1));
    let ip_counts: IpCounts = Arc::new(Mutex::new(HashMap::new()));
    let total_conns = Arc::new(AtomicU64::new(0));
    loop {
        let incoming = tokio::select! {
            incoming = endpoint.accept() => incoming,
            // A dropped sender also completes the receiver, so a leaked or
            // forgotten `NetServer` can never wedge this thread the other way.
            _ = &mut shutdown => break,
        };
        let Some(incoming) = incoming else { break };
        // QUIC address validation: force a retry round-trip before spending
        // any handshake work on a connection whose source address hasn't
        // been proven yet — closes the UDP amplification vector where a
        // spoofed source gets a bigger reply than it sent.
        if !incoming.remote_address_validated() {
            let _ = incoming.retry();
            continue;
        }

        // Connection caps: reserve a slot before the handshake even starts,
        // so a burst of near-simultaneous attempts can't slip past the cap
        // while earlier ones are still mid-handshake.
        let remote_ip = incoming.remote_address().ip();
        {
            let mut counts = ip_counts.lock().unwrap();
            let per_ip = *counts.get(&remote_ip).unwrap_or(&0);
            if total_conns.load(Ordering::Relaxed) as usize >= limits.max_connections
                || per_ip >= limits.max_connections_per_ip
            {
                incoming.refuse();
                continue;
            }
            *counts.entry(remote_ip).or_insert(0) += 1;
        }
        total_conns.fetch_add(1, Ordering::Relaxed);

        let id = next_id.fetch_add(1, Ordering::Relaxed);
        let events = events.clone();
        let conns = conns.clone();
        let rtts = rtts.clone();
        let peers = peers.clone();
        let metrics = metrics.clone();
        let ip_counts = ip_counts.clone();
        let total_conns = total_conns.clone();
        tokio::spawn(async move {
            match handle_connection(
                incoming, id, version, epoch, events.clone(), conns.clone(), rtts.clone(), peers.clone(),
                remote_ip, metrics.clone(),
            ).await {
                Ok(()) => log::info!("net: conn {id} closed"),
                Err(e) => log::info!("net: conn {id} ended: {e}"),
            }
            // Cleanup runs on every exit path; Disconnected only fires if Connected did.
            let removed = conns.lock().unwrap().remove(&id);
            rtts.lock().unwrap().remove(&id);
            peers.lock().unwrap().remove(&id);
            if let Some((_, _, depth)) = removed {
                // Frames still sitting in this connection's queue (never dequeued
                // because the writer task was aborted) must not leak into the
                // aggregate gauge forever.
                let leftover = depth.load(Ordering::Relaxed);
                if leftover > 0 {
                    metrics.writer_queue_depth.fetch_sub(leftover, Ordering::Relaxed);
                }
                let _ = events.send(ServerEvent::Disconnected(id));
            }
            // Release the connection-cap reservation taken before this task
            // was spawned, regardless of whether the handshake ever finished.
            total_conns.fetch_sub(1, Ordering::Relaxed);
            let mut counts = ip_counts.lock().unwrap();
            if let Some(c) = counts.get_mut(&remote_ip) {
                *c -= 1;
                if *c == 0 {
                    counts.remove(&remote_ip);
                }
            }
        });
    }

    // Shutdown: stop accepting, close every open connection with a reason
    // the client can surface, then give close frames a brief window to reach
    // the wire before the thread exits and the socket is released.
    endpoint.close(0u32.into(), b"server shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(3), endpoint.wait_idle()).await;
}

/// Handshake: the first frame must be `Hello` with a matching version —
/// reply `HelloAck`, or send `Reject` (flushed before returning, so the
/// reason reaches the client instead of being discarded by the close)
/// on a mismatch.
async fn handshake(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    version: u8,
) -> Result<(), NetError> {
    let (tag, payload) = read_frame_in(recv).await?;
    match (tag, decode_ctrl(&payload)) {
        (TAG_CTRL, Some(Ctrl::Hello { version: v })) if v == version => {}
        (TAG_CTRL, Some(Ctrl::Hello { version: v })) => {
            // A version mismatch must not be a silent close: send a Reject
            // frame with the reason before dropping the connection so the
            // client can surface it instead of being discarded by the close.
            let reason = format!("version mismatch: client {v}, server {version}");
            if write_frame(send, TAG_CTRL, &encode_ctrl(&Ctrl::Reject { reason: reason.clone() })).await.is_ok() {
                // A bare `return` here would drop `connection`/`send` with no
                // other handle left, which quinn treats as an implicit close
                // that can discard the frame just queued above before it
                // actually reaches the wire. `finish()` + `stopped()` waits
                // for the peer to receive it first.
                let _ = send.finish();
                let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
            }
            return Err(NetError::Handshake(reason));
        }
        _ => return Err(NetError::Handshake("expected Hello".into())),
    }
    write_frame(send, TAG_CTRL, &encode_ctrl(&Ctrl::HelloAck))
        .await
        .map_err(|e| NetError::Handshake(e.to_string()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    incoming: quinn::Incoming,
    id: ConnId,
    version: u8,
    epoch: Instant,
    events: UnboundedSender<ServerEvent>,
    conns: ConnMap,
    rtts: RttMap,
    peers: PeerMap,
    remote_ip: IpAddr,
    metrics: Arc<NetMetrics>,
) -> Result<(), NetError> {
    let connection = incoming.await.map_err(|e| NetError::Handshake(e.to_string()))?;
    let (mut send, mut recv) = connection
        .accept_bi()
        .await
        .map_err(|e| NetError::Handshake(e.to_string()))?;

    handshake(&mut send, &mut recv, version).await?;

    // Register the writer queue, announce the connection.
    let (write_tx, mut write_rx) = unbounded_channel::<(u8, Arc<Vec<u8>>)>();
    let depth = Arc::new(AtomicU64::new(0));
    conns.lock().unwrap().insert(id, (write_tx.clone(), connection.clone(), depth.clone()));
    // Own RTT handle, registered once at connect: the reader loop below
    // writes through this handle directly, with no map lock per frame.
    let rtt = Arc::new(RttHandle::new());
    rtts.lock().unwrap().insert(id, rtt.clone());
    // Source IP, registered once at connect: same lifecycle as `rtts`
    // above, removed by the same cleanup in `server_main`.
    peers.lock().unwrap().insert(id, remote_ip);
    let _ = events.send(ServerEvent::Connected(id));
    log::info!("net: conn {id} from {}", connection.remote_address());

    // Writer task — sole owner of the send stream.
    let writer_depth = depth.clone();
    let writer_metrics = metrics.clone();
    let writer = tokio::spawn(async move {
        while let Some((tag, payload)) = write_rx.recv().await {
            writer_depth.fetch_sub(1, Ordering::Relaxed);
            writer_metrics.writer_queue_depth.fetch_sub(1, Ordering::Relaxed);
            if write_frame(&mut send, tag, &payload).await.is_err() {
                break;
            }
            writer_metrics.record_frame_out(payload.len());
        }
    });

    // Datagram receive task — its own reader loop, independent of the
    // stream reader below, with its OWN token bucket (the stream
    // reader-loop bucket above is untouched). A datagram `Ctrl::Ping` is
    // answered DIRECTLY via `send_datagram` here, bypassing the writer queue
    // entirely — precisely the queueing delay a ping/pong is meant to avoid
    // measuring. Ends silently when `read_datagram` errors; the stream
    // reader loop below owns connection-teardown signaling, so this task is
    // aborted alongside the writer once that loop exits.
    let datagram_conn = connection.clone();
    let datagram_events = events.clone();
    let datagram_metrics = metrics.clone();
    let datagram_rtt = rtt.clone();
    let datagram_task = tokio::spawn(async move {
        let mut dgram_tokens = NetServer::MSG_BUCKET_CAPACITY;
        let mut dgram_last_refill = Instant::now();
        while let Ok(bytes) = datagram_conn.read_datagram().await {
            let Some((tag, payload)) = decode_datagram(&bytes) else { continue };
            datagram_metrics.record_datagram_in();
            match tag {
                TAG_CTRL => {
                    if let Some(Ctrl::Ping { t_client }) = decode_ctrl(payload) {
                        let pong = Ctrl::Pong { t_client, t_server: epoch.elapsed().as_micros() as u64 };
                        let out = encode_datagram(TAG_CTRL, &encode_ctrl(&pong));
                        match datagram_conn.send_datagram(Bytes::from(out)) {
                            Ok(()) => datagram_metrics.record_datagram_out(),
                            Err(_) => datagram_metrics.record_datagram_send_failure(),
                        }
                    }
                }
                TAG_APP => {
                    let now = Instant::now();
                    dgram_tokens = (dgram_tokens
                        + now.duration_since(dgram_last_refill).as_secs_f64() * NetServer::MSG_REFILL_PER_SEC)
                        .min(NetServer::MSG_BUCKET_CAPACITY);
                    dgram_last_refill = now;
                    if dgram_tokens < 1.0 {
                        datagram_metrics.record_reject();
                    } else {
                        dgram_tokens -= 1.0;
                        datagram_rtt.record(datagram_conn.rtt().as_micros() as u64);
                        let recv_micros = epoch.elapsed().as_micros() as u64;
                        let payload = payload.to_vec();
                        let _ = datagram_events.send(ServerEvent::Message { conn: id, data: payload, recv_micros });
                    }
                }
                _ => {}
            }
        }
    });

    // Reader-side token bucket: refills continuously, drained one token per
    // app frame. Bounds how fast this connection's frames turn into
    // `ServerEvent`s — without it, a client sending faster than the sim's
    // poll cadence grows the event channel without bound.
    let mut msg_tokens = NetServer::MSG_BUCKET_CAPACITY;
    let mut last_refill = Instant::now();

    // Reader loop — control frames answered here, app frames surfaced.
    let result = loop {
        match read_frame_in(&mut recv).await {
            Ok((tag, payload)) => {
                metrics.record_frame_in(payload.len());
                match tag {
                    TAG_CTRL => {
                        if let Some(Ctrl::Ping { t_client }) = decode_ctrl(&payload) {
                            let pong = Ctrl::Pong { t_client, t_server: epoch.elapsed().as_micros() as u64 };
                            enqueue(&write_tx, &depth, &metrics, &connection, TAG_CTRL, Arc::new(encode_ctrl(&pong)));
                        }
                    }
                    TAG_APP => {
                        let now = Instant::now();
                        msg_tokens = (msg_tokens
                            + now.duration_since(last_refill).as_secs_f64() * NetServer::MSG_REFILL_PER_SEC)
                            .min(NetServer::MSG_BUCKET_CAPACITY);
                        last_refill = now;
                        if msg_tokens < 1.0 {
                            // Over budget: drop the frame instead of queuing it. This
                            // is what keeps a flooding client from growing the
                            // ServerEvent channel without bound; the sim never even
                            // sees the frame existed.
                            metrics.record_reject();
                        } else {
                            msg_tokens -= 1.0;
                            rtt.record(connection.rtt().as_micros() as u64);
                            let recv_micros = epoch.elapsed().as_micros() as u64;
                            let _ = events.send(ServerEvent::Message { conn: id, data: payload, recv_micros });
                        }
                    }
                    _ => break Err(NetError::Handshake(format!("unknown frame tag {tag}"))),
                }
            }
            Err(e) => break Err(e),
        }
    };

    writer.abort();
    datagram_task.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{client_crypto, decode_ctrl, encode_ctrl, read_frame_out, write_frame, Ctrl, TAG_CTRL};
    use std::time::Duration;

    /// A steady stream of samples settles the EWMA mean near the sample
    /// value with variance collapsing toward zero — the quiet-connection
    /// baseline a later spike is measured against.
    #[test]
    fn estimator_converges_to_a_steady_rtt_with_near_zero_variance() {
        let mut est = RttEstimator::new();
        for _ in 0..200 {
            est.update(40_000.0);
        }
        assert!((est.mean - 40_000.0).abs() < 1.0, "mean should settle at the steady sample: {}", est.mean);
        assert!(est.var.sqrt() < 1.0, "variance should collapse on a constant input: {}", est.var);
    }

    /// Sigma tracks dispersion, not just the mean: the same absolute sample
    /// must clear a k*sigma check against a quiet connection's baseline but
    /// stay inside a jittery connection's baseline, because the jittery
    /// connection's own variance already accounts for swings of that size.
    #[test]
    fn estimator_flags_a_jump_on_a_quiet_connection_but_not_on_a_jittery_one() {
        let mut steady = RttEstimator::new();
        for _ in 0..200 {
            steady.update(40_000.0);
        }
        let (steady_mean, steady_std) = (steady.mean, steady.var.sqrt());
        assert!(steady_std < 1.0, "a constant input should collapse variance: {}", steady_std);

        let mut jittery = RttEstimator::new();
        for i in 0..400 {
            jittery.update(if i % 2 == 0 { 20_000.0 } else { 60_000.0 });
        }
        let (jittery_mean, jittery_std) = (jittery.mean, jittery.var.sqrt());
        assert!(jittery_std > 1_000.0, "an alternating input should carry real variance: {}", jittery_std);

        let k = 3.0;
        let spike_sample = 70_000.0;
        assert!(
            spike_sample > steady_mean + k * steady_std,
            "70_000 must clear the quiet connection's 3-sigma baseline (mean {steady_mean}, std {steady_std})"
        );
        assert!(
            spike_sample < jittery_mean + k * jittery_std,
            "70_000 must stay inside the jittery connection's 3-sigma baseline (mean {jittery_mean}, std {jittery_std})"
        );
    }

    /// Pins that a stalled reader gets kicked instead of the writer queue
    /// buffering forever: `WRITER_QUEUE_CAP` must actually be enforced, not
    /// merely declared. A raw (non-`NetClient`) connection is used here
    /// because `NetClient`'s own reader task always drains the wire
    /// regardless of whether the game polls it — this test needs a peer
    /// that genuinely never reads again, to force real QUIC flow-control
    /// backpressure on the server's send side.
    #[tokio::test]
    async fn stalled_reader_is_kicked_and_backlog_drains() {
        let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
        let addr = server.local_addr();

        // Tiny receive window: makes the server's writes block on flow
        // control almost immediately once we stop reading, deterministically
        // — instead of depending on however large quinn's default window is.
        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).expect("client endpoint");
        let mut client_config = client_crypto().expect("client crypto");
        let mut transport = quinn::TransportConfig::default();
        transport.stream_receive_window(quinn::VarInt::from_u32(4096));
        client_config.transport_config(Arc::new(transport));
        endpoint.set_default_client_config(client_config);

        let connection = endpoint
            .connect(addr, "localhost")
            .expect("connect")
            .await
            .expect("handshake");
        let (mut send, mut recv) = connection.open_bi().await.expect("open bi");
        write_frame(&mut send, TAG_CTRL, &encode_ctrl(&Ctrl::Hello { version: 1 }))
            .await
            .expect("send hello");
        // Read exactly the HelloAck, then never read again — the stalled reader.
        read_frame_out(&mut recv).await.expect("hello ack");

        // Wait for the server to register the connection.
        let deadline = Instant::now() + Duration::from_secs(5);
        let conn_id = loop {
            if let Some(id) = server.poll().into_iter().find_map(|ev| match ev {
                ServerEvent::Connected(id) => Some(id),
                _ => None,
            }) {
                break id;
            }
            assert!(Instant::now() < deadline, "server never saw the connection");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        // Flood well past WRITER_QUEUE_CAP while the client never reads again.
        let payload = vec![0xABu8; 512];
        for _ in 0..(NetServer::WRITER_QUEUE_CAP * 4) {
            server.broadcast(payload.clone());
        }

        // The server must kick the stalled connection instead of growing the
        // writer queue forever.
        let deadline = Instant::now() + Duration::from_secs(10);
        let kicked = loop {
            if server.poll().into_iter().any(|ev| matches!(ev, ServerEvent::Disconnected(id) if id == conn_id)) {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(kicked, "server never kicked the stalled reader — writer queue would grow without bound");

        // The aggregate backlog gauge must have drained back to zero — proves
        // the leftover (never-dequeued) frames were accounted for at cleanup,
        // not leaked into the gauge forever.
        let depth_after_kick = server.metrics().writer_queue_depth.load(Ordering::Relaxed);
        assert_eq!(depth_after_kick, 0, "writer queue depth did not drain after the stalled connection was kicked");

        drop(connection);
    }

    /// Pins that dropping a `NetServer` closes the accept loop and the QUIC
    /// endpoint, not merely the event/outgoing channels: the client must be
    /// told the server is gone, and the listening socket must be released
    /// immediately so a rebind on the same address succeeds.
    #[tokio::test]
    async fn drop_closes_endpoint_notifies_client_and_releases_port() {
        let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
        let addr = server.local_addr();

        let mut client = crate::NetClient::connect(addr, 1).expect("connect");

        // Wait for the server to register the connection.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if server.poll().into_iter().any(|ev| matches!(ev, ServerEvent::Connected(_))) {
                break;
            }
            assert!(Instant::now() < deadline, "server never saw the connection");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        drop(server);

        // (a) The client must observe Disconnected within a deadline — the
        // server's close(reason) must actually reach the wire.
        let deadline = Instant::now() + Duration::from_secs(5);
        let disconnected = loop {
            if client.poll().into_iter().any(|ev| matches!(ev, crate::ClientEvent::Disconnected)) {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(disconnected, "client never observed Disconnected after the server was dropped");

        // (b) The listening socket must be released immediately: dropping
        // the server frees the port so this rebind succeeds.
        let _rebound = NetServer::bind(addr, 1)
            .expect("rebind on the same address should succeed immediately after drop");
    }

    /// Pins `peer_ip`'s lifecycle, the per-IP failed-login rate limiter's
    /// dependency for attributing a failed login to an IP: it must mirror
    /// `rtts`'s exact lifecycle — populated once `Connected` fires, gone
    /// once `Disconnected` fires.
    #[tokio::test]
    async fn peer_ip_tracks_connection_lifecycle() {
        let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
        let addr = server.local_addr();

        let client = crate::NetClient::connect(addr, 1).expect("connect");

        let deadline = Instant::now() + Duration::from_secs(5);
        let conn_id = loop {
            if let Some(id) = server.poll().into_iter().find_map(|ev| match ev {
                ServerEvent::Connected(id) => Some(id),
                _ => None,
            }) {
                break id;
            }
            assert!(Instant::now() < deadline, "server never saw the connection");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        assert_eq!(
            server.peer_ip(conn_id),
            Some("127.0.0.1".parse().unwrap()),
            "peer_ip must report the connected client's source address"
        );

        drop(client);

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if server.poll().into_iter().any(|ev| matches!(ev, ServerEvent::Disconnected(id) if id == conn_id)) {
                break;
            }
            assert!(Instant::now() < deadline, "server never saw the disconnect");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(server.peer_ip(conn_id), None, "peer_ip must be gone after the connection ends");
    }

    /// Pins the datagram lane's server→client direction:
    /// `NetServer::send_datagram` must deliver to the client as a
    /// `ClientEvent::Message`, independent of the reliable ordered stream.
    #[tokio::test]
    async fn datagram_lane_delivers_server_to_client_message() {
        let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
        let addr = server.local_addr();
        let mut client = crate::NetClient::connect(addr, 1).expect("connect");

        let deadline = Instant::now() + Duration::from_secs(5);
        let conn_id = loop {
            if let Some(id) = server.poll().into_iter().find_map(|ev| match ev {
                ServerEvent::Connected(id) => Some(id),
                _ => None,
            }) {
                break id;
            }
            assert!(Instant::now() < deadline, "server never saw the connection");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        // Wait for the client to observe Connected too, so its datagram
        // receive task is definitely up before the server sends.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if client.poll().into_iter().any(|ev| matches!(ev, crate::ClientEvent::Connected)) {
                break;
            }
            assert!(Instant::now() < deadline, "client never observed Connected");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let payload = b"hello-over-datagram".to_vec();
        server.send_datagram(conn_id, payload.clone());

        let deadline = Instant::now() + Duration::from_secs(5);
        let received = loop {
            if let Some(data) = client.poll().into_iter().find_map(|ev| match ev {
                crate::ClientEvent::Message(data) => Some(data),
                _ => None,
            }) {
                break Some(data);
            }
            if Instant::now() >= deadline {
                break None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(
            received,
            Some(payload),
            "client never received the server's datagram-lane message"
        );
    }

    /// Pins the datagram lane's client→server direction:
    /// `NetClient::send_datagram` must surface at the server as the same
    /// `ServerEvent::Message` the stream produces, with a real (nonzero)
    /// `recv_micros` arrival stamp.
    #[tokio::test]
    async fn datagram_lane_delivers_client_to_server_message() {
        let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
        let addr = server.local_addr();
        let client = crate::NetClient::connect(addr, 1).expect("connect");

        let deadline = Instant::now() + Duration::from_secs(5);
        let conn_id = loop {
            if let Some(id) = server.poll().into_iter().find_map(|ev| match ev {
                ServerEvent::Connected(id) => Some(id),
                _ => None,
            }) {
                break id;
            }
            assert!(Instant::now() < deadline, "server never saw the connection");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        let payload = b"client-datagram-payload".to_vec();
        client.send_datagram(payload.clone());

        let deadline = Instant::now() + Duration::from_secs(5);
        let received = loop {
            if let Some(found) = server.poll().into_iter().find_map(|ev| match ev {
                ServerEvent::Message { conn, data, recv_micros } if conn == conn_id => Some((data, recv_micros)),
                _ => None,
            }) {
                break Some(found);
            }
            if Instant::now() >= deadline {
                break None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let (data, recv_micros) =
            received.expect("server never received the client's datagram-lane message");
        assert_eq!(data, payload);
        assert!(recv_micros > 0, "recv_micros should be nonzero — the server epoch has elapsed by arrival");
    }

    /// Pins that the datagram ctrl-ping path answers DIRECTLY via
    /// `send_datagram` instead of the per-connection writer queue —
    /// precisely the queueing delay a ping/pong is meant to measure around.
    /// A raw (non-`NetClient`) connection sends a `Ctrl::Ping` as a bare
    /// datagram and must get a `Ctrl::Pong` datagram back while
    /// `NetMetrics::frames_out` (the STREAM counter, incremented only by the
    /// writer task) stays at 0 — proof the reply never touched the writer
    /// queue at all.
    #[tokio::test]
    async fn datagram_ctrl_ping_gets_direct_pong_bypassing_writer_queue() {
        let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
        let addr = server.local_addr();

        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).expect("client endpoint");
        endpoint.set_default_client_config(client_crypto().expect("client crypto"));
        let connection = endpoint
            .connect(addr, "localhost")
            .expect("connect")
            .await
            .expect("handshake");
        let (mut send, mut recv) = connection.open_bi().await.expect("open bi");
        write_frame(&mut send, TAG_CTRL, &encode_ctrl(&Ctrl::Hello { version: 1 }))
            .await
            .expect("send hello");
        read_frame_out(&mut recv).await.expect("hello ack");

        // Wait for the server to register the connection.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if server.poll().into_iter().any(|ev| matches!(ev, ServerEvent::Connected(_))) {
                break;
            }
            assert!(Instant::now() < deadline, "server never saw the connection");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Send a Ctrl::Ping as a raw datagram — never touching the stream.
        let ping = Ctrl::Ping { t_client: 12345 };
        let dgram = encode_datagram(TAG_CTRL, &encode_ctrl(&ping));
        connection.send_datagram(Bytes::from(dgram)).expect("send ping datagram");

        let pong_bytes = tokio::time::timeout(Duration::from_secs(5), connection.read_datagram())
            .await
            .expect("timed out waiting for the pong datagram")
            .expect("read_datagram failed");
        let (tag, payload) = decode_datagram(&pong_bytes).expect("empty datagram");
        assert_eq!(tag, TAG_CTRL);
        match decode_ctrl(payload) {
            Some(Ctrl::Pong { t_client, .. }) => assert_eq!(t_client, 12345),
            _ => panic!("expected a decodable Ctrl::Pong datagram"),
        }

        assert_eq!(
            server.metrics().frames_out.load(Ordering::Relaxed),
            0,
            "pong must bypass the writer queue entirely — frames_out only counts stream writes"
        );

        drop(connection);
    }

    /// Pins that clock pings ride the datagram lane instead of the reliable
    /// stream: this test waits for the clock to converge
    /// (`server_offset_micros` becomes `Some`, which requires at least one
    /// ping/pong round trip) and asserts the client's stream `frames_out` is
    /// still 0 — the test itself never sends anything over the stream, so
    /// any nonzero count can only be ping traffic that leaked onto the
    /// ordered stream.
    #[tokio::test]
    async fn clock_pings_never_touch_the_stream() {
        let server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
        let addr = server.local_addr();
        let client = crate::NetClient::connect(addr, 1).expect("connect");

        let deadline = Instant::now() + Duration::from_secs(5);
        let synced = loop {
            if client.server_offset_micros().is_some() {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(synced, "client clock never converged");

        assert_eq!(
            client.metrics().frames_out.load(Ordering::Relaxed),
            0,
            "ctrl pings must ride the datagram lane, never the stream — frames_out counts stream writes only"
        );

        drop(server);
    }
}
