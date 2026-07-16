# Networking & Server Reworks — 2026-07-11

Rework-scale companion to `audit-networking-2026-07-11.md`: findings that need a
design pass before implementation. Consumed by /plan-rework, which turns one
rework into a plan of fix-sized steps for /implement-finding. Created
retroactively from the deferred remainders of implemented findings 7 and 8.

## Findings (implementation order)

> **Cross-type queue** (all 20 fix-sized findings of
> `audit-networking-2026-07-11.md` are done, so this file is the whole
> remaining queue): **~~8~~ → ~~10~~ → ~~1~~ → ~~5~~ → ~~3~~ → ~~4~~ → ~~7~~ → 2 → 6.**
> 8 done 2026-07-12 (plan-networking-rework-8-2026-07-12.md, 5 steps),
> 10 done 2026-07-13 (plan-networking-rework-10-2026-07-12.md, 3 steps),
> 1 done 2026-07-13 (plan-networking-rework-1-2026-07-13.md, 5 steps),
> 5 done 2026-07-13 (plan-networking-rework-5-2026-07-13.md, 5 steps;
> steady-state crowd snapshot measured 576 B, rework 3's MTU gate cleared).
> 3 done 2026-07-14 (plan-networking-rework-3-2026-07-13.md, 6 steps; snapshot
> states + intent ack and move intents now ride datagrams, clock pings moved off
> the writer queue, and the WAN-RTT after-probe shows inter-snapshot gaps bound
> to cadence multiples instead of retransmit cycles).
> 4 done 2026-07-14 (plan-networking-rework-4-2026-07-14.md, 4 steps; remote
> entities render from a tick-indexed sample buffer at a fixed 200 ms playback
> delay with capped extrapolation, smoothness probe gates in BASELINE.md;
> spawned finding 11, the RESYNC_TICKS vs extrapolation-cap interaction).
> 7 done 2026-07-16 (plan-networking-rework-7-2026-07-14.md, 4 steps; online-play
> diagram updated with shared-rule collision contract; with-statics reconcile
> benchmarks recorded in BASELINE.md).
> 11 done 2026-07-16 (plan-networking-rework-11-2026-07-16.md, 3 steps; the
> playback cursor now clamps at `latest_state_tick + EXTRAP_CAP_TICKS` and
> resyncs forward-only — a sustained stall is a terminal capped hold with
> graded forward-only recovery, no periodic backward pop).
> 8 first because two entries depend on it: 10 is blocked on its `NetServer`
> shutdown path, and 1's schema changes (accounts table, cooldown columns) need
> its `user_version` migration runner. 10 right after 8 while the shutdown work
> is fresh. 5 before 3 despite lower impact rank: QUIC datagrams carry ~1.2 KB
> and crowd snapshots are ~2.2 KB today, so 3's snapshots-on-datagrams step is
> physically impossible until 5's compaction shrinks them under the MTU.
> 4 after 3 because moving snapshots to lossy datagrams changes exactly the
> arrival pattern the jitter buffer is designed around. 2 and 6 after 1: both
> interact with its tokens (a migrated path or redirect must not become a
> session-hijack vector). 7 is fully independent — its slot is impact-based and
> it can be interleaved anywhere. **9 is parked, not ordered:** its own gate is
> ">50% of a core at target load" and the restored 200-bot soak measured
> `net_busy_pct=13.6`. Every plan produced from this queue must include a
> `docs/online-play.mmd` + SVG update step when it changes the online-play
> flow, so the diagram converged by audit finding 19 stays true. Numbers are
> stable — findings are never renumbered.

### 1. Account identity, auth tokens, and combat-state persistence (deferred from audit finding 8, Path steps 2–6)

- **Evidence:** `Login` is a bare character name (`vordar-protocol` `ClientMsg::Login { name }`,
  validated only as ≤ 32 printable ASCII in `net_plugin.rs`). Session takeover kicks purely
  by name match — anyone who knows a character name can kick its player and take the
  session. `PlayerConn.last_cast` lives only in server memory; `CharacterRecord` persists
  only zone/pos/health. The pessimistic-cooldowns fix (finding 8 step 1, commit `4a49adb`)
  closed the relog-reset exploit with an approximation, not with real persistence.
- **Ideal:** Account-based identity: an accounts table, a token-bearing `Login` the server
  verifies, zone-transfer handoff tokens so a `Redirect` can't be replayed or hijacked,
  and login rate limiting. Cooldown remainders persisted with the character so a relog
  restores the exact combat state rather than a pessimistic reset.
- **Gap:** Identity is spoofable and kick-by-name is open griefing; combat state survives
  relog only via the pessimistic approximation; nothing rate-limits login attempts.
- **Suggestion:** Design this as one coherent auth + persistence rework: schema (accounts,
  session tokens), protocol (versioned `Login` carrying a token), the transfer handoff
  flow between zones, login rate limiting, and cooldown-remainder columns on
  `CharacterRecord` — the pieces interlock, so ordering and protocol versioning need a
  plan before any code.
- **Path:** From finding 8's original Path: (2) persist cooldown remainders; (3) accounts
  table; (4) token-bearing `Login`; (5) transfer handoff tokens; (6) login rate limiting.
  A design pass must fix the ordering, the schema migration story, and how dev-mode
  (auth deliberately deferred, see project decision) coexists with the real flow.

### 2. QUIC connection migration for seamless network switching (deferred from audit finding 7, Path step 5)

- **Evidence:** The client reconnect state machine (finding 7 steps 1–4, commit `04fc276`)
  treats every connection loss the same way: teardown of the replicated world, backoff
  redial, relogin, full resync. A mere network path change (Wi-Fi → cellular, NAT rebind)
  goes through that whole cycle even though QUIC supports migrating a live connection.
- **Ideal:** quinn's connection migration keeps the session alive across client address
  changes — no relogin, no world teardown, no visible interruption beyond a latency blip.
- **Gap:** Every path change costs a full disconnect/reconnect cycle and its gameplay
  interruption; on mobile-style networks that is frequent, not exceptional.
- **Suggestion:** Design pass on enabling and validating quinn's migration support
  server-side (path validation, anti-amplification interplay with the finding-4 retry
  gate) and on the session-identity implications — this interacts directly with rework 1's
  tokens (a migrated path must not become a session-hijack vector).
- **Path:** (1) design: quinn migration config + security analysis against the finding-4
  flood controls; (2) impairment-layer knob for mid-session address switching (relates to
  audit finding 17); (3) e2e test migrating a session mid-combat with no relogin.

### 3. Every message class rides one reliable ordered stream — head-of-line blocking by design (moved from audit finding 9)

- **Evidence:** One bidirectional stream per connection (`server.rs:198-201`,
  `client.rs:171-174`); intents, snapshots, mechanics, world clock, deaths,
  redirects, and clock pings share it (tags at `common.rs:6-7`). The loss probe
  measured 164 ms worst gap at 5 % loss / 50 ms RTT and
  `docs/benchmarks/BASELINE.md:204-209` recorded the deferral (WEAKPOINTS #4).
- **Ideal:** Snapshots and clock pings on QUIC datagrams (superseded state should be
  skipped, not retransmitted); intents on datagrams with last-N redundancy; reliable
  streams for identity/transactional messages only. Datagram pings also remove
  writer-queue delay from RTT samples (`server.rs:237` stamps `t_server` before
  queuing behind snapshot frames).
- **Gap:** The measurement's envelope is narrow: 50 ms RTT, downstream loss only.
  At 150–250 ms RTT one retransmit cycle exceeds the 250 ms gate by arithmetic;
  upstream loss stalling the intent stream (client-felt rubber-banding) has never
  been measured because the impairment layer can't express it (Finding 17).
- **Suggestion / Path:** (1) both-direction impairment; (2) re-probe at WAN RTTs;
  (3) clock pings to datagrams; (4) `Snapshot.states` to datagrams (tick-stamped
  latest-wins — `states` are already order-independent; only `enters`/`leaves` need
  the stream); (5) input redundancy.

### 4. No jitter buffer or extrapolation — remote entities freeze at every late snapshot (moved from audit finding 10)

- **Evidence:** `client/net.rs:833-842` — `NetLerpSystem` completes in exactly one
  snapshot interval then holds; `apply_snapshot` restarts lerps from the displayed
  position (`net.rs:393-397`), converting jitter into speed warble. Measured gaps up
  to 164 ms against a 100 ms budget (`BASELINE.md:204-206`) = visible freezes today.
- **Ideal:** Fixed interpolation delay (~1.5–2 intervals behind newest) over a
  tick-indexed buffer, with capped extrapolation from `NetMotion.velocity`
  (`net.rs:394`) when the buffer runs dry. Snapshot `tick` is already on the wire
  (`vordar-protocol/src/lib.rs:48`) and currently unused for timing.
- **Suggestion / Path:** (1) tick-indexed buffer; (2) fixed-delay interpolation
  clocked off synced server time; (3) capped extrapolation; (4) loss-probe assertion
  on rendered-position smoothness.

### 5. Wire format waste: 5-byte-minimum entity ids, repeated prefab strings, unquantized absolute states (moved from audit finding 12)

- **Evidence:** Wire ids are hecs entity bits ≥ 2³² (`net_plugin.rs:884`) →
  5+ byte varints; `EntityPos` is raw 3×f32 + i32 hp (`vordar-protocol/src/lib.rs:102-108`);
  `EntityState.prefab` is a `String` per AOI entry (`lib.rs:94`); `hp: 0` conflates
  "no Health" with "dead" (`lib.rs:98-99`). Note this waste is also what pushed
  snapshots over the new 1 KiB cap (Finding 1) — id compaction and quantization
  shrink the same frames that are currently killing connections.
- **Ideal:** Zone-local u16/u32 replication ids bound at AOI entry; positions
  quantized to zone-local fixed point; prefab u16 registry pinned by content hash;
  hp as an explicit optional. Delta-vs-baseline only if numbers still bind after.
- **Suggestion / Path:** (1) compact ids bound in `enters`; (2) position
  quantization (protocol bump); (3) prefab registry + hash check at login;
  (4) measure via the bot `bytes` counter (`tests/common/mod.rs:44`); (5) delta
  compression last.

### 6. Certificate story and `Redirect { addr: SocketAddr }` — the final trust model can't be swapped in without a protocol change (moved from audit finding 15)

- **Evidence:** Fresh self-signed cert for `"localhost"` per boot
  (`common.rs:64-83`); client disables verification (`SkipServerVerification`,
  `common.rs:101-137`) and hardcodes SNI `"localhost"` (`client.rs:167`);
  `Redirect` carries a bare `SocketAddr` (`vordar-protocol/src/lib.rs:82`);
  directory is IP:port math (`main.rs:39-44`). Hostname-validated TLS needs names
  the protocol doesn't speak.
- **Ideal:** Zone directory and `Redirect` carry hostnames; client verifies against
  a real chain (public CA or pinned private game CA); skip-verification
  feature-gated out of release builds.
- **Suggestion / Path:** (1) hostname in `Redirect` + directory (protocol bump);
  (2) SNI parameter on `NetClient::connect`; (3) feature-gate the dev verifier;
  (4) real CA + pinned root at deployment; (5) handshake reason codes (Finding 16)
  so cert/version failures are distinguishable.

### 7. Collision-aware prediction replay (split from audit finding 11)

- **Evidence:** `replay_position` (`client/net.rs:451-457`) folds pending intents as pure
  `movement_velocity` steps; the simulation also applies collision response
  (`PhysicsPlugin`). Wall contact at latency causes constant correction tug. The
  fix-sized parts of audit finding 11 (leap-aware replay, dash correction suppression,
  150 ms e2e test) address the dash snap but not collision.
- **Ideal:** Client replay runs the full movement rule including static-geometry
  collision, so prediction error at a wall is zero and the shared-rule contract
  (`docs/online-play.mmd:48-50`) holds everywhere.
- **Gap:** Running collision inside replay means giving the client's reconciliation
  loop access to physics queries per replayed intent - an architectural decision about
  what the replay context owns, not a bounded diff.
- **Suggestion:** Design pass on exposing static collision to replay (shared collision
  rule crate-side, like `movement_velocity`?), its cost per reconciliation, and whether
  dynamic obstacles are in or out of the predicted set.
- **Path:** (1) design: replay-accessible collision query; (2) implement replay
  collision; (3) extend the 150 ms e2e test with a wall-hug scenario asserting
  corrections stay under `SNAP_DISTANCE`.

### 8. Persistence lifecycle: schema migrations, graceful shutdown, durability classes (split from audit finding 13, steps 4-6)

- **Evidence:** Schema evolution is `CREATE TABLE IF NOT EXISTS` only (`db.rs:24-34`),
  no `user_version`. Shutdown is `Drop`-join only (`db.rs:127-137`) while `main` runs
  forever with no signal handling (`main.rs:77-79`); a panicked zone thread dies
  silently. `NetServer` has no shutdown path at all (noted during the finding-7 work:
  its accept loop runs forever, the listening socket is never released).
- **Ideal:** `user_version`-driven migration runner; deliberate
  drain-save-flush-exit shutdown spanning zone threads, net threads, and the DbWorker;
  durability classes once items/trades exist (some writes synchronous-confirmed).
- **Gap:** Any schema change now requires hand-editing databases; any process stop is
  an unclean kill mid-save; every write shares one durability level.
- **Suggestion:** One design pass over process lifecycle ownership: who initiates
  shutdown, the ordering across zones/net/db, how `NetServer` gains a close path, then
  migrations and the durability taxonomy on top.
- **Path:** (1) design: shutdown ownership and ordering; (2) `NetServer` shutdown path;
  (3) SIGINT/SIGTERM -> drain-save-flush-exit; (4) migration runner with `user_version`;
  (5) durability classes with the first transactional feature. Fix-sized steps (1)-(3)
  of the original finding (PRAGMAs, batched transactions, staggered autosave) stay in
  the audit file.

### 9. Multi-core network runtime sharding (split from audit finding 14, step 3)

- **Evidence:** `server.rs:64` - `new_current_thread()`; all connections' TLS, packet
  processing, and framing share one OS thread per endpoint.
- **Ideal:** Network capacity scales with cores: runtime pool sharded by `ConnId`,
  no cross-shard locks on the per-frame paths.
- **Gap:** One core of QUIC crypto is the zone's hard vertical ceiling. Sharding is
  deliberately gated on measurement: audit finding 14's fix-sized steps (busy-time
  instrumentation, per-conn atomic RTT) produce the numbers first.
- **Suggestion:** Only design this once instrumentation shows >50% of a core at target
  load; then decide sharding boundary (per-connection vs per-endpoint), what state
  crosses shards, and how the sim-thread channel model changes.
- **Path:** (1) prerequisite: finding 14 steps (1)-(2) landed and soak numbers gathered;
  (2) design: shard boundary + state ownership; (3) implement behind the same public
  `NetServer` API; (4) soak comparison proving scaling.

### 10. Zone-thread watchdog recovery (restart or directory pull) blocked on `NetServer` shutdown path (deferred from audit finding 18, Path step 3)

- **Evidence:** Finding 18's visibility half is fixed: `main.rs` no longer discards a
  panicked zone thread's result (`join_zone_threads` in `lib.rs` logs it loudly). Actual
  recovery was investigated and found unsafe to build today: `NetServer::bind_with_limits`
  (`smirk/engine-net/src/server.rs`) spawns a detached network thread that owns the
  `quinn::Endpoint` and loops in `server_main` forever — there is no shutdown signal and
  no `Drop` impl on `NetServer`. When a zone's App panics, that background thread is
  never told to stop, so it keeps the port bound; a same-address restart attempt's
  `NetServer::bind` fails immediately ("address in use") and panics again, crash-looping
  instead of recovering. This is the identical prerequisite already named in rework 8's
  Path step (2) ("`NetServer` shutdown path"). The alternative — pulling the dead zone
  out of the shared directory so other zones stop redirecting into it — needs the
  per-thread-owned immutable `directory: HashMap<String, SocketAddr>` (`main.rs:39-44`,
  cloned once per zone thread; `net_plugin.rs` `NetServerState.directory`) to become
  shared mutable state read at both redirect sites (`net_plugin.rs` login-routing and
  `ZoneTransferSystem`) — also a real architecture decision, not a bounded diff.
- **Ideal:** `NetServer` exposes a real shutdown path (close the endpoint, join the
  network thread, release the port) so a watchdog can safely tear down and rebuild a
  panicked zone's App on the same address; a panicked zone recovers without operator
  intervention and without any redirect ever landing on a dead address.
- **Gap:** Neither recovery path is safe to build today: restart crash-loops on the
  still-bound port; directory-pull has no shared-state carrier yet. Only the visibility
  half of finding 18 was bounded, fix-sized work.
- **Suggestion:** Fold into rework 8's design pass — it already owns "how `NetServer`
  gains a close path." Once that lands, restart-on-same-address is very likely the
  simpler recovery model of the two, since it needs no protocol or redirect-routing
  change once `NetServer` can shut down cleanly.
- **Path:** (1) prerequisite: rework 8 step (2), `NetServer` shutdown path; (2) zone-
  thread supervisor that uses that shutdown path to rebuild a panicked zone's App on the
  same address, with a bounded restart count; (3) e2e test: a zone panics, the watchdog
  restarts it, and a fresh connection to the same address succeeds afterward.

### 11. Playback-cursor RESYNC threshold leaves almost no margin against the extrapolation cap — a sustained stall pops backward every ~35 ticks (discovered implementing `plan-networking-rework-4-2026-07-14.md` finding 2)

- **Evidence:** `advance_playback` (`client/vordar-client/src/net.rs`, rework-4
  finding 1) hard-snaps the shared playback cursor to `target =
  latest_state_tick - INTERP_DELAY_TICKS` once `|target - cursor| >
  RESYNC_TICKS` (30 ticks). Finding 2's capped extrapolation holds an
  entity's rendered position steady once `cursor - last_buffered_tick >
  EXTRAP_CAP_TICKS` (15 ticks). In cursor-vs-target coordinates the cap
  engages at divergence `EXTRAP_CAP_TICKS + INTERP_DELAY_TICKS = 15 + 12 =
  27` — only 3 ticks below RESYNC's 30-tick threshold. Reproduced by
  extending `net::tests::extrapolation_bridges_lost_snapshots_then_caps`'s
  own harness past its (deliberately short) cutoff: a remote entity moving
  +X at 6 u/s whose real samples stop after server tick 30 holds
  bit-identical at the capped point (position 4.5) for ticks 62–65, then at
  tick 66 RESYNC fires and the position pops backward to 1.8 (a 2.7-unit /
  27-nominal-step regression) — and the cycle repeats every ~35 ticks
  (~580 ms) for as long as the stall continues.
- **Ideal:** Once an entity is capped-and-held — or, more generally, once
  the connection has been silent long enough for RESYNC to consider
  snapping — recovering the cursor must not visibly regress a position that
  was already being rendered further along. Either RESYNC's target should
  account for the extrapolation cap (e.g. resync toward the capped point's
  own projection rather than the raw `latest_state_tick -
  INTERP_DELAY_TICKS`), or the two thresholds need enough margin that a
  stable capped-hold is a genuine terminal state until new real data
  arrives, with recovery handled by the existing dry-recovery synthetic
  sample instead of a blind snap.
- **Gap:** `RESYNC_TICKS` (finding 1) and `EXTRAP_CAP_TICKS` (finding 2)
  were each chosen independently within the same rework — 30 ticks
  "reconnect/long stall" vs. 15 ticks "matching the loss-probe gate" —
  without checking their interaction. The result is a real, reproducible
  periodic backward pop under any sustained stall (well past finding 2's
  1–2-loss target case, but squarely inside what a genuine multi-second
  network drop or reconnect produces), which finding 2's own Suggestion
  (sampling-function branches only) cannot fix without touching
  `advance_playback`.
- **Suggestion:** Needs a design pass, not a constant tweak — whether the
  fix is a smarter resync target, per-entity awareness of "currently
  capped" state feeding back into the shared cursor's resync decision, or
  reworking `advance_playback`'s hard-snap into something that also splices
  a synthetic sample the way `apply_states`'s dry recovery already does for
  ordinary arrivals.
- **Path:** (1) design: decide where the "capped, don't regress" invariant
  is owned (the shared cursor vs. per-entity state) and how it composes
  with a genuine reconnect (where snapping IS eventually correct — the
  world may have moved on entirely); (2) implement; (3) an extended version
  of `extrapolation_bridges_lost_snapshots_then_caps` (or a new test)
  running well past the current RESYNC boundary, asserting no backward
  position step ever occurs while the connection is stalled.

