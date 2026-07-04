// NetServer — listening side. One tokio runtime on a dedicated thread; the
// simulation polls events and pushes sends through unbounded channels.

use crate::common::{
    decode_ctrl, encode_ctrl, read_frame, server_crypto, write_frame, Ctrl, TAG_APP, TAG_CTRL,
};
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

/// Per-connection writer queue + the quinn handle (for server-side close).
type ConnMap = Arc<Mutex<HashMap<ConnId, (UnboundedSender<(u8, Arc<Vec<u8>>)>, quinn::Connection)>>>;
type RttMap = Arc<Mutex<HashMap<ConnId, u64>>>;

pub struct NetServer {
    events: UnboundedReceiver<ServerEvent>,
    out: UnboundedSender<Outgoing>,
    epoch: Instant,
    local_addr: SocketAddr,
    rtts: RttMap,
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

        // Report bind success/failure synchronously before the thread detaches.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<SocketAddr, NetError>>();

        let thread_conns = conns.clone();
        let thread_rtts = rtts.clone();
        std::thread::Builder::new()
            .name("engine-net-server".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => { let _ = ready_tx.send(Err(NetError::Io(e))); return; }
                };
                rt.block_on(server_main(
                    addr, version, epoch, event_tx, out_rx, thread_conns, thread_rtts, ready_tx,
                ));
            })
            .map_err(NetError::Io)?;

        let local_addr = ready_rx
            .recv()
            .map_err(|_| NetError::Handshake("network thread died during bind".into()))??;

        Ok(Self { events: event_rx, out: out_tx, epoch, local_addr, rtts })
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
    tokio::spawn(async move {
        while let Some(out) = out_rx.recv().await {
            let map = router_conns.lock().unwrap();
            match out {
                Outgoing::To(id, data) => {
                    if let Some((tx, _)) = map.get(&id) { let _ = tx.send((TAG_APP, data)); }
                }
                Outgoing::All(data) => {
                    // Arc clone: refcount bump, not a payload copy.
                    for (tx, _) in map.values() { let _ = tx.send((TAG_APP, data.clone())); }
                }
                Outgoing::Kick(id) => {
                    if let Some((_, connection)) = map.get(&id) {
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
        tokio::spawn(async move {
            match handle_connection(incoming, id, version, epoch, events.clone(), conns.clone(), rtts.clone()).await {
                Ok(()) => log::info!("net: conn {id} closed"),
                Err(e) => log::info!("net: conn {id} ended: {e}"),
            }
            // Cleanup runs on every exit path; Disconnected only fires if Connected did.
            let was_registered = conns.lock().unwrap().remove(&id).is_some();
            rtts.lock().unwrap().remove(&id);
            if was_registered {
                let _ = events.send(ServerEvent::Disconnected(id));
            }
        });
    }
}

async fn handle_connection(
    incoming: quinn::Incoming,
    id: ConnId,
    version: u8,
    epoch: Instant,
    events: UnboundedSender<ServerEvent>,
    conns: ConnMap,
    rtts: RttMap,
) -> Result<(), NetError> {
    let connection = incoming.await.map_err(|e| NetError::Handshake(e.to_string()))?;
    let (mut send, mut recv) = connection
        .accept_bi()
        .await
        .map_err(|e| NetError::Handshake(e.to_string()))?;

    // Handshake: first frame must be Hello with a matching version.
    let (tag, payload) = read_frame(&mut recv).await?;
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
    conns.lock().unwrap().insert(id, (write_tx.clone(), connection.clone()));
    let _ = events.send(ServerEvent::Connected(id));
    log::info!("net: conn {id} from {}", connection.remote_address());

    // Writer task — sole owner of the send stream.
    let writer = tokio::spawn(async move {
        while let Some((tag, payload)) = write_rx.recv().await {
            if write_frame(&mut send, tag, &payload).await.is_err() {
                break;
            }
        }
    });

    // Reader loop — control frames answered here, app frames surfaced.
    let result = loop {
        match read_frame(&mut recv).await {
            Ok((TAG_CTRL, payload)) => {
                if let Some(Ctrl::Ping { t_client }) = decode_ctrl(&payload) {
                    let pong = Ctrl::Pong { t_client, t_server: epoch.elapsed().as_micros() as u64 };
                    let _ = write_tx.send((TAG_CTRL, Arc::new(encode_ctrl(&pong))));
                }
            }
            Ok((TAG_APP, data)) => {
                rtts.lock().unwrap().insert(id, connection.rtt().as_micros() as u64);
                let recv_micros = epoch.elapsed().as_micros() as u64;
                let _ = events.send(ServerEvent::Message { conn: id, data, recv_micros });
            }
            Ok((tag, _)) => break Err(NetError::Handshake(format!("unknown frame tag {tag}"))),
            Err(e) => break Err(e),
        }
    };

    writer.abort();
    result
}
