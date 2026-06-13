// Shared e2e test harness: headless bot clients speaking the real protocol
// (engine-net directly, no renderer), plus small server-side test systems.
#![allow(dead_code)] // each test binary uses its own subset

use engine_app::scheduler::System;
use engine_core::prefab::queue_prefab_spawn;
use engine_core::traits::Resources;
use engine_core::World;
use engine_net::{ClientEvent, NetClient};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_protocol::{decode, encode, ClientMsg, ServerMsg, PROTOCOL_VERSION};

pub fn workspace_root() {
    // Prefabs load from content/ relative to cwd — run as if from workspace root.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(root).unwrap();
}

/// Fresh SQLite path in the temp dir for persistence tests.
pub fn temp_db(tag: &str) -> String {
    let path = std::env::temp_dir().join(format!("vordar-e2e-{tag}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path.to_str().unwrap().to_owned()
}

/// Distinct auto-names: a login for a name that is already online takes over
/// that session, so every bot in a multi-bot test needs its own character.
static NEXT_BOT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub struct Bot {
    pub client: NetClient,
    /// Character name sent as Login when the connection opens.
    pub name: String,
    pub player_id: Option<u64>,
    /// id → position, maintained from enter/leave/state messages
    pub last_snapshot: HashMap<u64, glam::Vec3>,
    /// id → prefab, learned from AOI enters
    pub prefabs: HashMap<u64, String>,
    pub seq: u32,
    /// last_processed_seq from the latest snapshot — the server's intent ack
    pub last_ack: u32,
    /// total bytes received in app messages (bandwidth measurement)
    pub bytes: usize,
    /// scheduled mechanics as (id, resolve_at_micros), in arrival order
    pub mechanics: Vec<(u64, u64)>,
    /// mechanic id → entity ids hit
    pub hit_results: HashMap<u64, Vec<u64>>,
    /// world − server clock offset from the latest WorldClock sample
    pub world_offset: Option<i64>,
    /// pending zone redirect as (zone, addr) — follow with `follow_redirect`
    pub redirect: Option<(String, SocketAddr)>,
    /// distinct snapshot ticks seen (snapshot-cadence measurement)
    pub snapshot_ticks: Vec<u64>,
    /// ids that appeared in the latest snapshot's `states` list
    pub last_states: Vec<u64>,
}

impl Bot {
    pub fn connect(addr: SocketAddr) -> Self {
        Self::connect_with_latency(addr, Duration::ZERO)
    }

    pub fn connect_with_latency(addr: SocketAddr, simulated_rtt: Duration) -> Self {
        let name = format!("bot-{}", NEXT_BOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        Self::connect_with_latency_as(addr, &name, simulated_rtt)
    }

    pub fn connect_as(addr: SocketAddr, name: &str) -> Self {
        Self::connect_with_latency_as(addr, name, Duration::ZERO)
    }

    pub fn connect_with_latency_as(addr: SocketAddr, name: &str, simulated_rtt: Duration) -> Self {
        let client = NetClient::connect_with_latency(addr, PROTOCOL_VERSION, simulated_rtt)
            .expect("connect failed");
        Self {
            client,
            name: name.to_owned(),
            player_id: None,
            last_snapshot: HashMap::new(),
            prefabs: HashMap::new(),
            seq: 0,
            last_ack: 0,
            bytes: 0,
            mechanics: Vec::new(),
            hit_results: HashMap::new(),
            world_offset: None,
            redirect: None,
            snapshot_ticks: Vec::new(),
            last_states: Vec::new(),
        }
    }

    /// Drop the current connection (the client closes — the server's
    /// Disconnected finds no PlayerConn after a transfer) and log in at the
    /// redirect target with the same character name.
    pub fn follow_redirect(&mut self) {
        let (_, addr) = self.redirect.take().expect("no redirect pending");
        let name = self.name.clone();
        *self = Bot::connect_as(addr, &name);
    }

    pub fn pump(&mut self) {
        for event in self.client.poll() {
            // Identity first: the server spawns the player and Welcomes only
            // after Login.
            if let ClientEvent::Connected = event {
                self.client.send(encode(&ClientMsg::Login { name: self.name.clone() }));
                continue;
            }
            if let ClientEvent::Message(data) = event {
                self.bytes += data.len();
                match decode::<ServerMsg>(&data) {
                    Some(ServerMsg::Welcome { player_id }) => self.player_id = Some(player_id),
                    Some(ServerMsg::Snapshot { tick, last_processed_seq, enters, leaves, states }) => {
                        self.last_ack = last_processed_seq;
                        if self.snapshot_ticks.last() != Some(&tick) {
                            self.snapshot_ticks.push(tick);
                        }
                        for e in enters {
                            self.last_snapshot.insert(e.id, e.pos);
                            self.prefabs.insert(e.id, e.prefab);
                        }
                        for id in leaves {
                            self.last_snapshot.remove(&id);
                            self.prefabs.remove(&id);
                        }
                        self.last_states = states.iter().map(|s| s.id).collect();
                        for s in states {
                            self.last_snapshot.insert(s.id, s.pos);
                        }
                    }
                    Some(ServerMsg::MechanicScheduled { id, resolve_at_micros, .. }) => {
                        self.mechanics.push((id, resolve_at_micros));
                    }
                    Some(ServerMsg::HitResult { mechanic, hits }) => {
                        self.hit_results.insert(mechanic, hits);
                    }
                    Some(ServerMsg::WorldClock { world_micros, at_server_micros }) => {
                        self.world_offset = Some(world_micros as i64 - at_server_micros as i64);
                    }
                    Some(ServerMsg::Redirect { zone, addr }) => {
                        self.redirect = Some((zone, addr));
                    }
                    None => panic!("undecodable server message"),
                }
            }
        }
    }

    pub fn wait_for(&mut self, what: &str, timeout: Duration, mut done: impl FnMut(&Bot) -> bool) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump();
            if done(self) { return; }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    /// Walk in `dir` until `arrived` or panic after `timeout`.
    pub fn walk_until(&mut self, what: &str, dir: glam::Vec2, timeout: Duration, mut arrived: impl FnMut(&Bot) -> bool) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.send_move(dir);
            self.pump();
            if arrived(self) {
                self.send_move(glam::Vec2::ZERO);
                return;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        panic!("timed out walking until {what}");
    }

    pub fn own_pos(&self) -> Option<glam::Vec3> {
        self.player_id.and_then(|id| self.last_snapshot.get(&id).copied())
    }

    pub fn send_move(&mut self, dir: glam::Vec2) {
        if let Some(t_server_micros) = self.client.server_now_micros() {
            self.seq += 1;
            self.client.send(encode(&ClientMsg::MoveIntent { seq: self.seq, t_server_micros, dir }));
        }
    }

    pub fn send_cast(&mut self, skill: &str, target: glam::Vec2) {
        if let Some(t_server_micros) = self.client.server_now_micros() {
            self.seq += 1;
            self.client.send(encode(&ClientMsg::CastIntent {
                seq: self.seq,
                t_server_micros,
                skill: skill.into(),
                target,
            }));
        }
    }
}

/// Pump for `dur` so in-flight intents/snapshots settle before reading state.
pub fn settle(bot: &mut Bot, dur: Duration) {
    let until = Instant::now() + dur;
    while Instant::now() < until {
        bot.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// One-shot world population, registered on the server App.
pub struct PopulateSystem {
    pub done: bool,
    pub positions: Vec<glam::Vec3>,
}

impl System for PopulateSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        if self.done {
            return;
        }
        self.done = true;
        for &pos in &self.positions {
            // "player" prefab as a stationary, harmless NPC stand-in: it has
            // Transform/Hitbox (so it's in the SpatialGrid) but no AI.
            queue_prefab_spawn(resources, "player", pos);
        }
    }
}
