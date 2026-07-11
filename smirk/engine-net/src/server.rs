// NetServer — listening side. One tokio runtime on a dedicated thread; the
// simulation polls events and pushes sends through unbounded channels.

use crate::common::{
    decode_ctrl, encode_ctrl, read_frame_in, server_crypto, write_frame, Ctrl, TAG_APP, TAG_CTRL,
};
use crate::metrics::NetMetrics;
use crate::NetError;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
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
}

/// Per-connection writer queue + the quinn handle (for server-side close) +
/// the queue's current backlog depth (items enqueued, not yet dequeued by the
/// writer task) — the budget `WRITER_QUEUE_CAP` is enforced against.
type ConnMap = Arc<
    Mutex<HashMap<ConnId, (UnboundedSender<(u8, Arc<Vec<u8>>)>, quinn::Connection, Arc<AtomicU64>)>>,
>;
type RttMap = Arc<Mutex<HashMap<ConnId, u64>>>;

pub struct NetServer {
    events: UnboundedReceiver<ServerEvent>,
    out: UnboundedSender<Outgoing>,
    epoch: Instant,
    local_addr: SocketAddr,
    rtts: RttMap,
    metrics: Arc<NetMetrics>,
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
    /// application protocol version checked during the handshake.
    pub fn bind(addr: SocketAddr, version: u8) -> Result<Self, NetError> {
        let epoch = Instant::now();
        let (event_tx, event_rx) = unbounded_channel();
        let (out_tx, out_rx) = unbounded_channel();
        let conns: ConnMap = Arc::new(Mutex::new(HashMap::new()));
        let rtts: RttMap = Arc::new(Mutex::new(HashMap::new()));
        let metrics = NetMetrics::new();

        // Report bind success/failure synchronously before the thread detaches.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<SocketAddr, NetError>>();

        let thread_conns = conns.clone();
        let thread_rtts = rtts.clone();
        let thread_metrics = metrics.clone();
        std::thread::Builder::new()
            .name("engine-net-server".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => { let _ = ready_tx.send(Err(NetError::Io(e))); return; }
                };
                rt.block_on(server_main(
                    addr, version, epoch, event_tx, out_rx, thread_conns, thread_rtts,
                    thread_metrics, ready_tx,
                ));
            })
            .map_err(NetError::Io)?;

        let local_addr = ready_rx
            .recv()
            .map_err(|_| NetError::Handshake("network thread died during bind".into()))??;

        Ok(Self { events: event_rx, out: out_tx, epoch, local_addr, rtts, metrics })
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

    /// Bounded writer queue policy: drop connection when backlog exceeds this.
    pub const WRITER_QUEUE_CAP: usize = 128;

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
        self.rtts.lock().unwrap().get(&conn).copied()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Frame/byte/backlog counters for this server (observability only).
    pub fn metrics(&self) -> Arc<NetMetrics> {
        self.metrics.clone()
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
    metrics: Arc<NetMetrics>,
    ready: std::sync::mpsc::Sender<Result<SocketAddr, NetError>>,
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
            }
        }
    });

    let next_id = Arc::new(AtomicU64::new(1));
    while let Some(incoming) = endpoint.accept().await {
        let id = next_id.fetch_add(1, Ordering::Relaxed);
        let events = events.clone();
        let conns = conns.clone();
        let rtts = rtts.clone();
        let metrics = metrics.clone();
        tokio::spawn(async move {
            match handle_connection(
                incoming, id, version, epoch, events.clone(), conns.clone(), rtts.clone(), metrics.clone(),
            ).await {
                Ok(()) => log::info!("net: conn {id} closed"),
                Err(e) => log::info!("net: conn {id} ended: {e}"),
            }
            // Cleanup runs on every exit path; Disconnected only fires if Connected did.
            let removed = conns.lock().unwrap().remove(&id);
            rtts.lock().unwrap().remove(&id);
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
        });
    }
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
    metrics: Arc<NetMetrics>,
) -> Result<(), NetError> {
    let connection = incoming.await.map_err(|e| NetError::Handshake(e.to_string()))?;
    let (mut send, mut recv) = connection
        .accept_bi()
        .await
        .map_err(|e| NetError::Handshake(e.to_string()))?;

    // Handshake: first frame must be Hello with a matching version.
    let (tag, payload) = read_frame_in(&mut recv).await?;
    match (tag, decode_ctrl(&payload)) {
        (TAG_CTRL, Some(Ctrl::Hello { version: v })) if v == version => {}
        (TAG_CTRL, Some(Ctrl::Hello { version: v })) => {
            return Err(NetError::Handshake(format!("version mismatch: client {v}, server {version}")));
        }
        _ => return Err(NetError::Handshake("expected Hello".into())),
    }
    write_frame(&mut send, TAG_CTRL, &encode_ctrl(&Ctrl::HelloAck))
        .await
        .map_err(|e| NetError::Handshake(e.to_string()))?;

    // Register the writer queue, announce the connection.
    let (write_tx, mut write_rx) = unbounded_channel::<(u8, Arc<Vec<u8>>)>();
    let depth = Arc::new(AtomicU64::new(0));
    conns.lock().unwrap().insert(id, (write_tx.clone(), connection.clone(), depth.clone()));
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
                        rtts.lock().unwrap().insert(id, connection.rtt().as_micros() as u64);
                        let recv_micros = epoch.elapsed().as_micros() as u64;
                        let _ = events.send(ServerEvent::Message { conn: id, data: payload, recv_micros });
                    }
                    _ => break Err(NetError::Handshake(format!("unknown frame tag {tag}"))),
                }
            }
            Err(e) => break Err(e),
        }
    };

    writer.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{client_crypto, encode_ctrl, read_frame_out, write_frame, Ctrl, TAG_CTRL};
    use std::time::Duration;

    /// Regression test for the writer-queue-cap facade fix (networking audit
    /// 2026-07-11, finding 3). Before this fix `WRITER_QUEUE_CAP` was declared
    /// but never read: the writer queue was a plain unbounded channel, so a
    /// client that stopped draining its stream made the server buffer frames
    /// forever. A raw (non-`NetClient`) connection is used here because
    /// `NetClient`'s own reader task always drains the wire regardless of
    /// whether the game polls it — this test needs a peer that genuinely
    /// never reads again, to force real QUIC flow-control backpressure on
    /// the server's send side.
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
}
