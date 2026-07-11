// vordar-protocol — the wire vocabulary between client and server.
//
// Versioned: bump PROTOCOL_VERSION on any breaking change; engine-net rejects
// mismatched versions during the handshake. Messages ride postcard-encoded
// inside engine-net app frames.
//
// Authority rules baked into the shape of these types (DESIGN.md §3):
//   - Clients send INTENTS, never positions or state. The server recomputes
//     everything from intents — a claimed position cannot exist on the wire.
//   - Intents carry the client's estimate of *server* time (synced clock) so
//     the server can lag-compensate within bounded limits.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 8;

/// Snapshot rate. The server drives its snapshot phase with this; the client
/// uses it to pace interpolation between snapshots.
pub const SNAPSHOT_HZ: f32 = 10.0;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ClientMsg {
    /// Desired movement direction (≤ unit length, world XZ plane).
    /// `seq` increases monotonically; `t_server_micros` is the synced-clock
    /// stamp — both are validated server-side (anti-cheat caps, DESIGN.md §3).
    MoveIntent { seq: u32, t_server_micros: u64, dir: Vec2 },
    /// Cast skill `skill` at world-XZ `target`. Shares the seq/timestamp
    /// stream (and its validation) with MoveIntent; bypasses the movement
    /// queue — the cast time is the delay.
    CastIntent { seq: u32, t_server_micros: u64, skill: String, target: Vec2 },
    /// First message after connect: which character this connection plays.
    /// Identity without authentication during development — accounts and
    /// passwords land later. The server gates spawn + Welcome on this.
    Login { name: String }, // validated ≤ 32 printable ASCII
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMsg {
    /// Sent once after connect: which replicated entity is yours.
    Welcome { player_id: u64 },
    /// Per-client area-of-interest snapshot. Entity identity (prefab) is sent
    /// once on AOI entry; afterward only positions flow. `last_processed_seq`
    /// is the highest intent seq the server had applied when the snapshot was
    /// taken: the client drops acknowledged pending intents and replays the
    /// rest on top of its own position (prediction reconciliation).
    Snapshot {
        tick: u64,
        last_processed_seq: u32,
        /// Entities that entered your AOI (or spawned inside it) — spawn these.
        enters: Vec<EntityState>,
        /// Entities that left your AOI (or despawned) — despawn these.
        leaves: Vec<u64>,
        /// Current position of every entity in your AOI.
        states: Vec<EntityPos>,
    },
    /// A mechanic was scheduled (DESIGN.md §3): area `radius` at `pos`,
    /// resolving at absolute server time `resolve_at_micros`. Sent IDENTICALLY
    /// to every AOI-scoped recipient — countdowns anchor to the synced clock,
    /// so receive time never matters. T = telegraph visual completion. Sent
    /// only to connections within AOI range of `pos` (Finding 5: this used to
    /// broadcast zone-wide, leaking telegraph positions to distant clients).
    MechanicScheduled {
        id: u64,
        telegraph_prefab: String,
        pos: Vec3,
        radius: f32,
        resolve_at_micros: u64,
        duration_micros: u64,
    },
    /// Outcome of a resolved mechanic: which entities were inside at T. Sent
    /// only to connections within AOI range of the mechanic's position.
    HitResult { mechanic: u64, hits: Vec<u64> },
    /// World-clock sample: world time `world_micros` corresponded to server
    /// time `at_server_micros`. Combined with clock sync, clients evaluate
    /// world time (day/night, world events) as a pure local function — the
    /// event definitions themselves are shared content (DESIGN.md §4). Sent
    /// on connect and re-broadcast periodically.
    WorldClock { world_micros: u64, at_server_micros: u64 },
    /// Your character belongs to zone `zone`, served at `addr`: drop this
    /// connection, connect there, and log in again. Sent on portal transfer
    /// (after the character has been persisted into the target zone) and on
    /// logging into a zone that doesn't own the character. The CLIENT closes
    /// the old connection — a server-side kick could outrace this frame.
    Redirect { zone: String, addr: std::net::SocketAddr },
    /// An entity in your AOI died at `pos` (v8). Snapshots stop mentioning it
    /// the same tick, so this is the client's only death signal — it drives
    /// the cosmetic corpse + death burst. Sent only to clients whose known
    /// set contains the entity.
    EntityDied { id: u64, pos: Vec3 },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EntityState {
    /// Server-side entity id (hecs Entity bits) — stable for the entity's lifetime.
    pub id: u64,
    /// Prefab to spawn client-side for this entity.
    pub prefab: String,
    pub pos: Vec3,
    /// Current health (v8) — cosmetic on the client (hit reacts, health bars);
    /// 0 for entities without a Health component.
    pub hp: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EntityPos {
    pub id: u64,
    pub pos: Vec3,
    /// Current health (v8); 0 for entities without a Health component.
    pub hp: i32,
}

pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    postcard::to_allocvec(msg).expect("protocol serialization cannot fail")
}

pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Option<T> {
    postcard::from_bytes(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_msg_roundtrip() {
        let msg = ClientMsg::MoveIntent { seq: 7, t_server_micros: 123_456, dir: Vec2::new(0.6, -0.8) };
        let bytes = encode(&msg);
        match decode::<ClientMsg>(&bytes).unwrap() {
            ClientMsg::MoveIntent { seq, t_server_micros, dir } => {
                assert_eq!(seq, 7);
                assert_eq!(t_server_micros, 123_456);
                assert!((dir - Vec2::new(0.6, -0.8)).length() < 1e-6);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn cast_intent_roundtrip() {
        let msg = ClientMsg::CastIntent {
            seq: 9,
            t_server_micros: 222,
            skill: "blast".into(),
            target: Vec2::new(3.0, -1.0),
        };
        let bytes = encode(&msg);
        match decode::<ClientMsg>(&bytes).unwrap() {
            ClientMsg::CastIntent { seq, skill, target, .. } => {
                assert_eq!(seq, 9);
                assert_eq!(skill, "blast");
                assert!((target - Vec2::new(3.0, -1.0)).length() < 1e-6);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn login_roundtrip() {
        let msg = ClientMsg::Login { name: "alice".into() };
        let bytes = encode(&msg);
        match decode::<ClientMsg>(&bytes).unwrap() {
            ClientMsg::Login { name } => assert_eq!(name, "alice"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn server_msg_roundtrip() {
        let msg = ServerMsg::Snapshot {
            tick: 42,
            last_processed_seq: 17,
            enters: vec![EntityState { id: 9, prefab: "player".into(), pos: Vec3::new(1.0, 0.0, -3.0), hp: 100 }],
            leaves: vec![4],
            states: vec![EntityPos { id: 9, pos: Vec3::new(1.0, 0.0, -3.0), hp: 100 }],
        };
        let bytes = encode(&msg);
        match decode::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::Snapshot { tick, last_processed_seq, enters, leaves, states } => {
                assert_eq!(tick, 42);
                assert_eq!(last_processed_seq, 17);
                assert_eq!(enters.len(), 1);
                assert_eq!(enters[0].prefab, "player");
                assert_eq!(leaves, vec![4]);
                assert_eq!(states[0].id, 9);
                assert_eq!(states[0].hp, 100, "hp rides in every state (v8)");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn entity_died_roundtrip() {
        let msg = ServerMsg::EntityDied { id: 77, pos: Vec3::new(2.0, 0.0, 5.0) };
        let bytes = encode(&msg);
        match decode::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::EntityDied { id, pos } => {
                assert_eq!(id, 77);
                assert!((pos - Vec3::new(2.0, 0.0, 5.0)).length() < 1e-6);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn redirect_roundtrip() {
        let addr: std::net::SocketAddr = "127.0.0.1:5152".parse().unwrap();
        let msg = ServerMsg::Redirect { zone: "east".into(), addr };
        let bytes = encode(&msg);
        match decode::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::Redirect { zone, addr: got } => {
                assert_eq!(zone, "east");
                assert_eq!(got, addr);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn corrupt_bytes_decode_to_none() {
        assert!(decode::<ServerMsg>(&[0xFF, 0xFF, 0xFF]).is_none());
    }
}
