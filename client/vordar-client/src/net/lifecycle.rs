// Connection lifecycle: reconnect backoff, event dispatch, world teardown.
//
// A connection is driven on the Input tick: maybe_reconnect polls the backoff
// timer and dials if due, then NetReceiveSystem drains events and dispatches
// them (snapshot updates, redirects, disconnects). Unexpected disconnect or
// a zone Redirect trigger teardown_replicated_world to reset the AOI and
// prediction state, then reschedule a redial on a backoff-doubled cadence.
// LoginDenied stops the redial entirely: retrying with the same bad credential
// would only be denied again.

use super::apply;
use super::NetClientState;
use engine_app::scheduler::System;
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use engine_net::{ClientEvent, NetClient};
use glam::Vec3;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_protocol::{decode, encode, ClientMsg, ServerMsg, PROTOCOL_VERSION};

/// Initial wait before the first redial after an unexpected disconnect — an
/// ordinary blip (brief loss, a moment of server-side hiccup) clears fast.
const RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
/// Backoff cap so a genuinely dead server doesn't spin the network thread,
/// while still retrying at a steady cadence.
const RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(8);
/// How long a redial is given to resolve (Connected or Disconnected) before
/// the backoff timer is allowed to fire again — must clear engine-net's own
/// handshake timeout (`client::HANDSHAKE_TIMEOUT`, 5 s) with margin.
const RECONNECT_ATTEMPT_GRACE: Duration = Duration::from_secs(6);

/// Backoff before reconnect attempt `attempt` (1-indexed): doubles each
/// attempt, capped at `RECONNECT_MAX_BACKOFF`.
pub(super) fn reconnect_backoff(attempt: u32) -> Duration {
    let doublings = attempt.saturating_sub(1).min(8);
    RECONNECT_INITIAL_BACKOFF.saturating_mul(1u32 << doublings).min(RECONNECT_MAX_BACKOFF)
}

/// Reconnect-in-progress bookkeeping: which attempt is current, and when to
/// act next — either "redial now" (waiting out the backoff) or "give up
/// waiting on the in-flight redial and reconsider" (`RECONNECT_ATTEMPT_GRACE`
/// after issuing it). `Some` for as long as the connection is down; cleared
/// the moment `ClientEvent::Connected` fires again.
pub(super) struct Reconnect {
    pub(super) attempt: u32,
    pub(super) retry_at: Instant,
}

pub(super) struct NetReceiveSystem;

impl System for NetReceiveSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // A due redial happens on its own clock, independent of any event
        // arriving this tick.
        maybe_reconnect(resources);

        let events = {
            let state = resources.get_mut::<NetClientState>().unwrap();
            state.client.as_mut().map(|c| c.poll()).unwrap_or_default()
        };

        for event in events {
            match event {
                ClientEvent::Connected => {
                    // Identity first: the server spawns us and sends Welcome
                    // only after Login (loads the character's saved state).
                    let state = resources.get_mut::<NetClientState>().unwrap();
                    state.reconnect = None;
                    let name = state.user.clone();
                    let token = state.token;
                    if let Some(client) = &state.client {
                        client.send(encode(&ClientMsg::Login { name: name.clone(), token }));
                    }
                    log::info!("connected to server, logging in as '{name}'");
                }
                ClientEvent::Disconnected => handle_disconnected(world, resources),
                // A hard handshake rejection (e.g. version mismatch) — log it
                // distinctly from an ordinary drop; `Disconnected` still
                // follows and drives the existing teardown/reconnect path.
                ClientEvent::Rejected(reason) => log::error!("connection rejected by server: {reason}"),
                ClientEvent::Message(data) => match decode::<ServerMsg>(&data) {
                    Some(ServerMsg::Welcome { player_id }) => {
                        log::info!("welcome: our player id is {player_id}");
                        let state = resources.get_mut::<NetClientState>().unwrap();
                        state.own_id = Some(player_id);
                        // A re-Welcome means death + respawn: the pending
                        // intents and correction belong to the old body.
                        state.pending.clear();
                        state.correction = Vec3::ZERO;
                    }
                    Some(ServerMsg::PrefabTable { names }) => {
                        log::info!("prefab table received: {} prefabs", names.len());
                        resources.get_mut::<NetClientState>().unwrap().prefab_names = names;
                    }
                    Some(ServerMsg::AoiDelta { tick, enters, leaves }) => {
                        apply::apply_aoi_delta(world, resources, tick, enters, leaves);
                    }
                    Some(ServerMsg::Snapshot { tick, last_processed_seq, states }) => {
                        apply::apply_states(world, resources, tick, last_processed_seq, states);
                    }
                    Some(ServerMsg::MechanicScheduled {
                        telegraph_prefab, pos, radius, resolve_at_micros, duration_micros, ..
                    }) => {
                        crate::telegraph::spawn_telegraph(world, resources, &telegraph_prefab, pos, radius, resolve_at_micros, duration_micros);
                    }
                    Some(ServerMsg::HitResult { mechanic, hits }) => {
                        log::info!("mechanic {mechanic} hit {} entities", hits.len());
                    }
                    Some(ServerMsg::WorldClock { world_micros, at_server_micros }) => {
                        let wt = resources.get_mut::<crate::world_time::WorldTime>().unwrap();
                        wt.offset_micros = world_micros as i64 - at_server_micros as i64;
                        wt.synced = true;
                    }
                    Some(ServerMsg::EntityDied { id, pos }) => {
                        apply::handle_entity_died(world, resources, id, pos);
                    }
                    Some(ServerMsg::LoginDenied { reason }) => {
                        // Denials are messages, not kicks: the server leaves
                        // the connection open, so WE close it — same lesson
                        // as Redirect, since a server-side kick could outrace
                        // this frame. `login_denied` then stops
                        // `handle_disconnected` from scheduling a redial that
                        // would only be denied again with the same credential.
                        log::error!("login denied: {reason:?}");
                        resources.get_mut::<NetClientState>().unwrap().login_denied = true;
                        handle_disconnected(world, resources);
                    }
                    Some(ServerMsg::Redirect { zone, addr }) => {
                        // Zone transfer: WE close the old connection (dropping
                        // the NetClient) and start fresh at the new address.
                        // Remaining drained events belong to the old session.
                        handle_redirect(world, resources, &zone, addr);
                        break;
                    }
                    None => log::warn!("undecodable server message ({} bytes)", data.len()),
                },
            }
        }

    }
}

/// Despawns every replicated entity and telegraph visual and resets the
/// per-connection reconciliation state. Shared by a zone Redirect and an
/// unexpected disconnect: both leave the client needing a fresh AOI rebuild
/// off the next Welcome.
fn teardown_replicated_world(world: &mut World, resources: &mut Resources) {
    let telegraphs: Vec<hecs::Entity> = world.query::<(hecs::Entity, &crate::telegraph::TelegraphVisual)>().iter().map(|(e, _)| e).collect();
    let replicated: Vec<hecs::Entity> = resources.get::<NetClientState>().unwrap().entities.values().copied().collect();
    {
        let queue = resources.get_mut::<DespawnQueue>().unwrap();
        for entity in replicated.into_iter().chain(telegraphs) {
            queue.push(entity, None);
        }
    }

    let state = resources.get_mut::<NetClientState>().unwrap();
    state.entities.clear();
    state.own_id = None;
    state.pending.clear();
    state.correction = Vec3::ZERO;
    // A redirect/reconnect lands in a different zone with a different
    // PrefabLibrary; clearing here forces the fresh table off the new
    // connection's Welcome instead of resolving enters against the old
    // zone's indices.
    state.prefab_names.clear();
    // Fresh connection, fresh validation stream (per-connection on the server).
    state.seq = 0;
    // The new connection starts its own last-3 redundancy window — resending
    // the old connection's seqs would just be silently skipped server-side,
    // but there is no reason to carry them over.
    state.move_ring.clear();
    // The new connection's tick sequence starts over — comparing against the
    // old zone's ticks would drop every snapshot until it catches up.
    state.latest_state_tick = 0;
    // The playback cursor is meaningless against a new connection's ticks —
    // `None` hard-snaps it fresh off the new zone's first snapshot.
    state.playback = None;
    resources.get_mut::<crate::world_time::WorldTime>().unwrap().synced = false;
}

/// Tear down the old zone's replicated world and reconnect to the new one.
/// The fresh connection's Connected event re-triggers Login; the server
/// spawns us at the position the transfer (or login routing) persisted.
fn handle_redirect(world: &mut World, resources: &mut Resources, zone: &str, addr: SocketAddr) {
    log::info!("redirected to zone '{zone}' at {addr}");
    teardown_replicated_world(world, resources);

    let state = resources.get_mut::<NetClientState>().unwrap();
    state.server_addr = addr;
    // Any in-flight backoff belonged to the old zone's address.
    state.reconnect = None;
    // Dropping the old NetClient closes the QUIC connection — the server sees
    // a normal Disconnected (which finds no PlayerConn and does nothing). A
    // failed redial here falls into the same reconnect state machine an
    // unexpected drop uses, instead of crashing with the character already
    // persisted into the target zone.
    match NetClient::connect_with_latency(addr, PROTOCOL_VERSION, state.simulated_rtt) {
        Ok(client) => state.client = Some(client),
        Err(e) => {
            log::error!("net: failed to connect to zone '{zone}' at {addr}: {e} — retrying in the background");
            state.client = None;
            state.reconnect = Some(Reconnect { attempt: 1, retry_at: Instant::now() + reconnect_backoff(1) });
        }
    }
    // ZoneDressingSystem rebuilds the floor/portals for the new zone.
    if let Some(current) = resources.get_mut::<crate::presentation::CurrentZone>() {
        current.0 = zone.to_owned();
    }
}

/// An unexpected disconnect (server killed, brief network loss, redial
/// failure): tear down the replicated world exactly like a zone Redirect,
/// then schedule (or advance) a backoff-retried redial of the same address.
fn handle_disconnected(world: &mut World, resources: &mut Resources) {
    teardown_replicated_world(world, resources);
    let state = resources.get_mut::<NetClientState>().unwrap();
    state.client = None;
    if state.login_denied {
        log::warn!("net: not reconnecting — the last login was denied");
        state.reconnect = None;
        return;
    }
    let attempt = state.reconnect.as_ref().map_or(1, |r| r.attempt + 1);
    let backoff = reconnect_backoff(attempt);
    log::warn!("net: disconnected from server — reconnect attempt {attempt} in {backoff:?}");
    state.reconnect = Some(Reconnect { attempt, retry_at: Instant::now() + backoff });
}

/// Redials `state.server_addr` once the current backoff/grace window has
/// elapsed. Runs every Input tick regardless of which events (if any) were
/// just drained — a due retry has nothing to do with the last message
/// received.
fn maybe_reconnect(resources: &mut Resources) {
    let state = resources.get_mut::<NetClientState>().unwrap();
    if state.login_denied {
        return;
    }
    let Some(reconnect) = &state.reconnect else { return };
    if Instant::now() < reconnect.retry_at {
        return;
    }
    let attempt = reconnect.attempt;
    let addr = state.server_addr;
    let simulated_rtt = state.simulated_rtt;
    match NetClient::connect_with_latency(addr, PROTOCOL_VERSION, simulated_rtt) {
        Ok(client) => {
            log::info!("net: reconnect attempt {attempt} dialing {addr}");
            state.client = Some(client);
            // Give this attempt a chance to resolve (Connected clears
            // `reconnect`; Disconnected reschedules with the real backoff)
            // before the timer is allowed to fire again.
            state.reconnect = Some(Reconnect { attempt, retry_at: Instant::now() + RECONNECT_ATTEMPT_GRACE });
        }
        Err(e) => {
            let next = attempt + 1;
            log::warn!("net: reconnect attempt {attempt} failed to start: {e}");
            state.reconnect = Some(Reconnect { attempt: next, retry_at: Instant::now() + reconnect_backoff(next) });
        }
    }
}
