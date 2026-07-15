/// Benchmark seam (vordar-benches only): exposes just enough of the private
/// snapshot/mechanic machinery to measure it. Sends to the fabricated ConnIds
/// are silently dropped by engine-net's router (no such connection), so the
/// benches measure the full sim-thread cost with zero network I/O.
use super::*;
pub use super::broadcast::SnapshotBroadcastSystem;
pub use super::mechanics::MechanicResolveSystem;

pub const MAX_STATES: usize = broadcast::MAX_SNAPSHOT_STATES;
pub const NEAREST: usize = broadcast::NEAREST_GUARANTEED;
pub const AOI: f32 = AOI_RADIUS;
pub const STAGGER_TICKS: u64 = STAGGER;

pub fn select_states(
    entries: &[(u32, f32)],
    cursor: usize,
    max: usize,
    nearest: usize,
) -> (Vec<usize>, usize) {
    broadcast::select_states(entries, cursor, max, nearest)
}

/// NetServerState with one PlayerConn per entity, keyed by fabricated
/// ConnIds 1..=n.
pub fn state_with_fake_conns(server: NetServer, db: DbHandle, players: &[Entity]) -> NetServerState {
    let zone = ZoneDef { name: "bench".into(), chapter: None, portals: Vec::new(), visuals: Default::default() };
    let directory = HashMap::from([("bench".to_owned(), server.local_addr())]);
    let mut state = NetServerState::new(server, db, None, zone, directory, Instant::now());
    for (i, &entity) in players.iter().enumerate() {
        state.conns.insert(
            (i + 1) as ConnId,
            PlayerConn {
                entity,
                name: format!("bench-{i}"),
                token: [0u8; 32],
                queue: VecDeque::new(),
                applied_seq: 0,
                last_seq: 0,
                last_t: 0,
                known: HashSet::new(),
                history: VecDeque::new(),
                cooldown_ready: HashMap::new(),
                rr_cursor: 0,
            },
        );
    }
    state
}

/// Fill every conn's applied-intent history to HISTORY_CAP with stamps at
/// `stamp` — mechanic resolution then rewinds the full history per player
/// target (the worst case) whenever `stamp` exceeds the rewind horizon.
pub fn fill_histories(state: &mut NetServerState, stamp: u64) {
    for pc in state.conns.values_mut() {
        pc.history.clear();
        for k in 0..HISTORY_CAP {
            pc.history.push_back((stamp + k as u64, Vec2::X));
        }
    }
}
