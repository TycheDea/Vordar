# Expert Review: MMO Netcode & Synchronization
**Reviewer persona:** Principal MMO Netcode Engineer
**Date:** 2026-07-27
**Scope:** engine-net, protocol, server net, client net, combat timing sync

## Executive summary

Vordar’s multiplayer stack is unusually coherent for an early MMO-shaped action RPG: QUIC dual-lane transport, opaque game payloads, NTP-style clock sync with drift slew, intent-only authority, AOI fan-out, fixed-delay remote playback, and scheduled-snapshot combat that matches DESIGN.md’s “T = telegraph completion” rule. The architecture deliberately deletes fighting-game rollback and instead spends complexity where this combat model needs it — clock discipline, arrival deadlines, and stamp-based rewind.

The strongest risks are not “missing prediction,” but **cross-lane sequence coupling** (casts on the reliable stream share one `seq` with moves on datagrams), **arrival-deadline floor semantics that diverge from DESIGN’s “~one RTT” wording**, and **scale/topology pieces still single-process** (no coordinator, per-process world clock origin, dev TLS). Test coverage for transport impairment, reconciliation, and combat e2e is better than most mid-stage titles; lag-switch variance monitoring and multi-zone fleet failure modes remain thin.

## Findings

### F1. [SEVERITY: Critical] Shared monotonic `seq` across stream casts and datagram moves can drop in-flight movement
- **Where:** `client/vordar-client/src/net/mod.rs` (`NetClientState::seq`, `send_cast_intent`); `client/vordar-client/src/net/prediction.rs` (`NetSendInputSystem`); `server/vordar-server/src/net/receive.rs` (`validate_intent`, `queue_move_intents`, `dispatch_cast`); `game/vordar-protocol/src/lib.rs` (`ClientMsg::MoveIntents` / `CastIntent`)
- **What:** One client counter stamps both lanes. Server validation is a single `PlayerConn::last_seq` stream. Casts ride the reliable ordered stream and advance `last_seq` immediately; moves ride unreliable datagrams with last-3 redundancy. Under normal reorder (datagram delayed relative to stream), a cast with `seq = N+1` can arrive before move `seq = N`. `queue_move_intents` then treats `N` as “expected redundancy” (`seq <= pc.last_seq` → silent skip). That move is never applied, never acked via a matching `applied_seq` path that could repair it, and last-3 resend still carries a seq the server will keep ignoring.
- **Why it matters:** One cast under loss/jitter can erase a tick of authoritative movement. Prediction will eventually snap/smooth, but combat fairness (position at T) and “bit-for-bit replay” assumptions break. This is a classic dual-lane seq-space bug.
- **Recommendation:** Split sequence spaces (`move_seq` / `cast_seq`) or keep one space but **do not advance movement monotonicity from cast**. Casts should validate on their own counter (or a cast-only gate: cooldown/range/class) and must not participate in `last_seq` used by `MoveIntents` dedupe. Add an e2e test: cast on stream while holding back the concurrent move datagram; assert the move still applies.

### F2. [SEVERITY: High] Arrival deadline uses `max(RTT, MAX_REWIND)` — always ≥ ~300 ms, not “~one RTT”
- **Where:** `server/vordar-server/src/net/receive.rs` (`validate_intent`); `server/vordar-server/src/net/mod.rs` (`MAX_REWIND_MICROS = 200_000`); constants `ARRIVAL_MARGIN_MICROS = 100_000`
- **What:** DESIGN.md §3 says an input claiming time T must arrive within ~one RTT after T, with compensation capped at `min(measured RTT, ~200 ms)`. Implementation does:
  - arrival age cap: `rtt.max(MAX_REWIND_MICROS) + ARRIVAL_MARGIN_MICROS`
  - resolve rewind: `t_eff = resolve_at.max(now.saturating_sub(MAX_REWIND))` in `mechanics.rs`
  So low-RTT clients always get a **200 ms floor + 100 ms margin** on arrival acceptance. Resolve rewind is capped at 200 ms from *now*, which is the right spirit, but arrival acceptance is more permissive than DESIGN’s anti-cheat wording.
- **Why it matters:** Arrival deadline is the primary “you can’t react to information you didn’t have” control. A permanent 300 ms accept window enables larger intentional backdating than a true one-RTT rule on LAN/good WAN. The resolve cap still bounds hit tests, but forged early stamps that arrive inside the wide window can still bias favor-the-defender rewind.
- **Recommendation:** Align with DESIGN: arrival max age ≈ `measured_rtt + skew_margin`, hard-capped by `MAX_REWIND`; keep `MAX_REWIND` as a **ceiling**, not a floor once RTT is warmed. Keep a short bootstrap floor only while `rtt_micros` is 0/unknown. Add tests at RTT=20 ms and RTT=180 ms that pin accept/reject boundaries.

### F3. [SEVERITY: High] No RTT-variance / lag-switch monitoring despite DESIGN day-one discipline
- **Where:** DESIGN.md §3 (“RTT-variance monitoring”); `smirk/engine-net/src/server.rs` (`rtt_micros` stores `connection.rtt()` per app datagram); `server/vordar-server/src/net/receive.rs` (consumes point RTT only)
- **What:** Server exposes smoothed QUIC RTT per connection and uses the instantaneous value in `validate_intent`. There is no EWMA/variance tracker, no spike flag during mechanic windows, and no policy hook when RTT jumps coincident with `MechanicScheduled` / resolve.
- **Why it matters:** Capped rewind neuters infinite lag-switch; it does not detect players who oscillate delay to widen effective forgiveness only during telegraphs. At RO-scale contested objectives this becomes a support/ops problem.
- **Recommendation:** Maintain per-conn RTT mean/variance (or P50/P95). On mechanic schedule/resolve, if current RTT ≫ baseline, clamp rewind to baseline or flag. Log structured samples; don’t ban from netcode alone.

### F4. [SEVERITY: Medium] Cast intents advance `last_seq` but never `applied_seq` — ack semantics are move-only while seq is shared
- **Where:** `server/vordar-server/src/net/receive.rs` (`dispatch_cast` sets `pc.last_seq`; `drain_intents` sets `pc.applied_seq`); `server/vordar-server/src/net/broadcast.rs` (`last_processed_seq = pc.applied_seq`); `client/vordar-client/src/net/prediction.rs` (`pending.retain(|p| p.seq > last_processed_seq)`)
- **What:** Snapshot ack is “highest **applied move** seq.” Casts consume seq numbers without becoming acks. With F1 fixed via split counters this becomes clean; with shared counters, client pending filtering still works only because casts are not pushed into `pending` — a fragile invariant (`send_cast_intent` must never record pending).
- **Why it matters:** Future “predict cast windup” or shared input buffer work will trip over ack holes (seq 10 move applied, 11 cast, 12 move → ack jumps 10→12). Debugging “why didn’t seq 11 ack?” will be painful.
- **Recommendation:** After splitting seq spaces, document `last_processed_seq` as move-only in the protocol comment. If cast prediction is added later, introduce `last_processed_cast_seq` or a unified input log with typed entries.

### F5. [SEVERITY: Medium] World clock authority is per-process `Instant`, not a coordinator clock
- **Where:** `server/vordar-server/src/main.rs` (`world_origin = Instant::now()` shared across zone threads); `server/vordar-server/src/net/mod.rs` (`world_offset_micros`); DESIGN.md §8 (coordinator owns authoritative world clock)
- **What:** Multi-zone today is one process, N zone threads, one shared `Instant` origin — correct for that topology. There is no cross-process clock, no heartbeat skew correction, and restart resets world time (known DESIGN gap).
- **Why it matters:** The moment zones split across machines, day/night and world events diverge unless a coordinator (or HLC/NTP service) appears. Portal Redirect + shared DB ordering are partly ready; global time is not.
- **Recommendation:** Keep `WorldClock { world_micros, at_server_micros }` as the client API. Introduce a process-level clock source trait now (`fn world_micros()`) so coordinator injection doesn’t rewrite broadcast. On multi-process, pin world time to coordinator samples the way clients pin to server.

### F6. [SEVERITY: Medium] Interest management is solid AOI, but telegraphs/hits are fire-and-forget for late enterers
- **Where:** `server/vordar-server/src/net/mod.rs` (`aoi_conns`, `AOI_RADIUS = 40.0`); `receive.rs` / `mechanics.rs` (MechanicScheduled / HitResult fan-out); comments explicitly accept miss-if-late
- **What:** Snapshots diff `known` with spatial grid + exact radius; crowd throttle keeps identity complete while capping `states` to 64 (32 nearest + RR). Mechanics only go to connections currently in AOI of the center. A client walking into range mid-telegraph never receives `MechanicScheduled`.
- **Why it matters:** Cosmetic for short telegraphs; wrong for long raid tells or “I walked into the arena and saw no floor.” Also a mild info asymmetry (insiders see countdown; late arrivals only see impact VFX if anything).
- **Recommendation:** For cast_micros above a threshold, either (a) include active mechanics in AOI enter payload, or (b) retain short-lived mechanic replicas in the zone and attach them on enter. Keep hit resolution server-side either way.

### F7. [SEVERITY: Medium] Snapshot datagram size vs `MAX_FRAME_OUT` / quinn datagram MTU not explicitly budgeted end-to-end
- **Where:** `smirk/engine-net/src/common.rs` (`MAX_FRAME_OUT = 64 KiB` stream); `server/vordar-server/src/net/broadcast.rs` (`MAX_SNAPSHOT_STATES = 64`); `NetServer::send_datagram` (failures counted, no fallback)
- **What:** Stream frames are hard-capped; datagrams are best-effort. 64 quantized `EntityPos` is usually small (protocol test targets ≤12 B/entity), but there is no server-side encode-size assert, no fragmentation strategy, and no client metric when snapshots vanish due to `datagram_send_failures` under PATH MTU pressure.
- **Why it matters:** Silent snapshot loss looks like “interpolation held at cap” (client already handles loss), but sustained MTU black holes degrade AOI motion for everyone on that path without a clear ops signal tied to gameplay.
- **Recommendation:** After encode, assert/metric snapshot byte length; alert if `datagram_send_failures` rises. Keep ≤1200 B payload target for WAN. If budget blows, shrink `MAX_SNAPSHOT_STATES` or split nearest/RR across staggered ticks (already have STAGGER).

### F8. [SEVERITY: Medium] Dev TLS model is correct for now, unsafe if shipped unchanged
- **Where:** `smirk/engine-net/src/common.rs` (`server_crypto` self-signed; `SkipServerVerification`); `ALPN = b"vordar/1"`
- **What:** Encryption on, authentication off. Documented. Handshake version is a single `u8` (`PROTOCOL_VERSION = 15`) with explicit `Reject` reason — good.
- **Why it matters:** Account tokens over MITM-able transport are forgeable by anyone on path once you leave localhost. Session takeover compares tokens (`receive.rs`) — attacker with MITM wins.
- **Recommendation:** Gate `SkipServerVerification` behind a dev feature; production path requires real certs / TOFU pin. Consider binding session tokens to a finished TLS exporter or channel binding later.

### F9. [SEVERITY: Low] Clock sync implementation exceeds DESIGN and is a preserve-list item — with one operational caveat
- **Where:** `smirk/engine-net/src/clock.rs` (`ClockSync`: windowed-min RTT, least-squares drift, 2000 ppm slew); `client.rs` (datagram ping/pong, burst 8×100 ms then 10 s)
- **What:** Matches DESIGN’s NTP-style offset and “re-check occasionally,” and improves on naive all-time-best RTT. Pings bypass stream writer queues so RTT isn’t app-queue inflated. Unit tests cover lucky-sample pinning and slew-vs-step.
- **Why it matters:** Telegraph fill (`client/.../telegraph.rs`) and intent stamps depend on this. Slew avoids countdown jumps — correct for FF14-style presentation.
- **Recommendation:** Preserve. Add a metrics gauge for `|offset - target|` and estimated drift ppm in client dev overlay. Optionally tighten slew during pure UI (telegraphs) vs allow slightly faster catch-up after multi-second stalls (today `MAX_SLEW_PPM` needs ~seconds to absorb large steps).

### F10. [SEVERITY: Low] Remote entity playback is production-grade fixed-delay; own-entity prediction is carefully matched to shared sim
- **Where:** `client/vordar-client/src/net/interpolate.rs` (`INTERP_DELAY_TICKS`, slew, extrap cap, no rewind); `prediction.rs` (trust 0.3 / snap 1.0, leap-aware pending, wall replay via `predict_step`); `server/.../mechanics.rs` (`rewound_position` uses applied velocity including `LeapImpulse`)
- **What:** Online-play diagram is implemented: ~200 ms delay (2 snapshot intervals), capped extrapolation, terminal hold, dry-recovery splice on late samples. Server and client both record dash override velocity for rewind/replay — tested on both sides.
- **Why it matters:** This is the difference between “feels like an MMO” and “rubber-bands every cast.” Leap/Onslaught path shows real multiplayer combat engineering, not prototype lerp.
- **Recommendation:** Preserve constants as named protocol-adjacent docs. Consider exposing interp delay as a function of measured RTT (still ≥ 1 snapshot) for LAN snappier remotes — optional, not required for fairness.

### F11. [SEVERITY: Low] Intent application is 1-per-tick drain with queue cap — good fairness, possible slowdown under sustained delay
- **Where:** `server/vordar-server/src/net/receive.rs` (`INTENT_QUEUE_CAP = 16`, `drain_intents`); client emits one move intent per Input tick
- **What:** Server applies exactly one queued move per connection per sim tick so prediction replay matches. Queue >16 drops oldest. Empty queue = stand still one tick.
- **Why it matters:** Prevents speed hacks via burst intents. Under severe uplink jitter, players experience temporary input backlog then drop — correct security tradeoff; may feel like sticky movement if cap is hit often.
- **Recommendation:** Metric `queue_depth` / drops per conn. If WAN profiles show frequent cap hits, raise modestly (e.g. 24) rather than applying multiple intents/tick (which desyncs client replay).

### F12. [SEVERITY: Low] Protocol versioning is handshake-hard but not capability-negotiated
- **Where:** `game/vordar-protocol/src/lib.rs` (`PROTOCOL_VERSION: u8 = 15`); `engine-net` Hello/HelloAck/Reject; tests in `smirk/engine-net/tests/handshake.rs`
- **What:** Breaking changes require bump + dual-bin deploy. No minor/feature bits, no graceful degrade.
- **Why it matters:** Fine pre-launch; painful for live clients if hotfixes need wire changes without forcing full client update.
- **Recommendation:** Before external playtests, reserve a `Hello` capabilities bitfield or min/max version range. Keep postcard + single enum for now.

### F13. [SEVERITY: Info] Authority model matches DESIGN: intents in, state out, no client positions
- **Where:** `vordar-protocol` crate docs; `ClientMsg` variants; server movement via `MoveIntent` EventBus; cooldowns/range/class validated in `dispatch_cast`
- **What:** Wire cannot express a claimed position. Speed capped by unit `dir` + server `Player.speed`. Mechanics resolve server-side; HP on snapshots is cosmetic.
- **Why it matters:** Deletes an entire cheat class. Keep this invariant sacred as features land (gliders, knockbacks, vehicles).
- **Recommendation:** Lint/test that no `ClientMsg` gains a position field. Knockback should be server impulse replicated via snapshots/secondary events, not client-written transforms.

### F14. [SEVERITY: Info] Transport isolation is clean — game thread stays deterministic
- **Where:** `smirk/engine-net/src/lib.rs` (dedicated net thread, channel API); server `NetReceiveSystem` Phase::Input; client same; flood token buckets + per-IP conn caps + writer queue kick
- **What:** Simulation never touches quinn directly. Impairment harness enables latency/loss/jitter/skew tests. Busy-time canary on server net thread is an ops-ready saturation proxy.
- **Why it matters:** Correct MMO boundary. Enables soak (`server/vordar-server/tests/soak.rs`), loss probes, and flood tests without poisoning sim determinism.
- **Recommendation:** Preserve. Eventually export `NetMetrics` to a `/metrics` or log shipper; today periodic `log::info` in snapshot broadcast is a start.

### F15. [SEVERITY: Info] Scalability path is zone-thread + AOI + stagger; not yet fleet-scale
- **Where:** `server/vordar-server/src/main.rs` (one thread per zone, `base_port + i`); `broadcast.rs` (`STAGGER`, crowd `select_states`); `supervisor.rs` (panic restart budget); DESIGN §7–§8 (hundreds/zone, gateway, channels)
- **What:** Per-zone Apps, shared DB worker, Redirect for wrong-zone login and portals, session takeover, autosave. Snapshot CPU is sliced across ticks. No channel sharding, no gateway, no warm pool.
- **Why it matters:** Adequate for dev pack and small playtests. “Hundreds per zone” will stress AOI gather (full grid query per staggered conn), postcard encode, and reliable stream for enters — not the 60 Hz move drain.
- **Recommendation:** Before load tests past ~100 concurrent: pre-sort AOI by dist once, reuse encode scratch, consider interest tiers (players vs trash mobs rate). Keep one App = one (zone, channel) as the scale unit.

### F16. [SEVERITY: Info] Test coverage is a major asset; a few netcode gaps remain
- **Where:** `smirk/engine-net/tests/*` (handshake, flood, impairment, WAN profiles, crowd snapshots); `server/vordar-server/tests/*` (e2e combat/security/persistence/zones/loss/soak); `client/.../net/{prediction,interpolate,apply}.rs` unit tests; `mechanics.rs` dash rewind test
- **What:** Strong unit coverage for clock slew, replay/walls/dashes, stale snapshot tick guard, move last-3 dedupe, security login/intent rejects, scheduled AoE and Onslaught e2e, loss gap measurement.
- **Why it matters:** Netcode without tests rots. This suite is why the leap/rewind fix is trustworthy.
- **Gaps to close:**
  1. Cross-lane cast-vs-move reorder (F1).
  2. Arrival deadline vs RTT matrix (F2).
  3. Multi-zone clock agreement if/when processes split.
  4. Client behavior when `datagram_send_failures` / long snapshot drought coincides with active telegraph.
  5. Intentional clock-lie client (offset forced wrong) → server rejects + client display-only corruption.

### F17. [SEVERITY: Low] `HitResult` is largely log-only on the client
- **Where:** `client/vordar-client/src/net/lifecycle.rs` (`ServerMsg::HitResult` → `log::info`); HP still comes from snapshots
- **What:** Server broadcasts who was hit; client doesn’t drive hit-react from it (health deltas on snapshots do).
- **Why it matters:** Missed opportunity for instantaneous feedback when snapshot cadence is 10 Hz; not a correctness bug.
- **Recommendation:** Optional: flash hit confirms from `HitResult` for entities in AOI; keep damage authority on HP replication.

### F18. [SEVERITY: Low] Ordering/tick model is consistent at 60 Hz sim / 10 Hz snapshot
- **Where:** `vordar_protocol::{TICK_HZ, SNAPSHOT_HZ}`; server `POST_HZ = TICK_HZ`; `STAGGER = POST_HZ/SNAPSHOT_HZ`; client playback uses same constants
- **What:** Tick counter on server stamps both `AoiDelta` and `Snapshot`. Client drops non-monotonic snapshot ticks before applying ack. Reliable stream carries identity; datagrams carry latest-wins state — correct split for this genre.
- **Why it matters:** Avoids HOL blocking movement replication on reliable streams (a common early MMO mistake).
- **Recommendation:** Preserve. If snapshot Hz changes, only touch protocol constants — both sides already derive delay from them.

## Strengths worth preserving

1. **Scheduled-snapshot combat aligned to absolute server time** — identical `MechanicScheduled` broadcast, client telegraph fill = pure function of synced clock (`telegraph.rs`), resolve at T (`mechanics.rs`). This is the product’s netcode thesis and it is implemented, not merely designed.
2. **Intent-only wire protocol** with postcard versioning and explicit authority comments in `vordar-protocol`.
3. **Dual-lane QUIC** with deliberate message placement (moves/snapshots = datagrams; login/AOI/casts/mechanics = stream) and control pings on datagrams to keep clock RTT clean.
4. **ClockSync quality** — windowed minimum, drift estimate, slew limit, impairment-tested skew harness.
5. **Prediction/reconciliation parity with shared movement** including static collision and leap overrides on both client replay and server rewind.
6. **AOI + crowd throttling** that refuses to corrupt identity diffs when positions are budgeted.
7. **Operational seams** — `NetMetrics`, writer backlog kicks, per-IP connection caps, token buckets, zone supervisor restart, reconnect backoff, LoginDenied/Redirect client-close pattern (avoids kick races).
8. **Evidence-heavy tests** for the hard paths (dash rewind, jittered playback, stale datagram ack, security).

## Suggested priority order

1. **Fix F1 (split or isolate cast/move sequence spaces)** — correctness bug under normal lane reorder; add e2e before more combat skills ship.
2. **Realign F2 arrival deadline with DESIGN** (RTT ceiling, not floor) + matrix tests; keeps anti-cheat honest as population grows.
3. **Add F3 RTT variance sampling** at least as metrics/flags during mechanics.
4. **Snapshot byte-budget + datagram failure visibility (F7)** before WAN playtests.
5. **Abstract world clock source (F5)** before any second server process.
6. **Long-telegraph AOI catch-up (F6)** when raid content lands.
7. **Production TLS path (F8)** before non-LAN accounts matter.
8. **Keep investing in the preserve-list** (clock, playback, leap parity, intent-only protocol) — do not “simplify” these into generic snapshot interpolation.

---

*Evidence basis: `smirk/engine-net/**`, `game/vordar-protocol/src/lib.rs`, `server/vordar-server/src/{main,lib,supervisor,net/**}.rs`, `client/vordar-client/src/{net/**,telegraph,world_time,cast}.rs`, `.claude/DESIGN.md`, `docs/online-play.mmd`, `README.md`. No files under `docs/reviews/**` were consulted.*
