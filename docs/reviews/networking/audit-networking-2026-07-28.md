# Networking Audit — 2026-07-28

Extraction pass, not a fresh sweep: source material is two external expert
reviews (grok, 2026-07-27 — netcode/sync and server security/persistence,
formerly `docs/reviews/grok/01,05-*.md`, deleted after this extraction; see git
history). Every finding was re-verified against the current tree by an
independent read of the cited code — evidence below is re-derived, not copied
— and filtered through project stage: dev single-player pack (real auth/TLS/
fleet deferred by design), pre-content foundation (no raid-scale content).
Production-multiplayer items are recorded once under **Deferred until
multiplayer** and stay out of the queue.

What the reviews confirmed holds up, verified: scheduled-snapshot combat
aligned to absolute server time; intent-only wire protocol (no client message
can express a position — `MoveIntentEntry` carries a unit-capped dir only);
dual-lane QUIC with clock pings bypassing the writer queue; ClockSync
(windowed-min RTT, least-squares drift, 2000 ppm slew, impairment-tested);
prediction/reconciliation parity with the shared sim including leap overrides
on both replay and rewind; AOI + crowd throttling that never corrupts identity
diffs; transport flood hygiene (retry validation, token buckets, per-IP caps,
writer-queue kick); token-gated session takeover with its e2e lock; the FIFO
DbWorker with save-before-redirect, migration ladder, and `fork()` reply
isolation; and the test culture across handshake/flood/impairment/WAN/soak
plus server e2e security/persistence/zones/shutdown.

## Ideal end state

Cast and movement validation that cannot interfere with each other; an
arrival deadline that means what DESIGN §3 says (one RTT, floored only while
unwarmed); persistence that fails closed on hostile or corrupt rows; every
connection accounted for from accept to login; and the datagram lane's health
visible in the same metrics line as everything else — so the netcode's
correctness claims are enforced by validation and tests, not by the absence
of adversaries.

## Findings (implementation order)

Cross-type queue:

> **QUEUE CLEARED — ~~finding 1~~ → ~~finding 2~~ → ~~finding 3~~ →
> ~~finding 4~~ → ~~finding 5~~ → ~~finding 6~~ → ~~finding 7~~ →
> ~~finding 8~~ → ~~finding 9~~.**
>
> Loop-final gate: clippy `-D warnings` clean, `cargo nextest run --workspace`
> **455 passed / 5 skipped** (443 before this queue). 7 `e9ff97e` (per-conn
> EWMA RTT baseline in `engine-net`, k·σ=3 structured samples at cast arrival
> and mechanic resolve — metrics only, no gameplay policy), 8 `8df21f6` (three
> datagram counters on the periodic line; snapshot gauge + `debug_assert` at
> 1200 B, crowded 64-state snapshot measured at **580 B**, 48% of budget),
> 9 `59be85c` (`HitResult` drives the existing `ParticleSim::burst` seam;
> HP/death authority stays on snapshot replication).
>
> Finding 9's Path step (3), the sandbox visual check, was NOT run and is not
> owed: this project verifies headless only. The handler is unit-tested
> (known id flashes at its transform, unknown id skipped); the flash's
> on-screen appearance is unverified, which is the stage-appropriate state.
>
> Struck entries done 2026-08-02, one commit each: 1 `3794018` (cast lane gets
> its own `cast_seq`/`cast_t`; `PROTOCOL_VERSION` 15→16; e2e holds the move
> datagram across a cast round trip), 2 `3373566`, 3 `4c69d44`, 4 `b0f789b`,
> 5 `a652467` (`subtle` 2.6, BSD-3-Clause, now a direct workspace dep),
> 6 `4da2655`.
>
> Three collateral corrections worth carrying. Finding 7's first estimator
> test could not fail either: 200 identical samples drive σ to ~0, so
> `400_000 > mean + 3·0` holds even against a no-op `update`, and the variance
> recurrence was wholly unpinned. Replaced by the property a k·σ consumer
> actually rests on — the same absolute sample must flag on a quiet
> connection and not on a jittery one — and red-proved by zeroing `self.var`.
>
> Finding 2's tightened deadline
> made finding 1's e2e guard (`held_age < 250_000`, sized for the dead 300 ms
> window) looser than the rule it stands on — retightened to the real ~100 ms
> warmed margin rather than widening the production window. Finding 6's first
> test asserted `mechanics.is_empty()`, which an unknown-skill lookup already
> satisfied pre-fix — it could not fail. Rewritten to the one thing the bound
> changes (an oversized id must not consume its cast seq) and red-proved by
> reverting the bound.
>
> 1–2 are the intent-validation cluster (2's boundary tests run on top of 1's
> split seq spaces). 3–6 are cheap server hardening, independent. 7–8 are the
> metrics pair (8's touch carries 7's counters naturally). 9 is client
> presentation, last.

### 1. Casts and moves share one sequence space — a cast can permanently drop an in-flight move

- **Evidence:** one client counter stamps both lanes:
  `client/vordar-client/src/net/mod.rs:135` (`seq`), `:221-223`
  (`send_cast_intent` increments it for the reliable-stream cast) and
  `client/vordar-client/src/net/prediction.rs:235-245` (`NetSendInputSystem`
  increments it for datagram moves); `net/mod.rs:85-86` orders the cast system
  after the move send in the same Input tick, so a same-tick cast takes
  `N+1` while move `N` rides a separately-routed datagram. Server-side, one
  validation stream: `server/vordar-server/src/net/receive.rs:264`
  (`dispatch_cast` sets `pc.last_seq = seq` after `validate_intent`) and
  `receive.rs:662` (`queue_move_intents`: `if seq <= pc.last_seq { continue; }`
  — silent skip, treated as last-3 redundancy). When the datagram carrying
  move `N` is delayed past the cast's stream delivery, `last_seq` becomes
  `N+1` and every resend of `N` (`MOVE_RING_LEN = 3`, `prediction.rs:30`) is
  skipped forever; the server stands still that tick (`receive.rs:568-571`),
  `applied_seq` jumps over `N` (`receive.rs:576-579`,
  `broadcast.rs:213`), and the client's pending entry is dropped by
  `pending.retain(|p| p.seq > last_processed_seq)` (`prediction.rs:68`) with
  the ~0.1-unit error landing inside `TRUST_DISTANCE = 0.3`, never corrected.
- **Ideal:** the two lanes validate independently — a cast can never advance
  movement monotonicity, and the snapshot ack is move-only by construction,
  not by the fragile invariant that `send_cast_intent` never records a
  pending entry.
- **Gap:** under ordinary lane reorder (stream beats datagram), one cast
  erases one tick of authoritative movement. The client absorbs it silently,
  which is exactly the problem: server position at T and client belief
  diverge inside the trust band, breaking the bit-for-bit replay contract
  the prediction tests stand on. (The external review rated this Critical;
  verified impact is one dropped move tick per cast under loss/reorder —
  High, a real correctness bug, not a visible break.)
- **Suggestion:** split the spaces: `cast_seq`/`cast_t` on `PlayerConn` and
  an own client-side cast counter; casts keep monotonic-seq + timestamp
  validation on their own pair (duplicate-cast protection stays) and stop
  touching `last_seq`/`last_t`. Document `last_processed_seq` as move-only on
  `ServerMsg::Snapshot` in `game/vordar-protocol/src/lib.rs`. Semantic wire
  change → bump `PROTOCOL_VERSION`.
- **Path:** (1) client: separate cast counter in `NetClientState`;
  (2) server: `cast_seq`/`cast_t` fields, `dispatch_cast` validates against
  them only; (3) protocol doc comment + version bump; (4) e2e: hold back the
  concurrent move datagram while the cast is delivered on-stream, assert the
  move still applies on the next batch; (5) existing prediction/e2e suites
  green.

### 2. Arrival deadline floors at `MAX_REWIND` permanently, not only while RTT is unwarmed

- **Evidence:** `server/vordar-server/src/net/receive.rs:640` — `let max_age
  = rtt.max(MAX_REWIND_MICROS) + ARRIVAL_MARGIN_MICROS;` with
  `MAX_REWIND_MICROS = 200_000` (`net/mod.rs:49`) and `ARRIVAL_MARGIN_MICROS
  = 100_000` (`receive.rs:34`) — every client, at any RTT, gets a 300 ms
  accept window. The comment above it claims "MAX_REWIND acts as a floor
  while RTT estimates settle"; the code never stops flooring. DESIGN §3:
  inputs must arrive within ~one RTT after T, compensation capped at
  `min(measured RTT, ~200 ms)`. The 2026-07-11 audit (finding 16) caught
  this same drift and fixed only the doc side; this is the code half.
- **Ideal:** arrival acceptance ≈ `measured_rtt + margin`, with the 200 ms
  value serving as a bootstrap floor only while `rtt == 0`, so a low-RTT
  client cannot backdate stamps 200 ms for maximum favor-the-defender.
- **Gap:** the exploit is bounded — `mechanics.rs:71` caps resolve rewind at
  `now − MAX_REWIND` regardless of stamps — so backdating buys at most what
  a legitimately high-RTT player gets. But that is still a permanent,
  intentional-lag-sized divergence from DESIGN's anti-cheat rule on every
  low-RTT connection. Note the external review's proposed fix capped arrival
  age *at* `MAX_REWIND` — that direction is wrong: it would reject every
  intent from a legitimate >~300 ms-RTT client whose inputs genuinely arrive
  one RTT late. Floor-only-while-unwarmed is the correct shape.
- **Suggestion:** `max_age = if rtt == 0 { MAX_REWIND_MICROS } else { rtt } +
  ARRIVAL_MARGIN_MICROS`; fix the comment at `receive.rs:638` to match.
- **Path:** (1) change `validate_intent`; (2) comment; (3) unit tests
  pinning accept/reject boundaries at rtt = 20 ms and 180 ms (both sides of
  each boundary); (4) existing e2e security suite green.

### 3. Persistence fails open on hostile or corrupt rows

- **Evidence:** three verified sub-holes. Health unclamped:
  `server/vordar-server/src/net/receive.rs:473-475` applies `hp.current =
  record.health` with no `[0, max]` clamp — a tampered row grants arbitrary
  or negative health. Cooldowns fail open: `db.rs:318-321` — cooldown-blob
  parse error → `unwrap_or_else(... HashMap::new())`, resetting every
  cooldown. FK enforcement off: `db.rs:154-158` sets only
  `journal_mode/synchronous/busy_timeout`; the `REFERENCES accounts(id)` FK
  from migration 2 (`db.rs:47`) is unenforced without
  `PRAGMA foreign_keys=ON`.
- **Ideal:** a hostile or corrupt DB row can degrade a character's state,
  never elevate it: health clamps into the live prefab's bounds, cooldown
  corruption locks abilities out rather than resetting them, and the schema's
  declared integrity is enforced.
- **Gap:** the DB file is the single trust boundary of the local pack; all
  three holes fail toward the attacker. (Dropped from the source review's
  bundle: xp anti-rollback — xp only enters via server systems, so its only
  bad writer is the same direct-tamper threat, and inflated xp has no live
  combat effect; and the `synchronous=NORMAL` durability tradeoff, already
  documented where it lives, `db.rs:148-153`.)
- **Suggestion:** (1) clamp `record.health` into `[1, hp.max]` at the apply
  site (floor 1: a ≤0 row would otherwise spawn a corpse that instantly
  death-respawns); (2) cooldown parse error → flag on `CharacterRecord` →
  full-cooldown lockout at spawn in `complete_db_load`, where the
  `ClassLibrary` is already in scope (the pre-rework-1 pessimistic-cooldown
  shape, commit `4a49adb`); (3) add `PRAGMA foreign_keys = ON;` to the
  `db.rs:154` batch.
- **Path:** (1) three changes above; (2) hostile-row tests: tampered health
  (negative, and > max) and corrupt cooldown blob, asserting clamp and
  lockout — this also closes the stage-appropriate slice of the source
  review's test-coverage gap list; (3) persistence e2e green.

### 4. A connection that never logs in is only reaped by the 30 s transport idle timeout

- **Evidence:** `server/vordar-server/src/net/receive.rs:74-79` —
  `ServerEvent::Connected` only logs "awaiting login"; no login deadline
  exists anywhere. The sole reaper is the transport idle timeout
  (`smirk/engine-net/src/common.rs:127-130`), which any traffic — including
  QUIC keepalives — resets, so a client can hold per-IP slots indefinitely
  without authenticating.
- **Ideal:** an unauthenticated connection is a bounded resource: handshake
  → login within a deadline, or the server closes it.
- **Gap:** pre-login slot-holding is the one connection state with no budget.
  (The source review also wanted duplicate `Login` answered with an error;
  rejected — the protocol is deliberately client-closes on `LoginDenied`
  (`game/vordar-protocol/src/lib.rs:153-156`), so denying a duplicate from a
  buggy-but-authenticated client would kill a healthy session. Silent ignore
  at `receive.rs:155-158` is correct.)
- **Suggestion:** record `now_micros()` per conn at `Connected` in a pending
  map on `NetServerState`; each `NetReceiveSystem` tick, kick conns older
  than ~10 s that are in neither `conns` nor `loading`; remove entries on
  login/disconnect.
- **Path:** (1) pending map + reaper; (2) test: a connection that handshakes
  and never logs in is closed within the deadline, while one mid-login is
  not; (3) flood/security suites green.

### 5. Token comparisons are not constant-time

- **Evidence:** `server/vordar-server/src/db.rs:366` (`if claimed != hash`,
  digest compare) and `server/vordar-server/src/net/receive.rs:174`, `:199`
  (raw `[u8; 32]` session-token compares). No constant-time primitive in the
  workspace (`subtle` present only transitively).
- **Ideal:** every comparison of auth material is constant-time, so timing
  can never become a channel regardless of future deployment.
- **Gap:** table-stakes hygiene rather than an exploitable hole (the source
  review's High is overstated): the db compare is digest-vs-digest of an
  attacker-chosen preimage — prefixes aren't steerable — and the session-path
  compares each burn the 5-failures/10 s/IP login budget
  (`net/login.rs:6-9`), making byte-position recovery impractical. Cheap to
  close properly now.
- **Suggestion:** hash the presented token once and compare digests with
  `subtle::ConstantTimeEq` at all three sites (or hash-both-sides at the
  session sites to avoid holding raw tokens in the compare at all).
- **Path:** (1) add `subtle` (already in the lock file transitively);
  (2) three sites; (3) unit test: mismatched token still denied; suites
  green.

### 6. `CastIntent.skill` is an unbounded string

- **Evidence:** the skill id is a bare `String` bounded only by the 1 KiB
  frame cap (`smirk/engine-net/src/common.rs:10`); unknown skills log and
  return (`server/vordar-server/src/net/receive.rs:270-272`), burning
  decode/lookup per message. Verified: a hostile string cannot poison
  `cooldown_ready` — insertion happens only after a successful
  `ClassLibrary` lookup — so the residual is CPU only.
- **Ideal:** every wire string has an explicit, validated bound at the top of
  its handler.
- **Gap:** one-line asymmetry with the name rule (≤32 printable ASCII at
  login, `receive.rs:149`).
- **Suggestion:** reject `skill_id.len() > 64` at the top of `dispatch_cast`,
  counted through `record_reject`.
- **Path:** (1) the check; (2) unit test; done.

### 7. No RTT-variance tracking despite DESIGN §3's day-one clause

- **Evidence:** per-conn RTT is a single smoothed point value written per app
  datagram/frame (`smirk/engine-net/src/server.rs:624`, `:580`) and read
  once per intent (`receive.rs:92`, `:257`). No history, no variance, no
  flag when RTT jumps coincident with `MechanicScheduled`/resolve. DESIGN §3
  lists RTT-variance monitoring in the build-in-from-day-one set.
- **Ideal:** each connection carries an RTT baseline (mean + variance) and
  mechanic-window spikes produce structured log samples — detection
  machinery in place before there are players to detect.
- **Gap:** the capped rewind neuters infinite lag-switch; oscillating delay
  to widen forgiveness only during telegraphs is invisible. Metrics-only
  scope — no gameplay policy, no bans from netcode.
- **Suggestion:** maintain per-conn EWMA mean/variance beside the existing
  atomic; on cast arrival and mechanic resolve, log a structured sample when
  current RTT exceeds baseline by k·σ.
- **Path:** (1) EWMA pair + update site; (2) flag at the two consumers;
  (3) unit test on the estimator; suites green.

### 8. Datagram-lane failures are counted but never surfaced; snapshot bytes unmeasured

- **Evidence:** `smirk/engine-net/src/metrics.rs:23-30` defines
  `datagrams_in/out` and `datagram_send_failures`, recorded at
  `server.rs:343`, `:566` and `client.rs:264` — and the periodic metrics log
  (`server/vordar-server/src/net/broadcast.rs:91-100`) omits all three. No
  encode-size check exists on the `Snapshot` datagram (`broadcast.rs:214-218`);
  `MAX_FRAME_OUT` guards streams only.
- **Ideal:** sustained datagram loss (PMTU black holes, path failure) is an
  ops signal, not something the client's interpolation silently papers over;
  snapshot size is pinned against the WAN budget by an assert, not by
  arithmetic in reviewers' heads.
- **Gap:** visibility only — the budget itself is healthy (`EntityPos` ≤ 12 B
  pinned by test, `vordar-protocol/src/lib.rs:453-459`; 64-state worst case
  ≈ 800 B; rework 5 measured 576 B steady-state crowds against a 1200 B
  target).
- **Suggestion:** add the three datagram counters to the existing periodic
  metrics line; debug-assert (and gauge) encoded snapshot length ≤ 1200 B at
  the send site. No fragmentation work — headroom is 35 %.
- **Path:** (1) metrics line; (2) assert/gauge; (3) crowd-snapshot test
  asserts the gauge; done.

### 9. `HitResult` is log-only on the client

- **Evidence:** `client/vordar-client/src/net/lifecycle.rs:107-109` —
  `ServerMsg::HitResult` → `log::info!`; hit feedback rides 10 Hz snapshot
  HP deltas only, though the server already fans the message out AOI-scoped.
- **Ideal:** instantaneous hit confirmation from the event the server
  already sends; damage authority stays on HP replication.
- **Gap:** presentation latency only — not a correctness bug; last in queue.
- **Suggestion:** drive a brief hit-confirm flash (existing VFX burst path)
  for entities in the known set on `HitResult`; HP/death continue to come
  from snapshots.
- **Path:** (1) wire `HitResult` into the client VFX seam; (2) unit test on
  the handler; visual check in sandbox/zone review.

## Deferred until multiplayer (verified once — not in the queue)

Facts checked on the current tree so this section is not re-audited later.
Triggers: real accounts/hosting work, or any non-loopback deployment.

- **TLS trust** — `SkipServerVerification` accepts any cert
  (`smirk/engine-net/src/common.rs:143`, `:155-165`; per-boot self-signed at
  `:98`). Already queued as **rework 6** in
  `reworks-networking-2026-07-11.md` (hostname-carrying Redirect,
  feature-gated dev verifier, real CA) — execute that plan; fold in: refuse
  non-loopback bind while the dev verifier is compiled in (default bind is
  `127.0.0.1:5151`, `main.rs:33-37`, zone ports `base + i`), and consider
  binding session tokens to a TLS channel exporter.
- **TOFU auth** — name claim = ownership forever; token is a plaintext-hex
  bearer file client-side (`client/vordar-client/src/credentials.rs:45-56`),
  SHA-256 hash at rest server-side (`db.rs:341`). Verified nuance: unsalted
  SHA-256 of a random 32-byte token is cryptographically fine at rest — the
  real gaps are lifecycle (rotation, revocation, recovery, multi-device).
  Replacement is DESIGN §8's gateway (argon2id or OAuth → short-lived zone
  tickets).
- **Transfer tickets** — Redirect is save-then-`{zone, addr}` with no signed
  proof (`net/transfer.rs:67-71`; login re-routing off DB zone at
  `receive.rs:427-440`). Verified: the addr is not client-influenceable
  today; MITM amplification is entirely the TLS item's. Ticket design is
  DESIGN §8 verbatim.
- **Cross-zone session uniqueness** — the review claimed two zones can hold
  one character concurrently; **refuted for the current tree**. Uniqueness is
  emergent: every login routes by DB zone ownership (`receive.rs:427-440`
  Redirects before spawn), a session only saves its own zone, transfer
  removes the `PlayerConn` in the same tick it saves the target zone, and
  the single FIFO `DbWorker` orders every save before the next load. The
  invariant rests on exactly two assumptions — one process, one FIFO DB
  channel — which the fleet design must replace with an explicit session
  lease (CAS-acquire, heartbeat, revocation).
- **Process isolation** — one process, thread-per-zone, shared DbWorker;
  `catch_unwind` restart with budget 3; dead-listener redirect after budget
  exhaustion is a known, logged state (`supervisor.rs:19-29`). Process-per-
  zone and a live directory are DESIGN §8 coordinator work; the short-term
  "circuit-break directory entries" idea was already investigated and
  rejected in rework 10's evidence (no shared-mutable directory exists by
  design).
- **Anti-cheat telemetry beyond finding 7** — verified implemented: intents-
  only wire, seq/time monotonicity, 50 ms future slack, dir cap, queue cap
  16, server-time scheduling, applied-velocity rewind capped at 200 ms,
  AOI-scoped combat meta. Correction to the review: live movement IS
  collision-validated (intents integrate through the full sim); the only
  uncollided path is the ≤200 ms rewind reconstruction (`mechanics.rs`
  replays `step` + `PlayRadius` without statics). Behavioral detection and
  rewind-tightening policy need a design pass at hardening time.
- **World clock authority** — per-process `Instant` origin (`main.rs:51`);
  restart resets world time. The seam already exists (all reads go through
  `NetServerState::world_at`/`world_micros`, `net/mod.rs:254-262`) — no
  clock-source trait now; pin process clocks to coordinator samples the way
  clients pin to server when a second process appears.
- **Fleet-scale snapshot cost** — thread-per-zone + AOI + stagger is the
  right dev shape; past ~100 concurrent, pre-sorted AOI gather, encode
  scratch reuse, and interest tiers are the candidates, gated on the soak
  suite's `net_busy_pct` (13.6 % at 200 bots today).
- **Capability negotiation** — single `u8` version with hard `Reject` is
  correct pre-launch; reserve a capabilities bitfield or min/max range in
  `Hello` before external playtests.
- **DoS remainder** — flood baseline verified live (buckets, caps, retry
  validation, login limiter). Unbounded account-row creation from fresh-name
  spam is real but WAN-only; creation rate-limiting belongs with the gateway,
  which removes direct zone `Login` anyway.
- **Secrets/ops/durability** — credentials-file ACLs/keychain, Linux
  `deny.toml` triple (the file's own comment names the trigger: "when server
  hosting lands"), backup/migrate runbook, RPO ≈ one 30 s autosave window,
  sync-acked critical saves (fold into rework 8's durability-classes design).
  Near-free piece if a hygiene pass wants it: `#![forbid(unsafe_code)]` on
  vordar-server/vordar-protocol/vordar-game (grep-verified zero `unsafe`
  today). Fleet-class security tests (cross-zone twin, transfer forgery,
  MITM regression) land with their respective reworks.

## Not extracted

- 01-F6 (late AOI enterers miss telegraphs) — deliberate, documented accept
  (`net/mod.rs:299-306`); only wrong for raid-length tells, which don't
  exist. Re-file when raid-scale `cast_micros` content is designed.
- 01-F9/F10/F11/F13/F14/F16/F18 and the security review's strengths list —
  preserve-grade entries, verified accurate, condensed into the intro.
- 05-F15 (protocol trust boundaries) — strengths-grade; the residue
  (decode-fuzz, ServerMsg-rejection property tests) is low-value: postcard
  decode already returns `Option` and malformed input drops with a log.
- 05-F18 (credentials file structure) — informational; structure documented
  here and in code, no action.
- Duplicate-`Login` error reply (half of 05-F12) — rejected: contradicts the
  deliberate client-closes protocol design (see finding 4).
