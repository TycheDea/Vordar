// Snapshot-apply: the two-lane contract for keeping replicated state current.
// AoiDelta rides the reliable stream (identity — prefab, enter/leave — sent
// once); Snapshot rides an unreliable, unordered datagram (current position +
// intent ack), so it is tick-guarded before any field is read. Both are
// dispatched from NetReceiveSystem's event loop.

use super::*;

/// The server's death signal (v8): burst + cosmetic corpse for the dying
/// entity. Snapshots stop mentioning it the same tick, so its local entity is
/// despawned here too instead of waiting for the AOI leave. Our own death is
/// burst-only — the server re-Welcomes us into a respawned entity.
pub(super) fn handle_entity_died(world: &mut World, resources: &mut Resources, id: u32, pos: Vec3) {
    let (entity, own) = {
        let state = resources.get_mut::<NetClientState>().unwrap();
        (state.entities.remove(&id), state.own_id == Some(id))
    };
    // Death burst at the server-authoritative position.
    let color = entity
        .and_then(|e| world.get::<&vordar_game::class::ClassId>(e).ok().map(|c| c.id.clone()))
        .map(|class| crate::vfx::class_tint(resources, &class))
        .unwrap_or(glam::Vec3::ONE);
    if let Some(sim) = resources.get_mut::<crate::vfx::ParticleSim>() {
        sim.burst(
            pos + Vec3::Y,
            color,
            crate::vfx::DEATH_COUNT,
            crate::vfx::DEATH_SPEED,
            crate::vfx::DEATH_SIZE,
        );
    }
    if own {
        return; // respawn arrives via re-Welcome; keep our entity
    }
    if let Some(entity) = entity {
        // Corpse for mesh characters, then remove the live entity.
        let corpse = {
            let transform = world.get::<&Transform>(entity).map(|t| Transform::clone(&t));
            let mesh = world
                .get::<&engine_core::components::RenderMesh>(entity)
                .map(|m| engine_core::components::RenderMesh::clone(&m));
            let clips = world
                .get::<&crate::locomotion::LocomotionClips>(entity)
                .map(|c| c.death.clone());
            match (transform, mesh, clips) {
                (Ok(t), Ok(m), Ok(death)) if !death.is_empty() => Some((t, m, death)),
                _ => None,
            }
        };
        if let Some((transform, mesh, death)) = corpse {
            crate::react::spawn_corpse(world, transform, mesh, &death);
        }
        resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
    }
}

/// Reliable-stream half of a snapshot (`ServerMsg::AoiDelta`, protocol v14,
/// networking rework 3 finding 4): entities entering or leaving the AOI.
/// Identity (prefab) is sent once here; `apply_states` keeps positions
/// current afterward. Stream ordering means this never needs a tick guard.
/// `tick` seeds an entering entity's `NetBuffer` (networking rework 4,
/// finding 1) so playback has a sample to hold at before the first real
/// `Snapshot` for it arrives.
pub(super) fn apply_aoi_delta(world: &mut World, resources: &mut Resources, tick: u64, enters: Vec<EntityState>, leaves: Vec<u32>) {
    // Take the map instead of cloning it — nothing below reads it through
    // NetClientState, and it is written back at the end of this function.
    // prefab_names is small (a handful of short strings) and cloned once per
    // delta — see ServerMsg::PrefabTable (protocol v13, networking rework
    // 5 finding 4).
    let (mut known, own_id, predict, prefab_names) = {
        let state = resources.get_mut::<NetClientState>().unwrap();
        (std::mem::take(&mut state.entities), state.own_id, state.predict, state.prefab_names.clone())
    };

    // Enters first, so a same-tick Snapshot's states can address the new entities.
    for enter in enters {
        if known.contains_key(&enter.id) {
            continue;
        }
        let is_own_predicted = predict && own_id == Some(enter.id);
        let Some(prefab_name) = prefab_names.get(enter.prefab as usize) else {
            log::error!("unresolvable prefab index {} in AOI enter (id {})", enter.prefab, enter.id);
            continue;
        };
        match spawn_prefab(prefab_name, enter.pos.0, &mut SpawnContext { world, resources }) {
            Ok(entity) => {
                // A predicted own player is moved by the simulation, not the buffer.
                if !is_own_predicted {
                    let _ = world.insert_one(entity, NetBuffer::seeded(tick, enter.pos.0));
                }
                // Seed replicated health (v8) so the hit-react watcher starts
                // from the server's value, not the prefab's. `None` (v12)
                // means the entity has no Health component — nothing to seed.
                if let Some(hp) = enter.hp {
                    if let Ok(mut health) = world.get::<&mut Health>(entity) {
                        health.current = hp;
                    }
                }
                known.insert(enter.id, entity);
            }
            Err(e) => log::error!("replicated spawn '{prefab_name}' failed: {e}"),
        }
    }

    // Entities that left our AOI (or despawned on the server).
    for id in leaves {
        if let Some(entity) = known.remove(&id) {
            resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
        }
    }

    resources.get_mut::<NetClientState>().unwrap().entities = known;
}

/// Datagram half of a snapshot (`ServerMsg::Snapshot`, protocol v14,
/// networking rework 3 finding 4): current position (+hp) of every entity in
/// the AOI, plus the intent ack. Datagrams can arrive out of order, so any
/// `tick` not strictly newer than the last one applied is dropped before any
/// field is read (ack included) — the tick guard is what makes an
/// unreliable, unordered lane safe to apply directly.
pub(super) fn apply_states(
    world: &mut World,
    resources: &mut Resources,
    tick: u64,
    last_processed_seq: u32,
    states: Vec<EntityPos>,
) {
    // Take the map instead of cloning it — nothing below reads it through
    // NetClientState, and it is written back at the end of this function.
    let (known, own_id, predict, cursor) = {
        let state = resources.get_mut::<NetClientState>().unwrap();
        if tick <= state.latest_state_tick {
            return;
        }
        state.latest_state_tick = tick;
        (std::mem::take(&mut state.entities), state.own_id, state.predict, state.playback)
    };

    // Own-player state is handled by reconciliation, which needs &mut World —
    // pull it out before the view below borrows the world.
    let own_state = match (predict, own_id) {
        (true, Some(own)) => states.iter().find(|s| s.id == own).map(|s| (own, s.pos.0)),
        _ => None,
    };

    // Replicated health (v8) — every state, own player included: the client
    // never simulates its own damage, so the snapshot is the only source.
    {
        let mut hp_q = world.query::<&mut Health>();
        let mut hp_view = hp_q.view();
        for state in &states {
            let Some(hp) = state.hp else { continue }; // None (v12): no Health component
            let Some(&entity) = known.get(&state.id) else { continue };
            if let Some(health) = hp_view.get_mut(entity) {
                health.current = hp;
            }
        }
    }

    // Positions land in each addressed entity's tick-indexed sample buffer;
    // NetInterpolateSystem renders Transform.position (and derives NetMotion
    // from the active segment's slope) at a fixed delay behind the newest
    // sample instead of restarting a lerp from wherever the entity is
    // currently displayed (networking rework 4, finding 1).
    {
        // One view for the whole batch instead of a world.get per entity.
        // Transform rides alongside NetBuffer so a dry-recovery synthetic
        // sample (networking rework 4, finding 2) can capture where the
        // entity is actually displayed before splicing in the real one.
        let mut buf_q = world.query::<(&mut NetBuffer, &Transform)>();
        let mut buf_view = buf_q.view();
        for state in &states {
            if own_state.is_some_and(|(own, _)| state.id == own) {
                continue;
            }
            let Some(&entity) = known.get(&state.id) else { continue };
            let Some((buffer, transform)) = buf_view.get_mut(entity) else { continue };
            // If this entity was extrapolating or holding (its buffer's
            // newest tick already behind the playback cursor), splice a
            // synthetic sample at the currently displayed position before
            // the real one so playback resumes by interpolating from where
            // the entity actually is instead of popping straight to the new
            // sample (networking rework 4, finding 2). `NetBuffer::push`
            // skips it if that tick wouldn't keep the ring strictly
            // increasing.
            if let Some(cursor) = cursor {
                if buffer.samples.back().is_some_and(|&(back_tick, _)| (back_tick as f64) < cursor) {
                    buffer.push(cursor.floor() as u64, transform.position);
                }
            }
            buffer.push(tick, state.pos.0);
        }
    }

    if let Some((own, server_pos)) = own_state {
        if let Some(&entity) = known.get(&own) {
            reconcile_own(world, resources, entity, server_pos, last_processed_seq);
        }
    }

    resources.get_mut::<NetClientState>().unwrap().entities = known;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::prediction::PendingIntent;
    use vordar_protocol::WirePos;

    const DT: f32 = 1.0 / 60.0;

    /// Networking rework 3, finding 4: `Snapshot` now rides an unreliable
    /// datagram, so a stale/reordered copy must never regress state. This
    /// drives the real `apply_states` receive path directly (no
    /// reimplemented logic, no network): a fresh snapshot at tick 20 puts a
    /// remote entity at P2, then a stale snapshot at tick 10 (a LOWER
    /// `last_processed_seq` too) tries to put it at P1. Without the tick
    /// guard, the remote entity's `NetBuffer` would regress to P1 and
    /// `reconcile_own` would re-run against the stale ack.
    #[test]
    fn apply_states_drops_a_stale_snapshot_tick() {
        let mut world = World::new();
        let mut resources = Resources::new();

        // A remote (non-own) replicated entity — the general states-apply path.
        let remote = world.spawn((Transform::new(Vec3::ZERO), NetBuffer::seeded(0, Vec3::ZERO)));
        // Our own predicted player — exercises reconcile_own in the same call.
        let own = world.spawn((Transform::new(Vec3::ZERO), Player { speed: 6.0 }));

        let mut entities = HashMap::new();
        entities.insert(1u32, remote);
        entities.insert(2u32, own);

        let mut state =
            NetClientState::new(None, "127.0.0.1:9".parse().unwrap(), "unit-test".into(), [0u8; 32], true, Duration::ZERO);
        state.own_id = Some(2);
        state.entities = entities;
        state.pending = VecDeque::from(vec![
            PendingIntent { seq: 48, dir: Vec2::X, dt: DT, leap: None },
            PendingIntent { seq: 49, dir: Vec2::X, dt: DT, leap: None },
        ]);
        resources.insert(state);

        let p2 = Vec3::new(5.0, 0.0, 0.0);
        apply_states(
            &mut world,
            &mut resources,
            20,
            50,
            vec![
                EntityPos { id: 1, pos: WirePos(p2), hp: None },
                EntityPos { id: 2, pos: WirePos(Vec3::ZERO), hp: None },
            ],
        );

        let newest_after_20 = world.get::<&NetBuffer>(remote).unwrap().samples.back().unwrap().1;
        assert!((newest_after_20 - p2).length() < 1e-6, "tick 20 must land at P2: {newest_after_20:?}");
        assert_eq!(
            resources.get::<NetClientState>().unwrap().pending.len(),
            0,
            "ack 50 must have trimmed both already-applied pending intents (seq 48/49 <= 50)"
        );

        // A new local intent sent AFTER the tick-20 snapshot was applied.
        resources
            .get_mut::<NetClientState>()
            .unwrap()
            .pending
            .push_back(PendingIntent { seq: 53, dir: Vec2::X, dt: DT, leap: None });

        // A stale, reordered datagram: lower tick, lower ack, wrong position.
        let p1 = Vec3::new(-5.0, 0.0, 0.0);
        apply_states(
            &mut world,
            &mut resources,
            10,
            5,
            vec![
                EntityPos { id: 1, pos: WirePos(p1), hp: None },
                EntityPos { id: 2, pos: WirePos(Vec3::ZERO), hp: None },
            ],
        );

        let newest_after_stale = world.get::<&NetBuffer>(remote).unwrap().samples.back().unwrap().1;
        assert!(
            (newest_after_stale - p2).length() < 1e-6,
            "stale snapshot must not move the buffer's newest sample off P2: {newest_after_stale:?}"
        );
        let pending_seqs: Vec<u32> =
            resources.get::<NetClientState>().unwrap().pending.iter().map(|p| p.seq).collect();
        assert_eq!(
            pending_seqs,
            vec![53],
            "the stale snapshot's ack must never be applied — pending must not be re-derived from it"
        );
    }
}
