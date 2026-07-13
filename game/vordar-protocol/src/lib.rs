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

pub const PROTOCOL_VERSION: u8 = 12;

/// A client's account credential: a random 32-byte token, presented on every
/// `Login` and verified server-side against `sha256(token)` stored in the
/// `accounts` table (trust-on-first-use — first login claims the name).
pub type AccountToken = [u8; 32];

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
    /// First message after connect: which character this connection plays,
    /// and the account credential proving it (networking rework 1 finding
    /// 3). `token` is verified server-side against `sha256(token)` stored in
    /// the `accounts` table — trust-on-first-use, so a fresh name claims
    /// itself on first login. The server gates spawn + Welcome on this.
    Login { name: String, token: AccountToken }, // name validated ≤ 32 printable ASCII
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMsg {
    /// Sent once after connect: which replicated entity is yours. `player_id`
    /// is a zone-local wire id (protocol v10, networking rework 5 finding 1) —
    /// small and monotonic, not raw hecs `Entity` bits.
    Welcome { player_id: u32 },
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
        leaves: Vec<u32>,
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
    /// `hits` are zone-local wire ids (protocol v10); `mechanic` is a
    /// separate id space (server's `next_mechanic_id`), unaffected.
    HitResult { mechanic: u64, hits: Vec<u32> },
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
    /// set contains the entity. `id` is a zone-local wire id (protocol v10).
    EntityDied { id: u32, pos: Vec3 },
    /// The presented `Login` was rejected (v9, networking rework 1 finding
    /// 3): invalid/mismatched token (`BadCredentials`), or — once finding 4
    /// wires it — too many recent failures from this IP (`RateLimited`,
    /// declared now so the wire never bumps twice). The server leaves the
    /// connection open; the CLIENT closes it, same as `Redirect` and the
    /// Phase-6 takeover — a server-side kick could outrace this frame.
    LoginDenied { reason: LoginDenyReason },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginDenyReason {
    /// An invalid name, or a name already claimed by a different token than
    /// the one presented (trust-on-first-use: a fresh name always claims and
    /// grants — this is only a mismatch against an existing claim).
    BadCredentials,
    /// This source IP has too many recent failed logins (finding 4).
    RateLimited,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EntityState {
    /// Zone-local wire id (protocol v10, networking rework 5 finding 1):
    /// small and monotonic, assigned by the server's `ReplIds` allocator on
    /// first reference — never raw hecs `Entity` bits. Stable for the
    /// entity's lifetime, never reused.
    pub id: u32,
    /// Prefab to spawn client-side for this entity.
    pub prefab: String,
    /// Quantized position (protocol v11) — see `WirePos`.
    pub pos: WirePos,
    /// Current health (v8) — cosmetic on the client (hit reacts, health bars).
    /// `None` means the entity has no `Health` component (protocol v12); a
    /// present reading may still be `Some(v)` with `v <= 0` momentarily
    /// (dying this tick) — `None` never conflates with "dead".
    pub hp: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EntityPos {
    /// Zone-local wire id (protocol v10) — see `EntityState::id`.
    pub id: u32,
    /// Quantized position (protocol v11) — see `WirePos`.
    pub pos: WirePos,
    /// Current health (v8); `None` = no `Health` component (protocol v12) —
    /// see `EntityState::hp`.
    pub hp: Option<i32>,
}

/// Quantization scale for `WirePos`: 256 units per meter (a 1/256 m quantum,
/// so rounding error is at most 1/512 m ≈ 2 mm per axis — two orders of
/// magnitude below the client's `TRUST_DISTANCE = 0.3` reconciliation band,
/// `client/net.rs:39`). Zigzag varints under postcard stay 1 byte near zero
/// and 3 bytes out to ±128 m, covering ±8_388 km end to end, so a per-zone
/// origin rebase buys nothing at current zone scales.
pub const POS_UNITS_PER_METER: f32 = 256.0;

/// A snapshot position, quantized to `1 / POS_UNITS_PER_METER` on the wire.
/// Rust-side code stays entirely in `Vec3` — the precision loss happens once,
/// at encode (protocol v11, networking rework 5 finding 2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WirePos(pub Vec3);

impl Serialize for WirePos {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let q = (
            (self.0.x * POS_UNITS_PER_METER).round() as i32,
            (self.0.y * POS_UNITS_PER_METER).round() as i32,
            (self.0.z * POS_UNITS_PER_METER).round() as i32,
        );
        q.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WirePos {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let (x, y, z) = <(i32, i32, i32)>::deserialize(deserializer)?;
        Ok(WirePos(Vec3::new(
            x as f32 / POS_UNITS_PER_METER,
            y as f32 / POS_UNITS_PER_METER,
            z as f32 / POS_UNITS_PER_METER,
        )))
    }
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
        let msg = ClientMsg::Login { name: "alice".into(), token: [7u8; 32] };
        let bytes = encode(&msg);
        match decode::<ClientMsg>(&bytes).unwrap() {
            ClientMsg::Login { name, token } => {
                assert_eq!(name, "alice");
                assert_eq!(token, [7u8; 32]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn login_denied_roundtrip() {
        let msg = ServerMsg::LoginDenied { reason: LoginDenyReason::RateLimited };
        let bytes = encode(&msg);
        match decode::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::LoginDenied { reason } => assert_eq!(reason, LoginDenyReason::RateLimited),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn server_msg_roundtrip() {
        let msg = ServerMsg::Snapshot {
            tick: 42,
            last_processed_seq: 17,
            enters: vec![
                EntityState { id: 9, prefab: "player".into(), pos: WirePos(Vec3::new(1.0, 0.0, -3.0)), hp: Some(100) },
                EntityState { id: 11, prefab: "bolt".into(), pos: WirePos(Vec3::ZERO), hp: None },
            ],
            leaves: vec![4],
            states: vec![
                EntityPos { id: 9, pos: WirePos(Vec3::new(1.0, 0.0, -3.0)), hp: Some(100) },
                EntityPos { id: 11, pos: WirePos(Vec3::ZERO), hp: None },
            ],
        };
        let bytes = encode(&msg);
        match decode::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::Snapshot { tick, last_processed_seq, enters, leaves, states } => {
                assert_eq!(tick, 42);
                assert_eq!(last_processed_seq, 17);
                assert_eq!(enters.len(), 2);
                assert_eq!(enters[0].prefab, "player");
                assert!((enters[0].pos.0 - Vec3::new(1.0, 0.0, -3.0)).length() < 1.0 / 256.0);
                assert_eq!(leaves, vec![4]);
                assert_eq!(states[0].id, 9);
                assert!((states[0].pos.0 - Vec3::new(1.0, 0.0, -3.0)).length() < 1.0 / 256.0);
                assert_eq!(states[0].hp, Some(100), "hp rides in every state (v8) as Some when Health exists");
                assert_eq!(states[1].hp, None, "a Health-less entity's hp is None (v12), not 0");
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

    #[test]
    fn snapshot_position_quantization_roundtrip() {
        // Awkward coordinates (negative, fractional) through the real
        // encode/decode path: quantization error must stay within half a
        // 1/256 m quantum (plus float slop) per axis.
        let awkward = Vec3::new(-37.123, 0.0, 81.987);
        let msg = ServerMsg::Snapshot {
            tick: 1,
            last_processed_seq: 0,
            enters: vec![EntityState { id: 1, prefab: "player".into(), pos: WirePos(awkward), hp: Some(100) }],
            leaves: vec![],
            states: vec![EntityPos { id: 1, pos: WirePos(awkward), hp: Some(100) }],
        };
        let bytes = encode(&msg);
        match decode::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::Snapshot { enters, states, .. } => {
                let tol = 1.0 / 512.0 + 1e-4;
                let e = enters[0].pos.0;
                assert!((e.x - awkward.x).abs() < tol, "x off by {}", (e.x - awkward.x).abs());
                assert!((e.y - awkward.y).abs() < tol, "y off by {}", (e.y - awkward.y).abs());
                assert!((e.z - awkward.z).abs() < tol, "z off by {}", (e.z - awkward.z).abs());
                let s = states[0].pos.0;
                assert!((s.x - awkward.x).abs() < tol);
                assert!((s.y - awkward.y).abs() < tol);
                assert!((s.z - awkward.z).abs() < tol);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn wirepos_entity_pos_encoding_is_compact() {
        // Raw f32 would cost id(u32 varint) + 3*4 bytes = 17 B for this id/pos.
        // Quantized zigzag varints must bring a single EntityPos under 12 B.
        let msg = EntityPos { id: 500, pos: WirePos(Vec3::new(12.34, 0.0, -7.89)), hp: Some(100) };
        let bytes = encode(&msg);
        assert!(bytes.len() <= 12, "EntityPos encoded to {} bytes, expected <= 12", bytes.len());
    }
}
