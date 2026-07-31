use engine_net::{ClientEvent, Impairment, NetClient};
use glam::{Vec2, Vec3};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vordar_protocol::{decode, encode, AccountToken, ClientMsg, LoginDenyReason, MoveIntentEntry, ServerMsg, PROTOCOL_VERSION, TICK_HZ};

/// Wall-backstop multiplier for `SimDeadline`: must exceed the worst sim
/// slowdown the suite is expected to survive. Measured ~6x sim slowdown at
/// 3x CPU oversubscription (2026-07-17); 8x leaves headroom without masking
/// a genuine hang.
pub const WALL_BACKSTOP_FACTOR: u32 = 8;

/// Cap on `Bot::move_tokens`. Must stay below the server's `INTENT_QUEUE_CAP`
/// (16, `server/vordar-server/src/net/receive.rs`) so a full-bucket burst can
/// never overflow the queue and drop intents.
pub const MOVE_TOKEN_CAP: u32 = 12;

/// A wait budget expressed in sim ticks (observed through `Bot::latest_state_tick`,
/// which advances once per fixed server sim step at `TICK_HZ`), backstopped
/// by a wall-clock deadline at `WALL_BACKSTOP_FACTOR` times the budget. Lets
/// a wait survive CPU starvation that slows the sim without silently
/// shrinking every deadline in wall terms, while still catching a genuine
/// hang or a sim that never progresses at all.
pub struct SimDeadline {
    anchor: Option<u64>,
    budget_ticks: u64,
    wall_deadline: Instant,
}

impl SimDeadline {
    pub fn new(budget: Duration) -> Self {
        Self {
            anchor: None,
            budget_ticks: (budget.as_secs_f32() * TICK_HZ) as u64,
            wall_deadline: Instant::now() + budget * WALL_BACKSTOP_FACTOR,
        }
    }

    /// Anchors on the bot's first nonzero tick — a bot's counter reads 0
    /// until its first snapshot, and each zone server mints its own tick
    /// epoch, so anchoring at construction would either never expire (0
    /// forever) or expire instantly (foreign epoch). A deadline that spans a
    /// reconnect to a different server keeps its anchor from the old epoch;
    /// `saturating_sub` then holds the sim check inert (never expiring) once
    /// the fresh bot's ticks read below that stale anchor, so only the wall
    /// backstop can fire for a reconnect-spanning wait.
    fn sim_expired(&mut self, bot: &Bot) -> bool {
        if self.anchor.is_none() && bot.latest_state_tick > 0 {
            self.anchor = Some(bot.latest_state_tick);
        }
        self.anchor.is_some_and(|anchor| bot.latest_state_tick.saturating_sub(anchor) > self.budget_ticks)
    }

    fn wall_expired(&self) -> bool {
        Instant::now() >= self.wall_deadline
    }

    /// Panics with a distinct message for each budget: "sim budget
    /// exhausted" marks a behavioral failure (the sim ran, the condition
    /// never held); "wall backstop exceeded" marks a hang or a sim that
    /// never progressed.
    pub fn check(&mut self, bot: &Bot, what: &str) {
        if self.sim_expired(bot) {
            panic!("sim budget exhausted waiting for {what}");
        }
        if self.wall_expired() {
            panic!("wall backstop exceeded waiting for {what}");
        }
    }
}

/// Steers straight toward `target`'s XZ (recomputed every tick) until within
/// `arrive_radius`, or panics after the sim/wall budget. A straight line is
/// not always a walkable line — the server's `SeparationSystem` collides a
/// Solid player against every Solid + Anchored static (chapter03's buildings
/// included), so a target on the far side of a building row needs a caller
/// that chains waypoints around it, same as any other walk.
pub fn walk_to(bot: &mut Bot, target: Vec3, arrive_radius: f32, timeout: Duration) {
    let mut deadline = SimDeadline::new(timeout);
    loop {
        bot.pump();
        if let Some(pos) = bot.own_pos() {
            let d = target - pos;
            let dir = Vec2::new(d.x, d.z);
            if dir.length() <= arrive_radius {
                bot.send_move(Vec2::ZERO);
                return;
            }
            bot.send_move(dir.normalize());
        }
        deadline.check(bot, "a waypoint");
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Steer toward `portal` (spawn points sit on a ring, so a straight east
/// walk can miss the 2-unit radius) until the server redirects us.
pub fn walk_into_portal(bot: &mut Bot, portal: Vec3, timeout: Duration) {
    let mut deadline = SimDeadline::new(timeout);
    loop {
        bot.pump();
        if bot.redirect.is_some() {
            return;
        }
        if let Some(pos) = bot.own_pos() {
            let d = portal - pos;
            let dir = Vec2::new(d.x, d.z);
            if dir.length_squared() > 1e-6 {
                bot.send_move(dir.normalize());
            }
        }
        deadline.check(bot, "the portal");
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Distinct auto-names: a login for a name that is already online takes over
/// that session, so every bot in a multi-bot test needs its own character.
static NEXT_BOT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Deterministic account token for `name`: name bytes zero-padded/truncated
/// into the 32-byte token. Every bot derives its token this way, so two bots
/// sharing a name (same-name tests: `login_takeover`, reconnect kicks)
/// automatically share credentials too — token-gated takeover keeps working
/// for them unchanged.
pub fn name_token(name: &str) -> AccountToken {
    let mut token = [0u8; 32];
    let bytes = name.as_bytes();
    let n = bytes.len().min(32);
    token[..n].copy_from_slice(&bytes[..n]);
    token
}

pub struct Bot {
    pub client: NetClient,
    /// Character name sent as Login when the connection opens.
    pub name: String,
    /// Account token sent alongside `name` — `name_token(&name)`.
    pub token: AccountToken,
    pub player_id: Option<u32>,
    /// id → position, maintained from enter/leave/state messages
    pub last_snapshot: HashMap<u32, glam::Vec3>,
    /// id → prefab name, learned from AOI enters — indices arriving in
    /// `EntityState::prefab` are resolved through `prefab_names` in `pump`,
    /// so every existing name-based assertion keeps working unchanged.
    pub prefabs: HashMap<u32, String>,
    /// This zone's prefab name table (`ServerMsg::PrefabTable`), received
    /// once per connection right after `Welcome`.
    pub prefab_names: Vec<String>,
    pub seq: u32,
    /// Last 3 sent `MoveIntentEntry`s, oldest first — resent every tick as
    /// the `ClientMsg::MoveIntents` batch (last-3 redundancy). Cleared
    /// implicitly on reconnect: `follow_redirect` and the `connect_*`
    /// constructors always build a fresh `Bot`.
    pub move_ring: VecDeque<MoveIntentEntry>,
    /// last_processed_seq from the latest snapshot — the server's intent ack
    pub last_ack: u32,
    /// total bytes received in app messages (bandwidth measurement)
    pub bytes: usize,
    /// per-frame byte size of every `Snapshot` message received, in arrival
    /// order — the crowd-snapshot size gate reads this.
    pub snapshot_bytes: Vec<usize>,
    /// scheduled mechanics as (id, resolve_at_micros), in arrival order
    pub mechanics: Vec<(u64, u64)>,
    /// mechanic id → entity ids hit
    pub hit_results: HashMap<u64, Vec<u32>>,
    /// world − server clock offset from the latest WorldClock sample
    pub world_offset: Option<i64>,
    /// pending zone redirect as (zone, addr) — follow with `follow_redirect`
    pub redirect: Option<(String, SocketAddr)>,
    /// distinct snapshot ticks seen (snapshot-cadence measurement)
    pub snapshot_ticks: Vec<u64>,
    /// pump-time arrival stamp of every snapshot (inter-snapshot gap
    /// measurement for the loss probes)
    pub snapshot_at: Vec<Instant>,
    /// ids that appeared in the latest snapshot's `states` list
    pub last_states: Vec<u32>,
    /// latest replicated hp per entity
    pub last_hp: HashMap<u32, i32>,
    /// EntityDied messages as (id, pos), in arrival order
    pub deaths: Vec<(u32, glam::Vec3)>,
    /// Set once `ClientEvent::Disconnected` is observed — proves a
    /// server-side shutdown actually closed the wire.
    pub disconnected: bool,
    /// Latest `LoginDenied` reason received, if any.
    pub denied: Option<LoginDenyReason>,
    /// Highest `ServerMsg::Snapshot.tick` applied so far — mirrors the
    /// client's tick guard: `Snapshot` rides an unreliable datagram, so a
    /// stale/reordered copy must be dropped before any field is read (ack
    /// included).
    pub latest_state_tick: u64,
    /// `send_move` credit, funded by observed sim-tick advance in `pump`'s
    /// Snapshot arm — see `MOVE_TOKEN_CAP`.
    pub move_tokens: u32,
}

impl Bot {
    fn new(client: NetClient, name: String, token: AccountToken) -> Self {
        Self {
            client,
            name,
            token,
            player_id: None,
            last_snapshot: HashMap::new(),
            prefabs: HashMap::new(),
            prefab_names: Vec::new(),
            seq: 0,
            move_ring: VecDeque::new(),
            last_ack: 0,
            bytes: 0,
            snapshot_bytes: Vec::new(),
            mechanics: Vec::new(),
            hit_results: HashMap::new(),
            world_offset: None,
            redirect: None,
            snapshot_ticks: Vec::new(),
            snapshot_at: Vec::new(),
            last_states: Vec::new(),
            last_hp: HashMap::new(),
            deaths: Vec::new(),
            disconnected: false,
            denied: None,
            latest_state_tick: 0,
            move_tokens: 0,
        }
    }

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

    /// Like `connect_as`, but returns `None` on a failed dial instead of
    /// panicking: a test polling a rebinding address across the
    /// supervisor's teardown-then-rebuild window expects some attempts to
    /// fail before the new listener is up, not to end the test.
    /// `NetClient::connect_impaired` itself only
    /// reports a synchronous thread-spawn failure — a dial rejected because
    /// the old listener is mid-teardown (or refused because no listener is
    /// up yet) surfaces later as a `ClientEvent::Disconnected`, not as an
    /// `Err` here — so this also pumps briefly for that real signal before
    /// handing the bot back, instead of treating "thread spawned" as
    /// "connected".
    pub fn try_connect_as(addr: SocketAddr, name: &str) -> Option<Self> {
        let mut client = NetClient::connect_impaired(addr, PROTOCOL_VERSION, Impairment::default()).ok()?;
        // Wait for the real, low-level signal instead of `connect_impaired`'s
        // immediate `Ok`: `ClientEvent::Connected` means the handshake
        // actually completed; `Disconnected` (or nothing within the window —
        // dialing a port with no listener bound yet, mid-rebuild, does not
        // necessarily produce a prompt `Disconnected`) means this attempt
        // must be treated as failed so the caller retries with a fresh dial.
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut connected = false;
        'wait: while Instant::now() < deadline {
            for event in client.poll() {
                match event {
                    ClientEvent::Connected => {
                        connected = true;
                        break 'wait;
                    }
                    ClientEvent::Disconnected => return None,
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if !connected {
            return None;
        }
        // Same identity step `pump` performs on `Connected` — this event was
        // consumed above, so it will never reach `pump` for this bot.
        let token = name_token(name);
        client.send(encode(&ClientMsg::Login { name: name.to_owned(), token }));
        Some(Self::new(client, name.to_owned(), token))
    }

    pub fn connect_with_latency_as(addr: SocketAddr, name: &str, simulated_rtt: Duration) -> Self {
        Self::connect_impaired_as(addr, name, simulated_rtt, 0.0)
    }

    /// Latency plus receive-side (server→client) datagram loss below QUIC
    /// (see engine-net's `connect_impaired`) — the downstream loss-probe
    /// constructor.
    pub fn connect_impaired_as(addr: SocketAddr, name: &str, simulated_rtt: Duration, loss: f32) -> Self {
        Self::connect_full_as(
            addr,
            name,
            Impairment { rtt: simulated_rtt, downstream_loss: loss, ..Default::default() },
        )
    }

    /// Latency plus send-side (client→server) datagram loss below QUIC — the
    /// upstream loss-probe constructor.
    pub fn connect_upstream_impaired_as(addr: SocketAddr, name: &str, simulated_rtt: Duration, loss: f32) -> Self {
        Self::connect_full_as(
            addr,
            name,
            Impairment { rtt: simulated_rtt, upstream_loss: loss, ..Default::default() },
        )
    }

    /// General constructor for the full network conditioner (latency, both-
    /// direction loss, jitter/reorder, clock skew — see `Impairment`).
    pub fn connect_full_as(addr: SocketAddr, name: &str, impairment: Impairment) -> Self {
        let client = NetClient::connect_impaired(addr, PROTOCOL_VERSION, impairment).expect("connect failed");
        Self::new(client, name.to_owned(), name_token(name))
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
                self.client.send(encode(&ClientMsg::Login { name: self.name.clone(), token: self.token }));
                continue;
            }
            if let ClientEvent::Disconnected = event {
                self.disconnected = true;
                continue;
            }
            if let ClientEvent::Message(data) = event {
                self.bytes += data.len();
                match decode::<ServerMsg>(&data) {
                    Some(ServerMsg::Welcome { player_id }) => self.player_id = Some(player_id),
                    Some(ServerMsg::PrefabTable { names }) => self.prefab_names = names,
                    // Reliable-stream identity delta: AOI enters/leaves, sent
                    // only when non-empty. Stream ordering means no tick
                    // guard is needed.
                    Some(ServerMsg::AoiDelta { enters, leaves, .. }) => {
                        for e in enters {
                            self.last_snapshot.insert(e.id, e.pos.0);
                            // None = no Health component — record only real
                            // readings, never a stand-in 0.
                            if let Some(hp) = e.hp {
                                self.last_hp.insert(e.id, hp);
                            }
                            // Resolve the u16 wire index through the table.
                            // A miss panics rather than silently dropping the
                            // entity: test hygiene — it also proves the
                            // table always arrives before any enter that
                            // references it (stream ordering).
                            let prefab = self.prefab_names.get(e.prefab as usize).unwrap_or_else(|| {
                                panic!("unresolvable prefab index {} (id {}) — table has {} entries; \
                                        did the enter arrive before PrefabTable?", e.prefab, e.id, self.prefab_names.len())
                            });
                            self.prefabs.insert(e.id, prefab.clone());
                        }
                        for id in leaves {
                            self.last_snapshot.remove(&id);
                            self.last_hp.remove(&id);
                            self.prefabs.remove(&id);
                        }
                    }
                    // Datagram state update: a stale/reordered copy is
                    // dropped before any field is read (ack included) —
                    // mirrors the client's `apply_states` tick guard.
                    Some(ServerMsg::Snapshot { tick, last_processed_seq, states }) => {
                        if tick <= self.latest_state_tick {
                            continue;
                        }
                        // The server's tick counter is an epoch, not a delta:
                        // a prev of 0 means this is the first snapshot, so
                        // seed a full bucket rather than crediting a huge
                        // one-off delta. Otherwise credit the tick delta
                        // (not a flat 1) so a lost snapshot datagram still
                        // funds the ticks it covered.
                        let prev = self.latest_state_tick;
                        self.latest_state_tick = tick;
                        self.move_tokens = if prev == 0 {
                            MOVE_TOKEN_CAP
                        } else {
                            (self.move_tokens + (tick - prev) as u32).min(MOVE_TOKEN_CAP)
                        };
                        self.last_ack = last_processed_seq;
                        if self.snapshot_ticks.last() != Some(&tick) {
                            self.snapshot_ticks.push(tick);
                        }
                        self.snapshot_at.push(Instant::now());
                        self.snapshot_bytes.push(data.len());
                        self.last_states = states.iter().map(|s| s.id).collect();
                        for s in states {
                            self.last_snapshot.insert(s.id, s.pos.0);
                            if let Some(hp) = s.hp {
                                self.last_hp.insert(s.id, hp);
                            }
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
                    Some(ServerMsg::EntityDied { id, pos }) => {
                        self.deaths.push((id, pos));
                    }
                    Some(ServerMsg::LoginDenied { reason }) => {
                        self.denied = Some(reason);
                    }
                    None => panic!("undecodable server message"),
                }
            }
        }
    }

    pub fn wait_for(&mut self, what: &str, timeout: Duration, mut done: impl FnMut(&Bot) -> bool) {
        let mut deadline = SimDeadline::new(timeout);
        loop {
            self.pump();
            if done(self) { return; }
            deadline.check(self, what);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Walk in `dir` until `arrived` or panic after the sim/wall budget.
    pub fn walk_until(&mut self, what: &str, dir: glam::Vec2, timeout: Duration, mut arrived: impl FnMut(&Bot) -> bool) {
        let mut deadline = SimDeadline::new(timeout);
        loop {
            self.send_move(dir);
            self.pump();
            if arrived(self) {
                self.send_move(glam::Vec2::ZERO);
                return;
            }
            deadline.check(self, what);
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    pub fn own_pos(&self) -> Option<glam::Vec3> {
        self.player_id.and_then(|id| self.last_snapshot.get(&id).copied())
    }

    pub fn send_move(&mut self, dir: glam::Vec2) {
        // A suppressed send — even a stop intent — is safe because the
        // server stands still on an empty queue (receive.rs's drain_intents);
        // over-sending, which pins the queue full and delays every command
        // including stop, is the only hazard.
        if self.move_tokens == 0 {
            return;
        }
        if let Some(t_server_micros) = self.client.server_now_micros() {
            self.move_tokens -= 1;
            self.seq += 1;
            // Last-3 redundancy: mirrors NetSendInputSystem's ring buffer —
            // this tick's entry plus the two previous, sent via datagram.
            self.move_ring.push_back(MoveIntentEntry { seq: self.seq, t_server_micros, dir });
            if self.move_ring.len() > 3 {
                self.move_ring.pop_front();
            }
            let intents: Vec<MoveIntentEntry> = self.move_ring.iter().cloned().collect();
            self.client.send_datagram(encode(&ClientMsg::MoveIntents { intents }));
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

/// Pump until `dur`-worth of sim ticks elapsed (same anchor rule as
/// `SimDeadline`) so in-flight intents/snapshots settle before reading
/// state. Returns — never panics — at the wall backstop too: settling is
/// best-effort bookkeeping, not a behavioral assertion.
pub fn settle(bot: &mut Bot, dur: Duration) {
    let mut deadline = SimDeadline::new(dur);
    loop {
        bot.pump();
        if deadline.sim_expired(bot) || deadline.wall_expired() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Raw (non-`Bot`) login probe: dials `addr` directly, sends `Login{name,
/// token}`, and waits up to `timeout` for either `LoginDenied` or `Welcome` —
/// the attacker/prober shape shared by the credential-mismatch and
/// rate-limit tests. Asserts a `Welcome` never arrives; `on_tick` runs once
/// per poll cycle so a caller can keep an unrelated victim bot pumped
/// alongside the probe.
pub fn raw_login_probe(
    addr: SocketAddr,
    name: &str,
    token: AccountToken,
    timeout: Duration,
    mut on_tick: impl FnMut(),
) -> LoginDenyReason {
    let mut attacker = NetClient::connect(addr, PROTOCOL_VERSION).expect("attacker connect");
    let mut denied = None;
    let mut got_welcome = false;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline && denied.is_none() && !got_welcome {
        for event in attacker.poll() {
            match event {
                ClientEvent::Connected => {
                    attacker.send(encode(&ClientMsg::Login { name: name.to_owned(), token }));
                }
                ClientEvent::Message(data) => match decode::<ServerMsg>(&data) {
                    Some(ServerMsg::LoginDenied { reason }) => denied = Some(reason),
                    Some(ServerMsg::Welcome { .. }) => got_welcome = true,
                    _ => {}
                },
                _ => {}
            }
        }
        on_tick();
        std::thread::sleep(Duration::from_millis(16));
    }
    assert!(!got_welcome, "the attacker must never receive a Welcome");
    denied.expect("attacker must receive a LoginDenied answer, not silence")
}
