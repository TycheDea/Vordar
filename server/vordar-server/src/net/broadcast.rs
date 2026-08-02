//! Per-connection AOI fan-out systems: snapshot positions on datagrams at SNAPSHOT_HZ
//! (staggered per STAGGER across all connections), identity enters/leaves on reliable stream;
//! states budget-throttled via round-robin rotation. DeathBroadcastSystem runs Phase::DespawnFlush
//! to emit EntityDied to all conns whose known set held the entity.

use engine_app::events::{EventBus, HealthDepleted};
use engine_app::scheduler::System;
use engine_core::components::{Health, Transform};
use engine_core::prefab::PrefabId;
use engine_core::spatial::SpatialGrid;
use engine_core::traits::Resources;
use engine_core::World;
use engine_net::ConnId;
use glam::Vec3;
use hecs::Entity;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use vordar_protocol::{encode, EntityPos, EntityState, ServerMsg, WirePos};

use super::{NetServerState, AOI_RADIUS, STAGGER};

pub const MAX_SNAPSHOT_STATES: usize = 64;
pub const NEAREST_GUARANTEED: usize = 32;
/// WAN budget for one encoded `Snapshot` datagram — steady-state crowds run
/// well under this; it pins headroom against the 64-state worst case.
const MAX_SNAPSHOT_BYTES: usize = 1200;

/// One AOI-gathered candidate before wire ids are assigned: entity, position,
/// health bucket (for wire encoding), and its AOI-test radius.
type AoiCandidate = (Entity, Vec3, Option<i32>, f32);

/// Pick which AOI entries get a position update this snapshot: everything if
/// the crowd fits the budget, else the `nearest` closest entries (by dist²,
/// id-tiebroken) plus a round-robin rotation over the rest. Returns selected
/// indices into `entries` and the advanced cursor. Pure — unit-tested.
pub(super) fn select_states(entries: &[(u32, f32)], cursor: usize, max: usize, nearest: usize) -> (Vec<usize>, usize) {
    if entries.len() <= max {
        return ((0..entries.len()).collect(), cursor);
    }
    let mut by_dist: Vec<usize> = (0..entries.len()).collect();
    by_dist.sort_by(|&a, &b| {
        entries[a].1.total_cmp(&entries[b].1).then(entries[a].0.cmp(&entries[b].0))
    });
    let mut selected: Vec<usize> = by_dist[..nearest].to_vec();
    let in_nearest: HashSet<usize> = selected.iter().copied().collect();
    // The rotation pool in stable id order, so the cursor sweeps the same
    // sequence between snapshots and every entity refreshes within
    // ceil(pool / budget) snapshots.
    let mut pool: Vec<usize> = (0..entries.len()).filter(|i| !in_nearest.contains(i)).collect();
    pool.sort_by_key(|&i| entries[i].0);
    let budget = max - nearest;
    for k in 0..budget {
        selected.push(pool[(cursor + k) % pool.len()]);
    }
    (selected, cursor + budget)
}

pub struct SnapshotBroadcastSystem {
    /// Per-run scratch, reused across runs: grid candidates, the dedupe set,
    /// and the id set swapped with each conn's `known` (no per-conn realloc).
    aoi_scratch: Vec<Entity>,
    seen: HashSet<Entity>,
    current_ids: HashSet<u32>,
}

impl Default for SnapshotBroadcastSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotBroadcastSystem {
    pub fn new() -> Self {
        Self { aoi_scratch: Vec::new(), seen: HashSet::new(), current_ids: HashSet::new() }
    }
}

impl System for SnapshotBroadcastSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let (tick, conn_players): (u64, Vec<(ConnId, Entity)>) = {
            let state = resources.expect_mut::<NetServerState>();
            state.tick += 1;
            // Periodic world-clock re-sync (every ~10 s at POST_HZ).
            if state.tick.is_multiple_of(600) {
                // Same cadence sweeps ReplIds: entities despawned since the
                // last sweep (bolts, dead enemies) stop holding a wire id.
                state.repl_ids.sweep(world);
                let at_server_micros = state.server.now_micros();
                let world_micros = state.world_at(at_server_micros);
                state.server.broadcast(encode(&ServerMsg::WorldClock { world_micros, at_server_micros }));

                // Periodic net metrics dump for operational visibility.
                let m = state.server.metrics();
                log::info!(
                    "net metrics: frames_in={} frames_out={} bytes_in={} bytes_out={} rejects={} writer_queue_depth={} busy_micros={} datagrams_in={} datagrams_out={} datagram_send_failures={} snapshot_bytes={}",
                    m.frames_in.load(Ordering::Relaxed),
                    m.frames_out.load(Ordering::Relaxed),
                    m.bytes_in.load(Ordering::Relaxed),
                    m.bytes_out.load(Ordering::Relaxed),
                    m.rejects.load(Ordering::Relaxed),
                    m.writer_queue_depth.load(Ordering::Relaxed),
                    m.busy_micros.load(Ordering::Relaxed),
                    m.datagrams_in.load(Ordering::Relaxed),
                    m.datagrams_out.load(Ordering::Relaxed),
                    m.datagram_send_failures.load(Ordering::Relaxed),
                    m.snapshot_bytes.load(Ordering::Relaxed),
                );
            }
            // Stagger: only this tick's slice of connections is served — each
            // conn still gets exactly SNAPSHOT_HZ snapshots per second.
            let tick = state.tick;
            let conns = state.conns.iter()
                .filter(|&(&conn, _)| conn % STAGGER == tick % STAGGER)
                .map(|(&conn, pc)| (conn, pc.entity))
                .collect();
            (tick, conns)
        };
        if conn_players.is_empty() {
            return;
        }

        // Per-client AOI: grid cells are coarse and multi-cell entities appear
        // more than once, so dedupe and apply the exact radius test — a fuzzy
        // border would make entities flap in and out between snapshots.
        let mut per_conn: Vec<(ConnId, Vec<AoiCandidate>)> = Vec::with_capacity(conn_players.len());
        {
            let grid = resources.expect::<SpatialGrid>();
            // One view for the whole gather: the replication filter (PrefabId),
            // position, and health come from a single lookup per candidate.
            let mut repl_q = world.query::<(&Transform, &PrefabId, Option<&Health>)>();
            let repl_view = repl_q.view();
            for &(conn, player) in &conn_players {
                let Ok(center) = world.get::<&Transform>(player).map(|t| t.position) else { continue };
                self.aoi_scratch.clear();
                grid.query_cells_overlapping_into(center, AOI_RADIUS, &mut self.aoi_scratch);
                self.seen.clear();
                let mut current: Vec<AoiCandidate> = Vec::with_capacity(self.aoi_scratch.len());
                for &entity in &self.aoi_scratch {
                    if !self.seen.insert(entity) {
                        continue;
                    }
                    let Some((t, _, hp)) = repl_view.get(entity) else { continue };
                    let dist_sq = t.position.distance_squared(center);
                    if dist_sq > AOI_RADIUS * AOI_RADIUS {
                        continue;
                    }
                    // None = no Health component — never flattened to 0,
                    // which would conflate "no Health" with "dead".
                    let hp = hp.map(|h| h.current);
                    current.push((entity, t.position, hp, dist_sq));
                }
                per_conn.push((conn, current));
            }
        }

        let state = resources.expect_mut::<NetServerState>();
        for (conn, current) in per_conn {
            // Resolve each AOI candidate's zone-local wire id (assigning a
            // fresh monotonic one on first reference) before touching this
            // connection's PlayerConn — done here, not in the gather block
            // above, because that block only holds an immutable SpatialGrid
            // borrow of `resources`, not the `&mut NetServerState` id_for needs.
            let ids: Vec<u32> = current.iter().map(|&(entity, ..)| state.repl_ids.id_for(entity)).collect();
            let current: Vec<(u32, Entity, Vec3, Option<i32>, f32)> = ids
                .into_iter()
                .zip(current)
                .map(|(id, (entity, pos, hp, dist_sq))| (id, entity, pos, hp, dist_sq))
                .collect();
            let Some(pc) = state.conns.get_mut(&conn) else { continue };
            let by_name = state.prefab_table.as_ref().map(|(_, by_name)| by_name);

            self.current_ids.clear();
            self.current_ids.extend(current.iter().map(|&(id, ..)| id));
            let leaves: Vec<u32> = pc.known.difference(&self.current_ids).copied().collect();
            let enters: Vec<EntityState> = current
                .iter()
                .filter(|(id, ..)| !pc.known.contains(id))
                .filter_map(|&(id, entity, pos, hp, _)| {
                    let prefab_name = world.get::<&PrefabId>(entity).ok()?.0.clone();
                    // A miss is unreachable in practice — spawn_prefab always
                    // attaches PrefabId from the same PrefabLibrary the table
                    // was built from — but skip rather than crash the whole
                    // snapshot over a content-bug edge case.
                    let prefab = match by_name.and_then(|m| m.get(&prefab_name)) {
                        Some(&idx) => idx,
                        None => {
                            log::error!("prefab '{prefab_name}' missing from the zone's prefab table");
                            return None;
                        }
                    };
                    Some(EntityState { id, prefab, pos: WirePos(pos), hp })
                })
                .collect();
            // Crowd throttling: only `states` is budgeted — identity (enters/
            // leaves/known) must track the full AOI or the diff corrupts.
            let entries: Vec<(u32, f32)> = current.iter().map(|&(id, _, _, _, d)| (id, d)).collect();
            let (selected, cursor) = select_states(&entries, pc.rr_cursor, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
            pc.rr_cursor = cursor;
            let states: Vec<EntityPos> = selected
                .into_iter()
                .map(|i| {
                    let (id, _, pos, hp, _) = current[i];
                    EntityPos { id, pos: WirePos(pos), hp }
                })
                .collect();
            // The old known set becomes next conn's current_ids scratch.
            std::mem::swap(&mut pc.known, &mut self.current_ids);

            // Identity delta rides the reliable stream (ordering with
            // PrefabTable/Welcome is what makes the diff protocol sound) and
            // only when non-empty — steady state then sends no stream
            // traffic at all.
            if !enters.is_empty() || !leaves.is_empty() {
                state.server.send(conn, encode(&ServerMsg::AoiDelta { tick, enters, leaves }));
            }
            // State update rides an unreliable datagram every snapshot
            // interval: a lost one is simply skipped, since the next cadence
            // supersedes it — this avoids head-of-line blocking a reliable
            // stream would otherwise impose.
            let last_processed_seq = pc.applied_seq;
            let snapshot_bytes = encode(&ServerMsg::Snapshot {
                tick,
                last_processed_seq,
                states,
            });
            debug_assert!(
                snapshot_bytes.len() <= MAX_SNAPSHOT_BYTES,
                "encoded snapshot exceeds WAN budget: {} > {MAX_SNAPSHOT_BYTES} bytes",
                snapshot_bytes.len(),
            );
            state.server.metrics().record_snapshot_bytes(snapshot_bytes.len());
            state.server.send_datagram(conn, snapshot_bytes);
        }
    }
}

/// Phase::DespawnFlush, First — after DeathSystem emitted the event
/// (CollisionResolve) but before the flush removes the entity, so its final
/// position is still readable. Snapshots stop mentioning the entity the same
/// tick; this message is the client's only death signal (corpse + burst).
/// Sent only to connections whose known set contains the entity.
pub(super) struct DeathBroadcastSystem;

impl System for DeathBroadcastSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let deaths: Vec<(Entity, Vec3)> = resources
            .get::<EventBus>()
            .map(|bus| {
                bus.read::<HealthDepleted>()
                    .filter_map(|e| {
                        let pos = world.get::<&Transform>(e.entity).ok()?.position;
                        Some((e.entity, pos))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if deaths.is_empty() {
            return;
        }
        let state = resources.expect_mut::<NetServerState>();
        for (entity, pos) in deaths {
            let id = state.repl_ids.id_for(entity);
            let msg = encode(&ServerMsg::EntityDied { id, pos });
            let targets: Vec<ConnId> = state
                .conns
                .iter()
                .filter(|(_, pc)| pc.known.contains(&id))
                .map(|(&conn, _)| conn)
                .collect();
            for conn in targets {
                state.server.send(conn, msg.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` entries with id = index and distance growing with the index.
    fn entries(n: usize) -> Vec<(u32, f32)> {
        (0..n).map(|i| (i as u32, i as f32)).collect()
    }

    #[test]
    fn small_crowds_pass_through_untouched() {
        let e = entries(MAX_SNAPSHOT_STATES);
        let (sel, cursor) = select_states(&e, 5, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
        assert_eq!(sel.len(), e.len());
        assert_eq!(cursor, 5);
    }

    #[test]
    fn nearest_always_included_over_budget() {
        let e = entries(200);
        for cursor in [0, 7, 1000] {
            let (sel, _) = select_states(&e, cursor, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
            assert_eq!(sel.len(), MAX_SNAPSHOT_STATES);
            for i in 0..NEAREST_GUARANTEED {
                assert!(sel.contains(&i), "nearest entry {i} missing at cursor {cursor}");
            }
        }
    }

    #[test]
    fn rotation_refreshes_every_entity() {
        let e = entries(200);
        let pool = e.len() - NEAREST_GUARANTEED; // 168
        let budget = MAX_SNAPSHOT_STATES - NEAREST_GUARANTEED; // 32
        let rounds = pool.div_ceil(budget); // ceil(168/32) = 6
        let mut cursor = 0;
        let mut seen: HashSet<usize> = HashSet::new();
        for _ in 0..rounds {
            let (sel, next) = select_states(&e, cursor, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
            seen.extend(sel);
            cursor = next;
        }
        // Every entry got at least one position update within the window.
        assert_eq!(seen.len(), e.len());
    }

    #[test]
    fn no_duplicate_indices_in_selection() {
        let e = entries(70); // barely over budget: pool of 38, budget 32
        let (sel, _) = select_states(&e, 31, MAX_SNAPSHOT_STATES, NEAREST_GUARANTEED);
        let unique: HashSet<usize> = sel.iter().copied().collect();
        assert_eq!(unique.len(), sel.len());
    }

    #[test]
    fn crowd_snapshot_gauge_reflects_a_full_budget_of_states() {
        use crate::db::DbWorker;
        use crate::net::PlayerConn;
        use engine_core::components::{CellOccupant, CollisionShape, Hitbox, Solid};
        use engine_net::NetServer;
        use engine_physics::cell_update::CellUpdateSystem;
        use std::collections::{HashMap, VecDeque};
        use std::time::Instant;
        use vordar_game::zones::ZoneDef;
        use vordar_protocol::PROTOCOL_VERSION;

        let worker = DbWorker::spawn(":memory:").unwrap();
        let server = NetServer::bind("127.0.0.1:0".parse().unwrap(), PROTOCOL_VERSION).unwrap();
        let directory = HashMap::from([("test".to_owned(), server.local_addr())]);
        let zone = ZoneDef { name: "test".into(), chapter: None, portals: Vec::new(), visuals: Default::default() };
        let mut state = NetServerState::new(server, worker.handle(), None, zone, directory, Instant::now());
        state.prefab_table = Some((std::sync::Arc::new(vec!["human".to_string()]), HashMap::from([("human".to_string(), 0u16)])));

        let mut world = World::new();
        let player = world.spawn((Transform::new(Vec3::ZERO),));
        state.conns.insert(1, PlayerConn {
            entity: player,
            name: "crowd-test".into(),
            token: [0u8; 32],
            queue: VecDeque::new(),
            applied_seq: 0,
            last_seq: 0,
            last_t: 0,
            cast_seq: 0,
            cast_t: 0,
            known: HashSet::new(),
            history: VecDeque::new(),
            cooldown_ready: HashMap::new(),
            rr_cursor: 0,
            carried_xp: 0,
        });

        // 100 entities packed inside AOI_RADIUS (40) — well over
        // MAX_SNAPSHOT_STATES, so the send site must throttle to 64.
        for i in 0..10 {
            for j in 0..10 {
                let pos = Vec3::new((i as f32 - 4.5) * 3.0, 0.0, (j as f32 - 4.5) * 3.0);
                world.spawn((
                    Transform::new(pos),
                    Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
                    CellOccupant { cells: Default::default() },
                    Solid,
                    PrefabId("human".into()),
                    Health::new(100),
                ));
            }
        }

        let mut resources = Resources::new();
        resources.insert(SpatialGrid::new(10.0));
        CellUpdateSystem::new().run(&mut world, &mut resources, 1.0 / 60.0);
        resources.insert(state);

        let mut sys = SnapshotBroadcastSystem::new();
        // Sweep a full stagger round so conn 1 is served regardless of STAGGER.
        for _ in 0..STAGGER {
            sys.run(&mut world, &mut resources, 1.0 / 60.0);
        }

        let gauge = resources.expect::<NetServerState>().server.metrics().snapshot_bytes.load(Ordering::Relaxed);
        assert!(gauge > 0, "snapshot gauge never recorded a send");
        assert!(
            gauge as usize >= MAX_SNAPSHOT_STATES * 8,
            "crowded snapshot only encoded to {gauge} bytes, expected a meaningful fraction of the {MAX_SNAPSHOT_BYTES} B budget"
        );
        assert!(
            (gauge as usize) <= MAX_SNAPSHOT_BYTES,
            "crowded snapshot {gauge} bytes exceeds the {MAX_SNAPSHOT_BYTES} B WAN budget"
        );
    }
}
