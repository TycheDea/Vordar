# Networking & Server Audit — 2026-07-11 (rev 2)

Second run of this date — supersedes the morning report. Between the two runs,
uncommitted changes landed in `game/vordar-protocol/src/lib.rs`,
`server/vordar-server/src/net_plugin.rs`, `smirk/engine-net/src/common.rs`,
`smirk/engine-net/src/server.rs`, `smirk/engine-net/src/lib.rs`, plus a new
`smirk/engine-net/src/metrics.rs`, attempting to address morning findings 1, 2,
and 16. Each change was re-verified against the code (`cargo check` passes with
3 dead-code warnings). Two of the attempted fixes introduced regressions — one of
them game-breaking under load — and two are dead code that fixes nothing. Those
are the new top findings. Everything else from the morning report was re-verified
and stands (all other files are untouched per `git status`).

## Ideal end state

Thousands of concurrent players per shard, each on a hostile, lossy internet link.
The server validates every byte a client sends, never allocates or queues unboundedly
on a client's behalf, and replicates per-client interest-managed, delta-compressed,
quantized state — time-sensitive data on loss-tolerant paths, transactional data on
reliable ones. The client hides 100+ ms of latency with prediction that models the
full movement rule (including dashes and collision), buffers snapshots against jitter,
and reconnects seamlessly. Persistence is transactional and batched off the tick;
identity is account-based with authenticated servers. Every one of those properties is
verifiable headless under simulated latency, loss, and jitter in both directions.

## Findings (ranked by impact)

> **All fix-sized findings in this file are implemented** (1–8, 11, 13, 14,
> 16–20 done; 9/10/12/15 moved to `reworks-networking-2026-07-11.md`). The
> remaining queue is rework-scale and lives in the reworks file, ordered
> cross-type there: **rework 8 → 10 → 1 → 5 → 3 → 4 → 7 → 2 → 6** (9 parked,
> gated on measurement). Numbers are stable — findings are never renumbered,
> so `/implement-finding N` and cross-references stay valid.

### 1. REGRESSION: `MAX_FRAME` cut to 1 KiB — but it is the shared cap for BOTH directions; snapshots over ~1 KiB now disconnect the client

- **Evidence:** `smirk/engine-net/src/common.rs:10` now reads
  `pub(crate) const MAX_FRAME: usize = 1024; // client→server: ~1 KiB is ample for all intents`.
  The comment reveals the misunderstanding: `MAX_FRAME` is enforced in `read_frame`
  (`common.rs:41`), which is used by **both** the server reader (client→server
  frames) and the **client reader** (server→client frames, `client.rs:233`). The
  morning report (old Finding 2) recommended *direction-specific* caps precisely
  because a single constant can't serve both. Server→client snapshot size: at the
  `MAX_SNAPSHOT_STATES = 64` budget (`net_plugin.rs:59`), each `EntityPos` costs a
  ≥5-byte varint id (hecs generation bits put every id ≥ 2³²) + 12 bytes position +
  1–5 bytes hp ≈ 18–22 bytes → a full `states` list alone is ~1.2–1.4 KiB, before
  `enters` (which carry prefab `String`s) are added. `write_frame` (`common.rs:25`)
  performs no size check, so the server happily sends the oversized frame; the
  client's `read_frame` rejects it as a protocol violation
  (`"bad frame length"`, `common.rs:42`) and the connection dies.
- **Ideal:** Two caps: client→server ~1 KiB (the intent vocabulary genuinely fits),
  server→client sized to the worst legitimate snapshot with headroom (e.g. 64 KiB),
  and a debug assertion in `write_frame` so an oversized outbound frame fails loudly
  at the sender, not as a cryptic disconnect at the receiver.
- **Gap:** Any client whose AOI contains roughly 50+ replicated entities gets
  disconnected the moment a full snapshot is emitted — and with client reconnect
  still unimplemented (Finding 7), that's a frozen world requiring a restart. The
  small e2e tests (2 bots) won't trip this; the 200-bot soak and any real crowd
  will, every time. This is strictly worse than the 1 MiB value it replaced: that
  was a DoS-hardening concern, this is a correctness break.
- **Suggestion:** Revert to a split: `MAX_FRAME_IN` (server-side read cap, 1 KiB is
  fine) and `MAX_FRAME_OUT`/client-side read cap (≥ 64 KiB), threaded through
  `read_frame` as a parameter or two constants chosen by caller. Add a soak-scale
  snapshot test that would have caught this (one bot inside a 100-entity crowd
  asserting it stays connected through several snapshot waves).
- **Path:** (1) split the constant and pick the reader's cap per side; (2) debug
  assert in `write_frame`; (3) crowd-snapshot regression test; (4) then the original
  Finding-2 hardening (rate limiting etc.) can proceed on top.

### 2. REGRESSION: over-unit move directions are now rejected instead of normalized — no epsilon, and the server rule diverges from the shared movement rule the client still runs

- **Evidence:** `net_plugin.rs:343` replaced the morning's normalize-on-cap with
  `if !dir.is_finite() || dir.length_squared() > 1.0 { continue; }`. The NaN check
  is correct (morning Finding 1). The rejection is not:
  - The client builds its dir via `dir.normalize()` (`client/vordar-client/src/lib.rs:128`)
    and sends the XZ projection. An f32 `normalize()` result routinely lands a few
    ULP **above** 1.0 in `length_squared()`. With a strict `> 1.0` comparison, an
    honest client's ordinary movement intent is intermittently rejected depending on
    camera yaw — the server stands still for that tick, the client predicted motion,
    and the difference surfaces as reconciliation corrections (`net.rs:37-45`) that
    look like random rubber-banding.
  - The shared movement rule still normalizes: `movement_velocity` clamps over-unit
    dirs (asserted by the client test `replay_normalizes_direction_like_the_simulation`,
    `client/vordar-client/src/net.rs:937-944`). Client prediction/replay and the
    server intent gate now implement **different rules** — the exact
    "same math on client and server" contract (`docs/online-play.mmd:48-50`) the
    codebase is built around, broken at the front door.
  - Note the rejected intent has already consumed `seq`/`t`
    (`net_plugin.rs:341-342` run before line 343), so the client's pending entry for
    that seq is silently skipped — reconciliation self-heals, but each rejection is
    one tick of guaranteed misprediction.
- **Ideal:** Reject only genuinely malicious input (NaN/Inf, or length² above
  1.0 + a small tolerance like 1e-3); inside the tolerance, clamp/normalize —
  identical to what `movement_velocity` does — so the validation layer and the
  simulation agree bit-for-bit with the client's replay.
- **Gap:** Anti-cheat strictness was bought by rejecting honest input and forking the
  shared rule. The cheat this guards against (over-unit speed) was already fully
  neutralized by normalization — rejection adds no security, only false positives.
- **Suggestion:** `if !dir.is_finite() || dir.length_squared() > 1.0 + 1e-3 { continue; }`
  followed by `let dir = if dir.length_squared() > 1.0 { dir.normalize() } else { dir };`
  — NaN/gross violations rejected, epsilon-scale float noise normalized, shared rule
  restored. Add a unit test feeding a `Vec2` at `1.0 + f32::EPSILON` length² through
  the receive path asserting acceptance.
- **Path:** (1) tolerance + clamp; (2) epsilon acceptance test; (3) the same pattern
  for any future client float that has a legal range.

### 3. NEW: facade fixes — `WRITER_QUEUE_CAP` and `NetMetrics` exist but are wired to nothing

- **Evidence:** `smirk/engine-net/src/server.rs:99` declares
  `pub const WRITER_QUEUE_CAP: usize = 128;` with a doc comment claiming a
  "Bounded writer queue policy: drop connection when backlog exceeds this" — but the
  writer queue is still the same unbounded channel (`server.rs:217`,
  `unbounded_channel`), and no code reads the constant (verified by grep: its only
  occurrence is the declaration). `smirk/engine-net/src/metrics.rs` defines
  `NetMetrics` with atomic counters, `server.rs:7` imports it, and nothing ever
  constructs or records into it — `cargo check` emits "struct `NetMetrics` is never
  constructed / associated items ... never used" (3 dead-code warnings).
- **Ideal:** Code that claims a policy enforces it. If a bounded writer queue or
  metrics aren't being built yet, no artifacts should exist that tell the next
  reader they are.
- **Gap:** This is worse than the fixes being absent: the morning report's Finding 2
  (slow-consumer backlog) and Finding 16 (observability) now *look* addressed to
  anyone skimming the code, while the unbounded-queue memory growth and the zero
  instrumentation are exactly as they were. The doc comment on the constant is
  actively false.
- **Suggestion:** Either finish the work — writer queue depth tracked at enqueue
  (the router at `server.rs:146-160` is the single producer, so a depth counter +
  `Outgoing::Kick` on breach is a ~15-line change) and `NetMetrics` actually
  incremented in `read_frame`/`write_frame` call sites and exposed via a
  `NetServer::metrics()` accessor — or delete both artifacts until the real
  implementation lands.
- **Path:** (1) decide finish-or-delete; (2) if finishing: depth counter + kick on
  breach, metrics recording at the four obvious sites, periodic dump from the sim
  thread; (3) a stalled-reader test (connect, never read, assert the server kicks
  within N seconds and memory stays flat).

### 4. No flood control: unbounded queues, no rate limiting, no connection caps (carried; one sub-item resolved, one regressed)

- **Evidence:** Re-verified unchanged:
  - Every channel is still unbounded: server events and outgoing router
    (`smirk/engine-net/src/server.rs:51-52`), per-connection writer queues
    (`server.rs:217`), client channels (`client.rs:76-77`), DB requests (`db.rs:85`).
  - The sim drains `ServerEvent`s once per Input tick (`net_plugin.rs:244`); a client
    sending frames faster than 60/s grows the event queue without bound.
  - Slow-consumer direction unchanged (see Finding 3 — the "cap" is a dead const).
  - `endpoint.accept()` loops with no connection cap, no per-IP limit, no QUIC
    retry/address validation (`server.rs:168`); still no `TransportConfig` on the
    server, so idle timeout, stream limits, and flow-control windows are quinn
    defaults.
  - **Resolved sub-item:** `Login.name` is now validated ≤ 32 chars, printable ASCII
    (`net_plugin.rs:281-284`). Correct. (Nit: `c != ' '` is redundant —
    `is_ascii_graphic()` already excludes space.)
  - **Regressed sub-item:** the frame-size cap became Finding 1.
- **Ideal:** Per-connection receive budget (messages/s and bytes/s) enforced on the
  network thread; direction-specific frame caps; bounded writer queues with a
  disconnect-on-backlog policy; explicit `TransportConfig`; QUIC retry tokens for
  source-address validation; global and per-IP connection caps.
- **Gap:** A hostile or stalled client can still grow server memory without limit in
  three independent places, and a UDP spoofer still gets amplification.
- **Suggestion / Path:** As the morning report: (1) direction-specific frame caps
  (now urgent — Finding 1); (2) bounded writer queues with kick policy (finish
  Finding 3); (3) reader-side token bucket; (4) explicit transport config +
  `Incoming::retry()`; (5) connection caps; (6) flood/stall hostile-bot test.

### 5. `MechanicScheduled` / `HitResult` broadcast to every connection — no interest management, position leak (carried, unchanged)

- **Evidence:** Re-verified at shifted lines: `net_plugin.rs:401`, `net_plugin.rs:465`
  (`MechanicScheduled`) and `net_plugin.rs:712` (`HitResult`) still use
  `state.server.broadcast(..)`, fanning out to every registered connection including
  pre-login ones (writer registration at `server.rs:218` precedes Login). Contrast
  `EntityDied`, which filters by the known set (`net_plugin.rs:959-966`).
- **Ideal:** AOI-scoped sends with identical payloads (clock-anchored countdown
  semantics don't require global addressing). `WorldClock` (`net_plugin.rs:843`)
  remains legitimately global.
- **Gap:** (a) aggregate mechanic traffic is O(players × casts) — the first
  networking bandwidth bottleneck at 1000 players; (b) a cheating client gets a
  zone-wide radar from telegraph positions.
- **Suggestion / Path:** (1) replace the three broadcasts with known-set/radius
  filtered sends; (2) handle AOI-entry onto an active telegraph (re-send or accept
  the miss); (3) e2e test asserting a far bot never sees out-of-AOI mechanics.

### 6. Clock sync locks onto the all-time-best RTT sample — offset drifts over long sessions (carried, unchanged)

- **Evidence:** `smirk/engine-net/src/client.rs:250-256` — offset updates only when
  `rtt <= clock.best_rtt` (all-time minimum); the 10 s re-check pings
  (`client.rs:33`, `221-225`) are discarded unless they beat the historical best.
  Client and server `Instant` clocks skew 10–50 ppm → 36–180 ms error per hour with
  no correction path.
- **Ideal:** Windowed-minimum sampling plus drift-rate estimation; offset changes
  applied as a slew, never a step.
- **Gap:** Telegraph fill/resolve timing (`vordar-protocol/src/lib.rs:61-68`,
  `client/net.rs:594-609`) and intent deadlines (`net_plugin.rs:599-618`,
  `FUTURE_SLACK_MICROS` = 50 ms at `net_plugin.rs:608`) all hang on this clock; a
  one-hour session erodes the entire validation budget.
- **Suggestion / Path:** (1) best-of-last-N window (~1–2 min); (2) slewed offset;
  (3) drift-rate estimate; (4) skewed-clock headless test (blocked on Finding 17's
  impairment gap).

### 7. Client connection lifecycle: disconnect is a log line, redirect failure is a panic (carried, unchanged)

- **Evidence:** `client/vordar-client/src/net.rs:181` —
  `ClientEvent::Disconnected => log::warn!(..)`; no reconnect, no teardown, no UI.
  Connect and redirect-connect `panic!` on failure (`net.rs:65`, `net.rs:257-258`)
  with the character already persisted into the target zone (`net_plugin.rs:786`).
  engine-net's `NetClient` remains single-connection with fire-and-forget `connect`.
- **Ideal:** A reconnect state machine: keep the local world, back-off retry,
  relogin (server-side takeover at `net_plugin.rs:295-310` already supports it),
  clock resync, AOI rebuild, connection state in the UI.
- **Gap:** On real links, brief loss is routine; today it's a crash or a frozen
  world — the most player-visible robustness gap, and Finding 1 currently *triggers*
  it in crowds.
- **Suggestion / Path:** (1) route `Disconnected` into the redirect-style teardown
  (`handle_redirect`, `net.rs:234-264`) + retry loop; (2) de-panic both connect
  sites; (3) reconnect UI; (4) kill-and-restart e2e test; (5) QUIC connection
  migration later.

### 8. Combat state not persisted, identity is a bare name — relog resets cooldowns, anyone can kick anyone (carried, unchanged)

- **Evidence:** `CharacterRecord` persists only zone/pos/health (`db.rs:37-41`);
  cooldowns live solely in `PlayerConn.last_cast` (`net_plugin.rs:153`); name-keyed
  takeover with no ownership proof (`net_plugin.rs:295-318`). Auth deferred by
  project decision (`db.rs:14-16`) — the gap's shape stays on record.
- **Ideal:** Relog never advantageous (persist or pessimistically restore
  cooldowns); account-based identity with session tokens in `Login`; signed handoff
  tokens through `Redirect` so zones can verify transfers (today a zone checks only
  `record.zone == state.zone.name`, `net_plugin.rs:502`).
- **Gap:** Two live exploits at the final bar: cooldown-reset-by-relog and
  kick-by-name.
- **Suggestion / Path:** (1) pessimistic full cooldowns on spawn (one line, no auth
  needed); (2) persist cooldown remainders; (3) accounts table + `account_id`;
  (4) token-bearing `Login`; (5) transfer handoff tokens; (6) login rate limiting.

### 9. Every message class rides one reliable ordered stream — head-of-line blocking by design (carried, unchanged)

- **Moved:** rework-scale, needs a design pass first - now finding 3 of
  `reworks-networking-2026-07-11.md` (implement via /plan-rework, not /implement-finding).

### 10. No jitter buffer or extrapolation — remote entities freeze at every late snapshot (carried, unchanged)

- **Moved:** rework-scale, needs a design pass first - now finding 4 of
  `reworks-networking-2026-07-11.md` (implement via /plan-rework, not /implement-finding).

### 11. Prediction replay models plain movement only — leaps and collisions produce snaps at real latency (carried, unchanged)

- **Evidence:** `replay_position` (`client/net.rs:451-457`) folds pending intents as
  pure `movement_velocity` steps; the simulation also applies `LeapImpulse`
  (client optimistic insert `net.rs:788-795`, server `net_plugin.rs:461-464`) and
  collision response (`PhysicsPlugin`). At 150 ms RTT an Onslaught replays ~9
  intents without the dash → error past `SNAP_DISTANCE` = 1.0 (`net.rs:41`) →
  visible mid-dash teleport; wall contact at latency causes constant correction tug.
- **Ideal:** Replay runs the full movement rule (dash override, collision) — the
  shared-rule contract (`docs/online-play.mmd:48-50`) honored in the one place it
  currently isn't. (Finding 2 notes the server-side gate also just forked this rule.)
- **Suggestion / Path:** (1) leap-aware replay; (2) static-geometry collision in
  replay (or suppress corrections during dashes as a stopgap); (3) 150 ms e2e
  Onslaught test asserting corrections stay under `SNAP_DISTANCE`.
- **Split:** the full collision-in-replay ideal moved to `reworks-networking-2026-07-11.md`
  finding 7. Implementable here: leap-aware replay, the dash correction-suppression stopgap,
  and the 150 ms e2e Onslaught test.

### 12. Wire format waste: 5-byte-minimum entity ids, repeated prefab strings, unquantized absolute states (carried, unchanged)

- **Moved:** rework-scale, needs a design pass first - now finding 5 of
  `reworks-networking-2026-07-11.md` (implement via /plan-rework, not /implement-finding).

### 13. Persistence engineering: per-row autocommit, no WAL, autosave bursts ahead of logins, no migration or shutdown story (carried, unchanged)

- **Evidence:** Each `Save` is a standalone `UPDATE` (`db.rs:150-158`) — one
  transaction (and fsync in rollback-journal mode) per player; `AutosaveSystem`
  enqueues every player the same tick (`net_plugin.rs:980-991`); the FIFO channel
  that guarantees cross-zone ordering (`db.rs:7-11`) queues logins behind the burst.
  No `journal_mode=WAL` / `synchronous` / `busy_timeout` anywhere in `db.rs`.
  Schema evolution is `CREATE TABLE IF NOT EXISTS` only (`db.rs:24-34`), no
  `user_version`. Shutdown is `Drop`-join only (`db.rs:127-137`) while `main` runs
  forever with no signal handling (`main.rs:77-79`); a panicked zone thread dies
  silently (`let _ = handle.join()`).
- **Ideal:** WAL + `synchronous=NORMAL`; autosaves staggered across ticks and
  batched one-transaction-per-wave; loads prioritized without breaking per-character
  save-before-load ordering; `user_version` migrations; deliberate
  drain-save-flush-exit shutdown; durability classes once items/trades exist
  (some writes synchronous-confirmed).
- **Suggestion / Path:** (1) PRAGMAs at `DbWorker::spawn`; (2) batched-transaction
  worker loop; (3) staggered autosave (same trick as snapshot STAGGER,
  `net_plugin.rs:68-71`); (4) migration runner; (5) SIGINT/SIGTERM → graceful
  shutdown; (6) durability taxonomy with inventory.
- **Split:** steps (4)-(6) - migration runner, graceful shutdown, durability classes - moved to
  `reworks-networking-2026-07-11.md` finding 8. Implementable here: steps (1)-(3)
  (PRAGMAs, batched-transaction worker loop, staggered autosave).

### 14. The entire network stack runs on one single-threaded runtime per endpoint (carried, unchanged)

- **Evidence:** `server.rs:64` — `new_current_thread()`; all connections' TLS,
  packet processing, and framing share one OS thread. Per-frame cross-thread locks:
  `RttMap` mutex insert per received app frame (`server.rs:241`), `ConnMap` lock per
  outgoing send/broadcast (`server.rs:149`), sim-thread `rtts` read per intent
  (`server.rs:114` via `net_plugin.rs:331`).
- **Ideal:** Network capacity scales with cores (runtime pool sharded by `ConnId`);
  per-connection atomic RTT instead of a global map lock.
- **Gap:** One core of QUIC crypto is the zone's hard vertical ceiling; nothing
  measures where it saturates.
- **Suggestion / Path:** (1) instrument network-thread busy time in the soak (ties
  into Finding 3's metrics, once real); (2) per-conn atomic RTT; (3) runtime
  sharding when the probe shows >50 % of a core at target load.
- **Split:** step (3), runtime sharding, moved to `reworks-networking-2026-07-11.md` finding 9.
  Implementable here: steps (1)-(2) (network-thread busy-time instrumentation, per-conn atomic RTT).

### 15. Certificate story and `Redirect { addr: SocketAddr }` — the final trust model can't be swapped in without a protocol change (carried, unchanged)

- **Moved:** rework-scale, needs a design pass first - now finding 6 of
  `reworks-networking-2026-07-11.md` (implement via /plan-rework, not /implement-finding).

### 16. Validation and handshake hardening: seq=0 replay wedge, `min` vs `max` doc drift, version mismatch is a silent close (carried, unchanged)

- **Evidence:** Re-verified at shifted lines:
  - `validate_intent` (`net_plugin.rs:601`): `if pc.last_seq != 0 && seq <= pc.last_seq`
    — `seq = 0` keeps `last_seq` at the sentinel forever, so repeated seq-0 intents
    pass monotonicity; the "replays are free rejects" claim (`net_plugin.rs:600`)
    doesn't hold for seq 0.
  - `server/vordar-server/src/lib.rs:10-11` still documents the arrival deadline as
    "min(RTT, MAX_REWIND)"; the code is `rtt.max(MAX_REWIND_MICROS)`
    (`net_plugin.rs:614`) — a floor. QUIC RTT is client-influenceable (delayed
    ACKs), widening `max_age`; the resolve-time rewind cap (`net_plugin.rs:660`)
    contains it, but the doc must state the real rule.
  - Version mismatch: server returns `Err` and the connection dies
    (`server.rs:207-209`); no reason reaches the client (`Ctrl` has no reject frame,
    `common.rs:18-23`); version is a bare `u8` equality with no compat range.
- **Suggestion / Path:** (1) reject `seq == 0` + unit test; (2) fix the doc line;
  (3) `Ctrl::Reject { reason }` (or `connection.close` with a well-known code, as
  kicks already do at `server.rs:160`) surfaced through `ClientEvent` + a
  version-mismatch e2e test.

### 17. Impairment layer only simulates server→client loss — the intent path, jitter, and clock skew are untestable (carried, unchanged)

- **Evidence:** `smirk/engine-net/src/impair.rs:18-29` drops **received** datagrams
  on a **client** endpoint only ("Client-side receive drop == server→client loss",
  `impair.rs:6`). No client→server loss, no jitter/reorder, no bandwidth cap, no
  clock-skew knob (needed for Finding 6's test). Latency simulation
  (`client.rs:194-243`) is symmetric-only.
- **Ideal:** Full both-direction UDP conditioner: loss, one-way delay, jitter,
  reorder, rate cap — so every client-feel claim has a headless test at WAN
  profiles.
- **Suggestion / Path:** (1) drop probability on `try_send` (same LCG, ~5 lines);
  (2) upstream-loss probe on applied-intent cadence (observable via snapshot acks);
  (3) jitter/reorder; (4) skewed-clock harness; (5) named WAN profiles in the
  ignored test suite.

### 18. Operational blindness: no real metrics, silent zone death (carried; attempted fix is dead code — see Finding 3)

- **Evidence:** The new `NetMetrics` is never constructed or recorded
  (`metrics.rs:8`, cargo dead-code warnings), so the server still exposes nothing:
  no reject counters (each reject is a `log::debug!`, `net_plugin.rs:337`), no
  queue depths, no bytes/s. A panicked zone thread is still swallowed
  (`main.rs:77-79`); other zones keep redirecting players into the dead zone.
- **Ideal:** Counters on every hostile-client-touched and capacity-relevant path,
  dumped periodically per zone; zone-thread watchdog that restarts or pulls the
  zone from the directory.
- **Suggestion / Path:** (1) wire `NetMetrics` for real (Finding 3) + a reject
  counter in `validate_intent`'s callers; (2) periodic structured log line;
  (3) zone watchdog; (4) exportable format when infrastructure exists.

### 19. `docs/online-play.mmd` diverges from the code it documents (carried, unchanged)

- **Evidence:** Re-verified against the diagram: no persistence lane
  (Login → DB load → spawn gating, `net_plugin.rs:284-329`/`491-545`; autosave;
  disconnect-save); no death path (`EntityDied`, re-`Welcome` respawn rebind,
  `net_plugin.rs:551-571`, client `net.rs:186-190`); redirect edge drawn from
  mechanic resolve (`online-play.mmd:55`) but redirects originate from
  `ZoneTransferSystem` (`net_plugin.rs:749`) and login routing
  (`net_plugin.rs:500-514`); session takeover undocumented; "broadcast
  area-of-interest snapshots" describes snapshots but not the global mechanic
  broadcasts (Finding 5).
- **Suggestion / Path:** (1) add persistence lane, death/re-Welcome edge, correct
  redirect origin, takeover note; (2) regenerate `online-play.svg`; (3) reconverge
  wording once Finding 5 closes.

### 20. NEW: `MAX_CONNECTIONS_PER_IP = 8` makes the 200-bot soak scenario unrunnable — the flood cap and the soak harness disagree about what one IP means

- **Evidence:** `NetServer::MAX_CONNECTIONS_PER_IP = 8` (`smirk/engine-net/src/server.rs:148`,
  added by finding 4's flood-control work) refuses the 9th connection from one
  source IP (`server.rs:279`). `server/vordar-server/tests/soak.rs` drives all its
  bots from localhost, so the default 200-bot run gets 8 accepted and 192 refused
  and fails at "only 8/200 bots welcomed in 60s". Discovered during finding 14:
  its busy-time verification had to run at `VORDAR_SOAK_BOTS=8`, under the cap —
  the soak's stated purpose (tick budget at crowd scale) is currently untestable.
- **Ideal:** Production keeps the per-IP cap at its hostile-client default, and
  capacity tests can still model a real crowd (many clients, distinct identities)
  from one machine. Neither goal is sacrificed to the other, and the knob is a
  deliberate part of the transport's public configuration, not a test backdoor.
- **Gap:** The caps are hard associated consts on `NetServer` — there is no way
  for any embedder (soak harness today, a stress CLI or LAN deployment tomorrow)
  to state a different trust model for source IPs.
- **Suggestion:** Promote the connection caps into bind-time configuration with
  the current values as defaults: e.g. `NetServer::bind` keeps today's signature
  and defaults, plus a `bind_with_limits` (or small `NetLimits` struct) the soak
  harness uses to raise `max_connections_per_ip` to its bot count. The flood
  test keeps asserting the default; the soak states its single-IP-crowd reality
  explicitly. No env-var special case inside the transport, no weakened default.
- **Path:** (1) introduce the limits struct with today's consts as `Default`;
  (2) thread it through `bind` → accept-loop check (`server.rs:279`); (3) soak
  harness passes `max_connections_per_ip: BOT_COUNT`; (4) run the full 200-bot
  soak and record the restored baseline (including the new `net_busy_pct`);
  (5) flood-control test unchanged, still pinning the default at 8.

## Carried forward from previous report

Findings 4–19 above are the morning report's findings 2–17, re-verified against the
current working tree (only `net_plugin.rs`, `common.rs`, `server.rs`, protocol
`lib.rs`, and the new `metrics.rs` changed; all other cited files are untouched).
Line references were updated where the `net_plugin.rs` edits shifted them (+4 lines
after the Login check, +5 after the cast-target check).

## Resolved since last report

- **Morning Finding 1 (NaN injection) — core resolved, with a regression.**
  `!dir.is_finite()` (`net_plugin.rs:343`) and `!target.is_finite()`
  (`net_plugin.rs:375`) close the NaN/Inf hole for both intent types. Still missing
  from that finding's path: the world-bounds sanity clamp on cast targets and the
  adversarial decode-fuzz test. The accompanying change from normalize-to-reject
  introduced new Finding 2.
- **Morning Finding 2, name-validation sub-item — resolved.** `Login.name` capped at
  32 printable-ASCII chars (`net_plugin.rs:281-284`); protocol doc annotated
  (`vordar-protocol/src/lib.rs:35`). The rest of that finding stands (Finding 4),
  and its frame-cap sub-item regressed into Finding 1.

No other prior finding is resolved; two attempted fixes (writer-queue cap, metrics)
are inert — see Finding 3.
