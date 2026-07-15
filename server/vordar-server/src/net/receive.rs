//! The Input-phase network edge: drains `ServerEvent`s once per Input tick,
//! validates and queues client intents, and completes async DB logins. A
//! connection enters the game only at DbLoaded-grant — anything it sends
//! before then is dropped. Exactly one queued intent applies per tick, so the
//! client's fixed-tick prediction replay matches the server bit-for-bit.

use crate::db::{CharacterRecord, DbLoaded, DbLoginOutcome};
use engine_app::events::EventBus;
use engine_app::scheduler::System;
use engine_core::components::{Health, Transform};
use engine_core::prefab::{spawn_prefab, PrefabLibrary};
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use engine_net::{ConnId, NetMetrics, ServerEvent};
use glam::{Vec2, Vec3};
use hecs::Entity;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use vordar_game::combat::leap::{leap_velocity, LeapImpulse};
use vordar_game::combat::projectile::spawn_projectile;
use vordar_game::combat::stats::DamageType;
use vordar_game::events::MoveIntent;
use vordar_game::player::class::{ClassId, ClassLibrary, DEFAULT_CLASS};
use vordar_game::skills::AbilityEffect;
use vordar_game::world::WorldTime;
use vordar_game::Mechanic;
use vordar_protocol::{decode, encode, AccountToken, ClientMsg, LoginDenyReason, MoveIntentEntry, ServerMsg};

use super::{aoi_conns, save_character, NetServerState, PlayerConn, HISTORY_CAP, MAX_REWIND_MICROS};

/// Slack on the arrival deadline for clock-sync error and jitter.
const ARRIVAL_MARGIN_MICROS: u64 = 100_000;
/// Intents may not be stamped further in the future than clock-sync error allows.
const FUTURE_SLACK_MICROS: u64 = 50_000;
/// Validated intents waiting to be applied (~250 ms of input). Jitter bursts
/// buffer here; beyond the cap the oldest are dropped (the client re-converges
/// via reconciliation). Flooding buys queue latency, never extra speed.
const INTENT_QUEUE_CAP: usize = 16;

/// What a character spawns as. The Ravager is the playable class while there
/// is no character-creation/class-picker; the "human" prefab and its kit
/// stay shipped and tested.
const PLAYER_PREFAB: &str = "ravager";

/// Spread spawn points so simultaneous joins don't stack on the origin.
fn spawn_position(conn: ConnId) -> Vec3 {
    let angle = (conn as f32) * (std::f32::consts::TAU / 8.0);
    Vec3::new(angle.cos() * 3.0, 0.0, angle.sin() * 3.0)
}

pub(super) struct NetReceiveSystem;

impl System for NetReceiveSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Cloned once up front: ClassLibrary is read-only content, and this
        // sidesteps holding an immutable Resources borrow across the event
        // loop's many `resources.get_mut::<NetServerState>()` calls below.
        let class_library = resources.get::<ClassLibrary>()
            .expect("ClassLibrary not in resources")
            .clone();

        // Publish the world clock for world systems (events, future schedules).
        let world_now = resources.get::<NetServerState>().unwrap().world_micros();
        resources.get_mut::<WorldTime>().unwrap().0 = world_now;

        let events = resources.get_mut::<NetServerState>().unwrap().server.poll();

        // Projectile casts accepted this tick — spawned after the event loop
        // releases the NetServerState borrow (spawn_projectile needs resources).
        let mut pending_bolts: Vec<(String, Vec3, Vec3, f32, i32, DamageType, f32, Entity)> = Vec::new();

        for event in events {
            match event {
                ServerEvent::Connected(conn) => {
                    // The connection isn't in the game until Login arrives:
                    // identity picks the character, the character picks the
                    // spawn (loaded position + health).
                    log::info!("conn {conn}: connected, awaiting login");
                }
                ServerEvent::Disconnected(conn) => {
                    handle_disconnect(world, resources, conn);
                }
                ServerEvent::Message { conn, data, recv_micros } => {
                    let Some(msg) = decode::<ClientMsg>(&data) else {
                        log::warn!("conn {conn}: undecodable message ({} bytes)", data.len());
                        continue;
                    };
                    match msg {
                        ClientMsg::Login { name, token } => handle_login(world, resources, conn, name, token),
                        ClientMsg::MoveIntents { intents } => {
                            let state = resources.get_mut::<NetServerState>().unwrap();
                            let rtt = state.server.rtt_micros(conn).unwrap_or(0);
                            let Some(pc) = state.conns.get_mut(&conn) else { continue };
                            queue_move_intents(pc, &intents, recv_micros, rtt, &state.server.metrics());
                        }
                        ClientMsg::CastIntent { seq, t_server_micros: t, skill: skill_id, target } => {
                            let state = resources.get_mut::<NetServerState>().unwrap();
                            let rtt = state.server.rtt_micros(conn).unwrap_or(0);
                            let Some(pc) = state.conns.get_mut(&conn) else { continue };
                            if let Err(reason) = validate_intent(pc, seq, t, recv_micros, rtt) {
                                log::warn!("conn {conn}: cast rejected ({reason})");
                                state.server.metrics().record_reject();
                                continue;
                            }
                            pc.last_seq = seq;
                            pc.last_t = t;
                            let caster = pc.entity;
                            let class_id = world.get::<&ClassId>(caster)
                                .map(|c| c.id.clone())
                                .unwrap_or_else(|_| DEFAULT_CLASS.to_owned());
                            let Some(def) = class_library.get(&class_id, &skill_id) else {
                                log::warn!("conn {conn}: unknown ability '{skill_id}' for class '{class_id}'");
                                continue;
                            };
                            let now = state.server.now_micros();
                            let on_cooldown = pc.cooldown_ready.get(&skill_id)
                                .is_some_and(|&ready_at| now < ready_at);
                            if on_cooldown {
                                log::debug!("conn {conn}: '{skill_id}' on cooldown");
                                continue;
                            }
                            let Ok(caster_pos) = world.get::<&Transform>(caster).map(|tr| tr.position) else {
                                continue;
                            };
                            let target = Vec3::new(target.x, 0.0, target.y);
                            if !target.is_finite() { continue; }
                            match &def.effect {
                                AbilityEffect::Scheduled { telegraph_prefab, radius, damage, damage_type, cast_micros, max_range } => {
                                    let (telegraph_prefab, radius, damage, damage_type, cast_micros, max_range) =
                                        (telegraph_prefab.clone(), *radius, *damage, *damage_type, *cast_micros, *max_range);
                                    if caster_pos.distance_squared(target) > max_range * max_range {
                                        log::debug!("conn {conn}: cast out of range");
                                        continue;
                                    }
                                    pc.cooldown_ready.insert(skill_id.clone(), now + def.cooldown_micros);
                                    state.next_mechanic_id += 1;
                                    let id = state.next_mechanic_id;
                                    // Schedule in ABSOLUTE server time and tell everyone the
                                    // same thing (DESIGN.md §3) — T = telegraph completion.
                                    let resolve_at_micros = now + cast_micros;
                                    world.spawn((
                                        Transform::new(target),
                                        Mechanic {
                                            id,
                                            radius,
                                            damage,
                                            damage_type,
                                            resolve_at_micros,
                                            caster,
                                        },
                                    ));
                                    let frame = encode(&ServerMsg::MechanicScheduled {
                                        id,
                                        telegraph_prefab,
                                        pos: target,
                                        radius,
                                        resolve_at_micros,
                                        duration_micros: cast_micros,
                                    });
                                    for c in aoi_conns(&state.conns, world, target) {
                                        state.server.send(c, frame.clone());
                                    }
                                    log::info!("conn {conn}: mechanic {id} ('{skill_id}') resolves at {resolve_at_micros}");
                                }
                                AbilityEffect::Projectile { prefab, speed, damage, damage_type, ttl_secs, spawn_offset } => {
                                    let (prefab, speed, damage, damage_type, ttl_secs, spawn_offset) =
                                        (prefab.clone(), *speed, *damage, *damage_type, *ttl_secs, *spawn_offset);
                                    // No range gate: the target only fixes the
                                    // flight direction; the projectile itself
                                    // is the range limit (speed × ttl).
                                    let mut dir = target - caster_pos;
                                    dir.y = 0.0;
                                    if dir.length_squared() < 1e-6 {
                                        continue; // degenerate aim at own feet
                                    }
                                    let dir = dir.normalize();
                                    pc.cooldown_ready.insert(skill_id.clone(), now + def.cooldown_micros);
                                    pending_bolts.push((
                                        prefab,
                                        caster_pos + dir * spawn_offset,
                                        dir,
                                        speed,
                                        damage,
                                        damage_type,
                                        ttl_secs,
                                        caster,
                                    ));
                                }
                                AbilityEffect::Leap { telegraph_prefab, radius, damage, damage_type, cast_micros, max_range } => {
                                    let (telegraph_prefab, radius, damage, damage_type, cast_micros, max_range) =
                                        (telegraph_prefab.clone(), *radius, *damage, *damage_type, *cast_micros, *max_range);
                                    if caster_pos.distance_squared(target) > max_range * max_range {
                                        log::debug!("conn {conn}: leap out of range");
                                        continue;
                                    }
                                    pc.cooldown_ready.insert(skill_id.clone(), now + def.cooldown_micros);
                                    state.next_mechanic_id += 1;
                                    let id = state.next_mechanic_id;
                                    // Same scheduling as Scheduled — the arrival hit test IS a
                                    // Mechanic — plus a dash whose countdown ends at the same
                                    // instant (both derived from cast_micros).
                                    let resolve_at_micros = now + cast_micros;
                                    let cast_secs = cast_micros as f32 / 1e6;
                                    world.spawn((
                                        Transform::new(target),
                                        Mechanic {
                                            id,
                                            radius,
                                            damage,
                                            damage_type,
                                            resolve_at_micros,
                                            caster,
                                        },
                                    ));
                                    let _ = world.insert_one(caster, LeapImpulse {
                                        velocity: leap_velocity(caster_pos, target, cast_secs),
                                        remaining: cast_secs,
                                    });
                                    let frame = encode(&ServerMsg::MechanicScheduled {
                                        id,
                                        telegraph_prefab,
                                        pos: target,
                                        radius,
                                        resolve_at_micros,
                                        duration_micros: cast_micros,
                                    });
                                    for c in aoi_conns(&state.conns, world, target) {
                                        state.server.send(c, frame.clone());
                                    }
                                    log::info!("conn {conn}: leap mechanic {id} ('{skill_id}') resolves at {resolve_at_micros}");
                                }
                            }
                        }
                    }
                }
            }
        }

        // Spawn the projectiles accepted above (player-fired: damages enemies).
        for (prefab, origin, dir, speed, damage, damage_type, ttl, caster) in pending_bolts {
            spawn_projectile(world, resources, &prefab, origin, dir, speed, damage, damage_type, ttl, caster, false);
        }

        // Finished character loads → spawn + Welcome (or a denial). The
        // connection enters the game only now; anything it sent earlier was
        // dropped by the PlayerConn guard.
        let loaded = resources.get_mut::<NetServerState>().unwrap().db.poll();
        for DbLoaded { conn, name, outcome } in loaded {
            // The in-flight login's presented token, captured either way —
            // a `Granted` record below seeds the new PlayerConn's token
            // without re-reading the wire.
            let Some((_, token)) = resources.get_mut::<NetServerState>().unwrap().loading.remove(&conn) else {
                continue; // disconnected while the load was in flight
            };
            let record = match outcome {
                DbLoginOutcome::Granted(record) => record,
                DbLoginOutcome::BadToken => {
                    log::warn!("conn {conn}: '{name}' login denied — token mismatch");
                    let state = resources.get_mut::<NetServerState>().unwrap();
                    // The conn may already have dropped while the DB
                    // roundtrip was in flight — peer_ip is then None, and
                    // there is nothing to record against.
                    if let Some(ip) = state.server.peer_ip(conn) {
                        let now = state.server.now_micros();
                        state.login_failures.record(ip, now);
                    }
                    state.server.send(conn, encode(&ServerMsg::LoginDenied { reason: LoginDenyReason::BadCredentials }));
                    continue;
                }
            };
            // Login routing: this zone serves only characters it owns. The
            // owner's address comes from the directory; the client closes
            // this connection and logs in there instead.
            {
                let state = resources.get_mut::<NetServerState>().unwrap();
                if record.zone != state.zone.name {
                    match state.directory.get(&record.zone) {
                        Some(&addr) => {
                            log::info!("conn {conn}: '{name}' belongs to zone '{}' — redirecting to {addr}", record.zone);
                            state.server.send(conn, encode(&ServerMsg::Redirect { zone: record.zone, addr }));
                        }
                        None => {
                            log::error!("conn {conn}: '{name}' in unknown zone '{}' — disconnecting", record.zone);
                            state.server.disconnect(conn);
                        }
                    }
                    continue;
                }
            }
            // This zone's prefab table is built lazily, once, on the first
            // grant reaching this point — by App-build time every chapter's
            // prefab dir has loaded, so PrefabLibrary is fully populated.
            // Read here, before spawn_prefab needs `resources` mutably below.
            let new_prefab_table: Option<Vec<String>> = {
                let has_table = resources.get::<NetServerState>().unwrap().prefab_table.is_some();
                if has_table {
                    None
                } else {
                    let library = resources.get::<PrefabLibrary>().expect("PrefabLibrary not in resources");
                    let names = library.names();
                    assert!(
                        names.len() <= u16::MAX as usize + 1,
                        "zone prefab count {} exceeds the u16 wire index space",
                        names.len()
                    );
                    Some(names)
                }
            };

            let result = spawn_prefab(PLAYER_PREFAB, record.pos, &mut SpawnContext { world, resources });
            let state = resources.get_mut::<NetServerState>().unwrap();
            if let Some(names) = new_prefab_table {
                let by_name: HashMap<String, u16> =
                    names.iter().cloned().enumerate().map(|(i, n)| (n, i as u16)).collect();
                state.prefab_table = Some((Arc::new(names), by_name));
            }
            match result {
                Ok(entity) => {
                    // The prefab is the source of truth for everything but
                    // the persisted fields; the DB overrides Health.current.
                    if let Ok(mut hp) = world.get::<&mut Health>(entity) {
                        hp.current = record.health;
                    }
                    // Cooldowns are persisted as remainders (`record.cooldowns`),
                    // so a relog or zone transfer restores the exact remaining
                    // cooldown instead of resetting every ability to full.
                    let spawn_now = state.server.now_micros();
                    let cooldown_ready: HashMap<String, u64> = record.cooldowns
                        .into_iter()
                        .map(|(id, remaining)| (id, spawn_now + remaining))
                        .collect();
                    state.conns.insert(conn, PlayerConn {
                        entity,
                        name: name.clone(),
                        token,
                        queue: VecDeque::new(),
                        applied_seq: 0,
                        last_seq: 0,
                        last_t: 0,
                        known: HashSet::new(),
                        history: VecDeque::new(),
                        cooldown_ready,
                        rr_cursor: 0,
                    });
                    let player_id = state.repl_ids.id_for(entity);
                    state.server.send(conn, encode(&ServerMsg::Welcome { player_id }));
                    // Prefab table right after Welcome, on the same ordered
                    // stream, so it always precedes the first Snapshot's
                    // enters. NOT resent on the respawn re-Welcome below —
                    // the connection keeps its table.
                    let names = (*state.prefab_table.as_ref().expect("prefab table built above").0).clone();
                    state.server.send(conn, encode(&ServerMsg::PrefabTable { names }));
                    let at_server_micros = state.server.now_micros();
                    let world_micros = state.world_at(at_server_micros);
                    state.server.send(conn, encode(&ServerMsg::WorldClock { world_micros, at_server_micros }));
                    log::info!("conn {conn}: '{name}' joined as {entity:?} ({} online)", state.conns.len());
                }
                Err(e) => log::error!("conn {conn}: player spawn failed: {e}"),
            }
        }

        // A connection must always own a live player: combat can kill the
        // entity, and there is no death/respawn design yet — so respawn at
        // the connection's spawn point and re-Welcome the client so
        // prediction and snapshots rebind to the new body.
        let dead: Vec<ConnId> = {
            let state = resources.get::<NetServerState>().unwrap();
            state.conns.iter()
                .filter(|&(_, pc)| !world.contains(pc.entity))
                .map(|(&conn, _)| conn)
                .collect()
        };
        for conn in dead {
            let result = spawn_prefab(PLAYER_PREFAB, spawn_position(conn), &mut SpawnContext { world, resources });
            let state = resources.get_mut::<NetServerState>().unwrap();
            let Some(pc) = state.conns.get_mut(&conn) else { continue };
            match result {
                Ok(entity) => {
                    pc.entity = entity;
                    pc.queue.clear();
                    let player_id = state.repl_ids.id_for(entity);
                    state.server.send(conn, encode(&ServerMsg::Welcome { player_id }));
                    log::info!("conn {conn}: player died — respawned as {entity:?}");
                }
                Err(e) => log::error!("conn {conn}: respawn failed: {e}"),
            }
        }

        // Apply exactly one queued intent per connection per tick for the
        // shared movement system. An empty queue (arrival jitter) means one
        // tick standing still — the position deficit stays accounted for in
        // the client's pending replay, so prediction error remains zero.
        let intents: Vec<(Entity, Vec2)> = {
            let state = resources.get_mut::<NetServerState>().unwrap();
            state.conns.values_mut()
                .filter_map(|pc| {
                    let (seq, stamp, dir) = pc.queue.pop_front()?;
                    pc.applied_seq = seq;
                    pc.history.push_back((stamp, dir));
                    if pc.history.len() > HISTORY_CAP {
                        pc.history.pop_front();
                    }
                    Some((pc.entity, dir))
                })
                .collect()
        };
        let bus = resources.get_mut::<EventBus>().unwrap();
        for (entity, dir) in intents {
            bus.emit(MoveIntent { entity, dir });
        }
    }
}

fn handle_disconnect(world: &mut World, resources: &mut Resources, conn: ConnId) {
    let state = resources.get_mut::<NetServerState>().unwrap();
    state.loading.remove(&conn);
    if let Some(pc) = state.conns.remove(&conn) {
        // Persist before queuing the despawn — DespawnFlush
        // runs later in the frame, the entity is still alive.
        save_character(world, state, &pc);
        resources.get_mut::<DespawnQueue>().unwrap().push(pc.entity, None);
        log::info!("conn {conn}: disconnected, despawning {:?}", pc.entity);
    }
}

/// Login arrives from a connection that has no `PlayerConn` yet; grant and
/// spawn happen only later, when the DB load completes.
fn handle_login(world: &mut World, resources: &mut Resources, conn: ConnId, name: String, token: AccountToken) {
    let state = resources.get_mut::<NetServerState>().unwrap();
    // Per-IP failed-login rate limit: resolved and
    // checked before anything else — an over-budget IP
    // is turned away without running credential
    // verification again. Successful logins are never
    // throttled; only the failures recorded below count
    // against the budget.
    let peer_ip = state.server.peer_ip(conn);
    let now = state.server.now_micros();
    if peer_ip.is_some_and(|ip| state.login_failures.is_limited(ip, now)) {
        log::warn!("conn {conn}: login denied — rate limited");
        state.server.send(conn, encode(&ServerMsg::LoginDenied { reason: LoginDenyReason::RateLimited }));
        return;
    }
    if name.len() > 32 || !name.chars().all(|c| c.is_ascii_graphic() && c != ' ') {
        log::warn!("conn {conn}: invalid login name");
        if let Some(ip) = peer_ip { state.login_failures.record(ip, now); }
        state.server.send(conn, encode(&ServerMsg::LoginDenied { reason: LoginDenyReason::BadCredentials }));
        return;
    }
    if state.conns.contains_key(&conn) || state.loading.contains_key(&conn) {
        log::debug!("conn {conn}: duplicate login ignored");
        return;
    }
    // Session takeover: the newest connection wins, but
    // ONLY when the presented token matches the connected
    // session's — a mismatch denies the NEW connection
    // without touching the victim, no DB roundtrip. A
    // bare name match would let anyone who knew a
    // character name hijack or kick its session. The old
    // one is usually a stale session — a closed client
    // whose QUIC close never arrived (process exit can
    // outrace the close frame) lingers until the idle
    // timeout, and ignoring the relogin until then would
    // leave the new client waiting forever for Welcome.
    let old = state.conns.iter()
        .find(|(_, pc)| pc.name == name)
        .map(|(&c, pc)| (c, pc.token));
    if let Some((old_conn, old_token)) = old {
        if old_token != token {
            log::warn!("conn {conn}: login as '{name}' denied — active session token mismatch");
            if let Some(ip) = peer_ip { state.login_failures.record(ip, now); }
            state.server.send(conn, encode(&ServerMsg::LoginDenied { reason: LoginDenyReason::BadCredentials }));
            return;
        }
        let pc = state.conns.remove(&old_conn).unwrap();
        // Same save-then-despawn as a real disconnect, so
        // the takeover load (FIFO behind it) restores the
        // freshest state.
        save_character(world, state, &pc);
        state.server.disconnect(old_conn);
        log::info!("conn {conn}: '{name}' takes over session from conn {old_conn}");
        resources.get_mut::<DespawnQueue>().unwrap().push(pc.entity, None);
    }
    let state = resources.get_mut::<NetServerState>().unwrap();
    // A same-name load still in flight belongs to another
    // stale connection — forget it (its DbLoaded result
    // gets discarded) and kick that connection too, but
    // again only on a token match; a mismatch denies the
    // NEW connection and leaves the in-flight login alone.
    let stale = state.loading.iter()
        .find(|(_, (n, _))| n == &name)
        .map(|(&c, &(_, t))| (c, t));
    if let Some((stale_conn, stale_token)) = stale {
        if stale_token != token {
            log::warn!("conn {conn}: login as '{name}' denied — in-flight login token mismatch");
            if let Some(ip) = peer_ip { state.login_failures.record(ip, now); }
            state.server.send(conn, encode(&ServerMsg::LoginDenied { reason: LoginDenyReason::BadCredentials }));
            return;
        }
        state.loading.remove(&stale_conn);
        state.server.disconnect(stale_conn);
    }
    log::info!("conn {conn}: login as '{name}', loading character");
    state.loading.insert(conn, (name.clone(), token));
    // Defaults seed a NEW character only: ring spawn +
    // the player prefab's full health (human.ron is
    // the source of truth; the DB merely overrides
    // Health.current after spawn).
    // (The zone field is decorative here: the schema
    // default puts every NEW character in 'start'.)
    let defaults = CharacterRecord {
        zone: "start".into(),
        pos: spawn_position(conn),
        health: 100,
        cooldowns: HashMap::new(),
    };
    state.db.login(conn, name, token, defaults);
}

/// Anti-cheat caps from DESIGN.md §3, in the protocol from v1.
fn validate_intent(pc: &PlayerConn, seq: u32, t: u64, recv_micros: u64, rtt: u64) -> Result<(), &'static str> {
    // seq=0 is PlayerConn::last_seq's "nothing received yet" sentinel, never a
    // value a genuine client sends (the client's own seq counter starts at 1)
    // — reject it outright first, so a spoofed/replayed seq=0 intent can't
    // hide behind the sentinel and pass monotonicity forever.
    if seq == 0 {
        return Err("stale seq");
    }
    // Monotonic, stream-consistent: replays and backdated contradictions are free rejects.
    if seq <= pc.last_seq {
        return Err("stale seq");
    }
    if t < pc.last_t {
        return Err("timestamp not monotonic");
    }
    // No future stamps beyond plausible clock-sync error.
    if t > recv_micros + FUTURE_SLACK_MICROS {
        return Err("timestamp in the future");
    }
    // Arrival deadline: an input claiming time T must arrive within ~one RTT
    // of T. MAX_REWIND acts as a floor while RTT estimates settle; the actual
    // lag-compensation rewind is capped separately.
    let max_age = rtt.max(MAX_REWIND_MICROS) + ARRIVAL_MARGIN_MICROS;
    if recv_micros.saturating_sub(t) > max_age {
        return Err("arrived past deadline");
    }
    Ok(())
}

/// Applies a `ClientMsg::MoveIntents` batch in order: the client resends up
/// to the last 3 intents each tick, so a lost datagram is fully recovered by
/// the next tick's batch. An entry whose `seq` this connection has already
/// seen (`seq <= pc.last_seq`) is expected redundancy — skipped silently, no
/// reject, no log — not a violation; only entries advancing `last_seq` run
/// the full `validate_intent` + dir-cap checks and enqueue exactly as the
/// old single-intent path did.
fn queue_move_intents(pc: &mut PlayerConn, entries: &[MoveIntentEntry], recv_micros: u64, rtt: u64, metrics: &NetMetrics) {
    for entry in entries {
        let MoveIntentEntry { seq, t_server_micros: t, dir } = *entry;
        // Redundant resend of an already-seen seq (the last-3 window
        // sliding forward, or a duplicate under reorder) — expected, not a
        // violation. validate_intent's own seq<=last_seq check would reject
        // this too, but doing it here keeps it silent: no metrics noise for
        // ordinary redundancy.
        if seq <= pc.last_seq {
            continue;
        }
        if let Err(reason) = validate_intent(pc, seq, t, recv_micros, rtt) {
            log::debug!("move intent rejected ({reason})");
            metrics.record_reject();
            continue;
        }
        pc.last_seq = seq;
        pc.last_t = t;
        // Max-speed validation: direction can never exceed unit length.
        // Reject only genuine violations (NaN/Inf, or well past unit
        // length); tolerate epsilon-scale float noise from the client's
        // f32 `normalize()` and clamp it — same rule as the shared
        // `movement_velocity` the client replays, so validation and
        // simulation agree instead of forking.
        if !dir.is_finite() || dir.length_squared() > 1.0 + 1e-3 { continue; }
        let dir = if dir.length_squared() > 1.0 { dir.normalize() } else { dir };
        pc.queue.push_back((seq, t, dir));
        if pc.queue.len() > INTENT_QUEUE_CAP {
            pc.queue.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// `last_seq: 0` is the connection's "nothing received yet" sentinel, so
    /// a naive `seq <= pc.last_seq` monotonicity check must not special-case
    /// seq==0 as "not yet checked" — a spoofed/replayed seq=0 intent must be
    /// rejected outright rather than passing validation every time. A
    /// genuine client's own seq counter starts at 1, so seq=0 should never be
    /// a legitimate value on the wire.
    #[test]
    fn zero_seq_is_always_rejected() {
        let mut world = World::new();
        let entity = world.spawn(());
        let pc = PlayerConn {
            entity,
            name: "victim".into(),
            token: [0u8; 32],
            queue: VecDeque::new(),
            applied_seq: 0,
            last_seq: 0,
            last_t: 0,
            known: HashSet::new(),
            history: VecDeque::new(),
            cooldown_ready: HashMap::new(),
            rr_cursor: 0,
        };
        // Otherwise-well-formed intent (monotonic t, arrives on time) —
        // the only thing wrong with it is seq == 0.
        let result = validate_intent(&pc, 0, 1_000, 1_000, 0);
        assert_eq!(result, Err("stale seq"), "seq=0 must never pass validation");
    }

    fn fresh_pc(entity: Entity) -> PlayerConn {
        PlayerConn {
            entity,
            name: "bot".into(),
            token: [0u8; 32],
            queue: VecDeque::new(),
            applied_seq: 0,
            last_seq: 0,
            last_t: 0,
            known: HashSet::new(),
            history: VecDeque::new(),
            cooldown_ready: HashMap::new(),
            rr_cursor: 0,
        }
    }

    /// A client resends up to the last 3 move intents every Input tick, so
    /// the server must treat an already-seen `seq` as expected redundancy —
    /// skipped silently, no reject, no re-queue — not a violation: running
    /// every entry through `validate_intent` unconditionally would trip its
    /// own `seq <= last_seq` rejection on ordinary resends and count them in
    /// `NetMetrics::rejects`.
    #[test]
    fn move_intents_dedupe_silently_without_rejecting() {
        let mut world = World::new();
        let entity = world.spawn(());
        let mut pc = fresh_pc(entity);
        let metrics = NetMetrics::new();
        let recv_micros = 1_000_000u64;
        // Stamps close behind recv_micros (well inside the arrival-deadline
        // margin) and monotonically increasing with seq — only redundancy
        // handling is under test here, not the deadline/monotonicity checks.
        let entry = |seq: u32| MoveIntentEntry { seq, t_server_micros: recv_micros - (8 - seq as u64) * 16_000, dir: Vec2::X };

        // First batch: [5, 6, 7] — all newer than last_seq=0, all queue.
        queue_move_intents(&mut pc, &[entry(5), entry(6), entry(7)], recv_micros, 0, &metrics);
        assert_eq!(pc.last_seq, 7, "last_seq must advance to the highest applied entry");
        assert_eq!(pc.queue.len(), 3, "all three entries in the first batch must queue");
        assert_eq!(metrics.rejects.load(Ordering::Relaxed), 0);

        // Second batch: [6, 7, 8] — 6 and 7 are redundant resends (the
        // last-3 window sliding forward), only 8 is genuinely new.
        queue_move_intents(&mut pc, &[entry(6), entry(7), entry(8)], recv_micros, 0, &metrics);
        assert_eq!(pc.last_seq, 8, "last_seq must advance to 8, the only genuinely new entry");
        assert_eq!(pc.queue.len(), 4, "only seq 8 is newly queued (3 from batch 1 + 1 from batch 2)");
        assert_eq!(
            metrics.rejects.load(Ordering::Relaxed), 0,
            "resending already-seen seqs 6/7 is expected redundancy, not a reject"
        );
    }

    /// A genuinely invalid entry inside a batch (future timestamp) must
    /// still reject through `NetMetrics::rejects` — the silent-skip rule
    /// applies only to already-seen `seq`s, never to a stamp violation.
    #[test]
    fn move_intents_still_rejects_a_genuinely_invalid_entry() {
        let mut world = World::new();
        let entity = world.spawn(());
        let mut pc = fresh_pc(entity);
        let metrics = NetMetrics::new();
        let recv_micros = 1_000_000u64;
        let good = MoveIntentEntry { seq: 1, t_server_micros: recv_micros, dir: Vec2::X };
        let future = MoveIntentEntry { seq: 2, t_server_micros: recv_micros + FUTURE_SLACK_MICROS + 1, dir: Vec2::X };

        queue_move_intents(&mut pc, &[good, future], recv_micros, 0, &metrics);

        assert_eq!(pc.last_seq, 1, "the future-stamped entry must not advance last_seq");
        assert_eq!(pc.queue.len(), 1, "only the valid entry queues");
        assert_eq!(
            metrics.rejects.load(Ordering::Relaxed), 1,
            "the future-stamped entry must still be counted as a reject"
        );
    }
}
