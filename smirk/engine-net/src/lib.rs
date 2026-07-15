// engine-net — QUIC transport for client/server games.
//
// Design (DESIGN.md §5):
//   - The simulation thread stays single-threaded and deterministic: this crate
//     runs tokio + quinn on a dedicated network thread and exposes a synchronous,
//     channel-based API (`NetServer` / `NetClient`) polled from Phase::Input.
//   - Payloads are opaque bytes — the game protocol (vordar-protocol) lives above.
//   - Handshake (version byte) and NTP-style clock synchronization are handled
//     here as control frames, invisible to the application.
//   - Dev security model: self-signed server certificate, clients skip
//     verification. Real certificates can be swapped in later without touching
//     the API.
//
// Two lanes carry frames:
//   - Reliable ordered stream, per QUIC bidirectional stream:
//       [u32 LE frame length][u8 tag][payload]
//       tag 0 = control (postcard-encoded Ctrl), tag 1 = application bytes.
//   - Unreliable QUIC datagram, one per `send_datagram`/`read_datagram`:
//       [u8 tag][payload] — no length prefix; a datagram is self-delimiting
//       on arrival. Same tag values as the stream. Payloads are opaque bytes
//       either way; which messages tolerate loss/reorder well enough to ride
//       the datagram lane is a decision for the layer above (vordar-protocol),
//       not this crate. Both directions surface a received datagram exactly
//       like a received stream frame — `ServerEvent::Message`/
//       `ClientEvent::Message` — so callers don't fork on which lane a
//       message arrived over. A datagram `Ctrl::Ping` is answered directly
//       via `send_datagram`, bypassing the stream's per-connection writer
//       queue entirely, so its RTT sample carries no queueing delay.
//       `send_datagram` failures (connection closing, payload too large) are
//       counted in `NetMetrics` and dropped — best-effort by contract, never
//       falling back to the stream.
//
// Clock sync: the client pings (t_client), the server pongs (t_client, t_server),
// the client computes offset = (t_server + rtt/2) - t_now, keeping the sample
// with the lowest RTT (lowest-RTT samples have the most symmetric paths).
// `server_now_micros()` then maps local time onto the server clock — the basis
// for telegraph timing and intent timestamps.

mod common;
mod client;
mod clock;
mod impair;
mod metrics;
mod server;

pub use client::{ClientEvent, Impairment, NetClient};
pub use server::{ConnId, NetLimits, NetServer, ServerEvent};
pub use common::{MAX_FRAME_IN, MAX_FRAME_OUT};
pub use metrics::NetMetrics;

/// Errors surfaced to the simulation thread. Network-thread internals log and
/// translate into Disconnected events instead of bubbling errors per call.
#[derive(Debug)]
pub enum NetError {
    Io(std::io::Error),
    Tls(String),
    Handshake(String),
    Closed,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e)        => write!(f, "io error: {e}"),
            Self::Tls(e)       => write!(f, "tls setup error: {e}"),
            Self::Handshake(e) => write!(f, "handshake failed: {e}"),
            Self::Closed       => write!(f, "connection closed"),
        }
    }
}

impl std::error::Error for NetError {}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}
