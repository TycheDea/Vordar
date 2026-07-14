# Plan: Every message class rides one reliable ordered stream — head-of-line blocking by design — 2026-07-13

Source: docs/reviews/networking/reworks-networking-2026-07-11.md finding 3.

## Ideal end state

Superseded state never waits behind a retransmit. Per-client snapshot state
(`states` + the intent ack) and clock ping/pong ride QUIC datagrams — a lost
datagram is simply skipped, because the next 100 ms cadence supersedes it.
Movement intents ride datagrams with last-3 redundancy, so a single lost
packet costs nothing and a client behind upstream loss never rubber-bands on
a stalled stream. The reliable ordered stream carries only identity and
transactional messages: `Login`, `CastIntent`, `Welcome`, `PrefabTable`, AOI
enters/leaves, `MechanicScheduled`, `HitResult`, `WorldClock`, `Redirect`,
`EntityDied`, `LoginDenied`. The loss probe, re-run at WAN RTT (200 ms), shows
snapshot gaps bounded by the cadence (multiples of 100 ms) instead of by
retransmit cycles, and BASELINE.md records the before/after evidence.

## Design decisions

- **Datagrams surface through the existing event vocabulary.** engine-net
  gains a datagram lane (`NetServer::send_datagram(conn, Vec<u8>)`,
  `NetClient::send_datagram(Vec<u8>)`, and per-connection receive tasks), but
  received datagrams surface as the SAME `ServerEvent::Message { recv_micros }`
  / `ClientEvent::Message` the stream uses. Payloads are opaque bytes; which
  messages tolerate loss/reorder is a protocol-crate concern, and every message
  routed to the lossy lane is designed idempotent/latest-wins. Rejected:
  separate `DatagramReceived` event variants — they would fork every consumer
  (sim loop, Bot, client) for zero information the self-describing `ServerMsg`/
  `ClientMsg` enums don't already carry.
- **Datagram wire format is `[u8 tag][payload]`** — no u32 length prefix
  (datagrams are self-delimiting). `TAG_CTRL`/`TAG_APP` keep their stream
  meanings, so ctrl ping/pong and app messages share the lane exactly like
  they share the stream.
- **Snapshot splits into an identity delta (stream) and a state update
  (datagram).** `ServerMsg::AoiDelta { tick, enters, leaves }` rides the
  stream (ordering with `PrefabTable`/`Welcome` is what makes the diff
  protocol sound) and is sent only when non-empty. `ServerMsg::Snapshot
  { tick, last_processed_seq, states }` rides a datagram every snapshot
  interval; the client keeps a per-connection `latest_state_tick` and drops
  any datagram whose tick is not strictly newer (per-connection ticks are
  strictly increasing — `NetServerState.tick` is monotonic and each conn is
  served on its stagger slice). `last_processed_seq` rides the datagram
  because the ack is itself superseded state; the client's
  `pending.retain(seq > ack)` is idempotent and the tick guard prevents
  regression under reorder. A state entry referencing an id the client
  doesn't know yet (datagram outran the enter, or arrived after the leave)
  is skipped — the client already does exactly this.
- **One datagram per snapshot, size enforced by the existing gate — no
  chunking, no fallback path.** The `crowd_snapshot_fits_datagram_budget`
  e2e gate (e2e.rs:1202) already proves the worst case (full 64-entry
  `states` budget) encodes ≤ 1100 B, under quinn's ≥ ~1150 B usable datagram
  floor at the 1200 B minimum QUIC MTU (rework 5 shrank steady state to
  ~576 B — the queue note's "MTU gate cleared"). `send_datagram` failures
  (connection closing, or a hypothetical `TooLarge`) are counted in a new
  `NetMetrics` counter and dropped — datagrams are best-effort by contract,
  and the next cadence supersedes. Rejected: multi-datagram chunking and
  stream-fallback-on-oversize — machinery for a case the size gate makes
  unreachable, and a fallback would quietly reintroduce the stream path.
- **Move intents batch with last-3 redundancy; casts stay on the stream.**
  `ClientMsg::MoveIntent` is REPLACED by `ClientMsg::MoveIntents
  { intents: Vec<MoveIntentEntry> }` (ascending seq, at most 3: this tick's
  plus the two previous), sent via datagram each Input tick. The server
  applies only entries with `seq > pc.last_seq` — already-seen entries are
  skipped SILENTLY (expected redundancy, not `record_reject` noise); genuine
  violations (non-monotonic stamp, future stamp, deadline miss) still reject.
  `CastIntent` and `Login` stay on the reliable stream: casts are
  transactional (a lost cast eats the input; a duplicate is a griefing
  vector), and the cross-lane seq interleave (a cast advancing `last_seq`
  past a late move datagram) costs at most one dropped move tick, which
  reconciliation already absorbs. Rejected: keeping the single-intent
  variant alongside the batch — one lane, one shape.
- **Clock ping/pong moves to the datagram lane on both sides.** The client
  pinger sends `Ctrl::Ping` as a datagram; the server's datagram receiver
  answers `Ctrl::Pong` via `send_datagram` directly — never through the
  per-connection writer queue, which is precisely the queueing delay the
  finding's Ideal calls out (server.rs today stamps `t_server` and then
  queues the pong behind snapshot frames). The server keeps answering pings
  that arrive on the stream (engine-net is game-agnostic transport; the arm
  already exists), but this workspace's client stops sending them there.
  Lost pings cost one sample; the 8-ping burst and 10 s recheck cadence
  absorb that.
- **Impairment covers the new lane.** UDP-level loss (`impair.rs`) already
  drops datagram-bearing packets below QUIC — and unlike stream frames they
  are genuinely gone, which is the point. Latency/jitter must be routed
  explicitly: client datagram sends go through their own `delay_reorder`
  pipeline (one_way + jitter, like the stream writer), and received
  datagrams are stamped and fed into the SAME `delay_reorder` →
  `ordered_in_rx` pipeline the stream reader uses, so one consumer loop
  handles both lanes.
- **Rate limiting: the server datagram receiver gets its own token bucket**
  with the same `MSG_BUCKET_CAPACITY`/`MSG_REFILL_PER_SEC` constants. Two
  independent buckets bound a flooding client at 2× the old ceiling — still
  bounded, and it avoids threading shared mutable bucket state across two
  tasks. The per-conn RTT atomic is additionally refreshed from app
  datagrams, since after this rework the stream carries almost no
  client→server traffic.
- **Protocol versioning:** client and server ship from one workspace and
  engine-net's handshake rejects mismatches, so no compatibility window is
  needed. `PROTOCOL_VERSION` bumps once per wire-shape step: 13 → 14
  (snapshot split), 14 → 15 (intent batch).
- **Finding-path step 1 is already done:** both-direction impairment landed
  with audit finding 17 (`Impairment { upstream_loss, jitter, clock_skew_ppm }`,
  `impair.rs` send-side drop, upstream loss probe). This plan starts at the
  finding's path step 2 (re-probe at WAN RTTs) and adds the transport lane
  the remaining steps need.

## Findings (execution order)

### 1. Loss probes measure WAN RTTs — the before-evidence at 200 ms

- **Evidence:** `server/vordar-server/tests/loss.rs:24` — `const RTT:
  Duration = Duration::from_millis(50)` is the only RTT either probe runs
  at; `loss_probe_inter_snapshot_gaps` iterates loss ∈ {0,1,3,5}% at that
  single RTT, and `docs/benchmarks/BASELINE.md:195-209` records only the
  50 ms envelope ("Re-probe if RTT or loss assumptions change materially").
  The finding's Gap: at 150–250 ms RTT one retransmit cycle exceeds the
  250 ms gate by arithmetic, but it has never been measured.
- **Ideal:** Both probes in `loss.rs` run an RTT × loss matrix — 50 ms
  (continuity with the existing baseline) and 200 ms (the WAN case) — and
  BASELINE.md's loss-probe section records the 200 ms rows as the
  stream-snapshot "before" numbers this rework's final step will be compared
  against.
- **Gap:** No probe row exists above 50 ms RTT; the head-of-line stall at
  WAN RTT is asserted by arithmetic, not measurement.
- **Suggestion:** Wrap each probe's existing per-loss loop in an outer
  `for rtt in [Duration::from_millis(50), Duration::from_millis(200)]`,
  threading `rtt` into `Bot::connect_impaired_as` /
  `Bot::connect_upstream_impaired_as` and the printed header. Keep windows
  and rates unchanged. Use distinct bot names per (rtt, loss) cell (e.g.
  `observer-200-5`). The downstream probe's server sim budget
  (`60 * 600` ticks = 10 min) already covers 8 × 30 s windows; the upstream
  probe's `60 * 200` (200 s) does NOT cover 10 × 8 s windows plus settle
  time comfortably — raise it to `60 * 400`. The upstream probe's
  EXTREME_LOSS-vs-baseline assertion must compare within the same RTT row
  (reset `baseline_max` at the top of each RTT iteration).
- **Path:** (1) restructure both probes in
  `server/vordar-server/tests/loss.rs` as the RTT × loss matrix; (2) run
  `cargo test -p vordar-server --release --test loss -- --ignored
  --nocapture` and capture the printed tables; (3) update
  `docs/benchmarks/BASELINE.md`'s "Packet-loss probe" section with the new
  matrix, explicitly labeling the 200 ms rows as the pre-datagram baseline
  for rework 3 and noting whether the 250/500 ms decision gate is breached
  at WAN RTT (arithmetic says p99 will be). The probe test itself (compiles,
  `--ignored` by default) is the regression artifact; the workspace stays
  green because nothing outside the ignored test and the doc changes.

### 2. engine-net grows a datagram lane (transport only — no callers change behavior)

- **Evidence:** `smirk/engine-net/src/common.rs:6-16` — the wire format is
  stream frames only (`[u32 len][u8 tag][payload]`); `server.rs:446-449`
  accepts exactly one bidi stream and `common.rs:109-111` caps concurrent
  streams at 1/0; there is no `send_datagram`/`read_datagram` anywhere in
  the crate. `client.rs:494-556` shows the two impairment pipelines
  (`delay_reorder` for writes and reads) that a new lane must pass through;
  `metrics.rs` has no datagram counters.
- **Ideal:** `NetServer::send_datagram(conn: ConnId, data: Vec<u8>)` and
  `NetClient::send_datagram(data: Vec<u8>)` exist; datagrams travel as
  `[u8 tag][payload]` (no length prefix); each side runs a per-connection
  datagram receive loop (`quinn::Connection::read_datagram`) that surfaces
  `TAG_APP` payloads as the existing `ServerEvent::Message { recv_micros }`
  / `ClientEvent::Message`, and answers/consumes `TAG_CTRL` symmetrically
  to the stream: the server replies to a datagram `Ctrl::Ping` with a
  `Ctrl::Pong` sent DIRECTLY via `connection.send_datagram` (bypassing the
  writer queue); the client's datagram receiver feeds the same
  `ordered_in_rx` consumer loop that already handles `Ctrl::Pong` and app
  messages. Client-side impairment applies: outbound datagrams get their own
  `delay_reorder` pipeline (one_way + jitter, fresh Jitter seed), inbound
  datagrams are stamped and pushed into the SAME `in_tx` channel the stream
  reader uses. `NetMetrics` gains `datagrams_in`, `datagrams_out`, and
  `datagram_send_failures` (a failed `send_datagram` — connection lost,
  too large — increments the counter and drops; never panics, never falls
  back to the stream). The lib.rs module doc's wire-format section describes
  both lanes. No vordar code calls any of this yet.
- **Gap:** The transport cannot express an unreliable send in either
  direction; everything shares the one stream and its writer queue.
- **Suggestion:** Server side: add `Outgoing::Datagram(ConnId, Vec<u8>)`;
  the router task calls `connection.send_datagram(bytes)` inline (quinn's
  send_datagram is synchronous and non-blocking) with the `TAG_APP` prefix
  applied at `NetServer::send_datagram`; spawn a datagram receive task per
  connection in `handle_connection` alongside the writer task, holding a
  clone of `events`, the conn's `rtt` atomic, `metrics`, and its OWN token
  bucket (same `MSG_BUCKET_CAPACITY`/`MSG_REFILL_PER_SEC` constants; the
  reader-loop bucket stays untouched); the task ends silently when
  `read_datagram` errors (the stream reader owns connection-teardown
  signaling), and `writer.abort()`-style cleanup aborts it on exit. Client
  side: a second unbounded out-channel for datagrams feeding
  `delay_reorder` → a small sender task calling `connection.send_datagram`;
  a datagram read task stamping arrivals into the existing `in_tx` (never
  sending `Err` — end silently so only the stream reader breaks the main
  loop). engine-net's Cargo.toml gains `bytes = "1"` (already in quinn's
  tree) for `Bytes::from(Vec<u8>)`. quinn 0.11 enables datagram support by
  default in `TransportConfig`; do not touch the existing stream caps.
- **Path:** (1) wire format + API + tasks as above in `common.rs`,
  `server.rs`, `client.rs`, `metrics.rs`, `lib.rs` docs; (2) new engine-net
  tests (in `server.rs`'s or a new `#[cfg(test)]` module, following
  `drop_closes_endpoint_notifies_client_and_releases_port`'s
  connect-poll-deadline style): (a) fail-first — server
  `send_datagram(conn, payload)` after `Connected`, client polls until
  `ClientEvent::Message(payload)` arrives (fails before the lane exists
  because the API doesn't compile/deliver); (b) client
  `send_datagram(payload)` → server polls for
  `ServerEvent::Message { data: payload, .. }` with a nonzero
  `recv_micros`; (c) a raw-quinn client (like the stalled-reader test)
  sends `[TAG_CTRL][Ctrl::Ping]` as a datagram and reads a datagram back,
  asserting it decodes as `Ctrl::Pong` — while `metrics().frames_out`
  (stream counter) stays 0, proving the pong bypassed the writer queue;
  (3) `cargo test -p engine-net` and full workspace green (no caller
  changed, zero behavior change elsewhere).

### 3. Clock pings ride the datagram lane

- **Evidence:** `smirk/engine-net/src/client.rs:519-534` — the pinger task
  enqueues `Ctrl::Ping` onto `write_tx`, the SAME delayed pipeline and
  QUIC stream the app frames use, so a ping/pong queues behind snapshot
  frames in both writer queues; the finding's Ideal calls this out
  explicitly ("Datagram pings also remove writer-queue delay from RTT
  samples"). Step 2 of this plan gave the server a datagram `Ping` → direct
  datagram `Pong` path and gave the client a datagram-out pipeline plus a
  shared inbound consumer that already handles `Ctrl::Pong` from either
  lane.
- **Ideal:** The client pinger (burst and steady-state, both loops in
  `client_main`) sends every `Ctrl::Ping` through the datagram-out pipeline
  instead of `write_tx`. RTT/offset samples flow exactly as before
  (`ClockSync::on_pong` is lane-agnostic); the reliable stream carries no
  ping/pong at all for this workspace's client. A lost ping/pong datagram
  costs one sample — the 8-ping burst and 10 s recheck absorb it.
- **Gap:** Pings still share the ordered stream, so a retransmitting stream
  inflates RTT samples with queueing delay that has nothing to do with the
  path.
- **Suggestion:** In `client_main`, point the pinger's send at the
  datagram-out channel (with `TAG_CTRL` framing and the same
  one_way + jitter deadline the pipeline applies). Keep the server's stream
  ping arm (`server.rs` reader loop `TAG_CTRL` case) as-is — engine-net is
  transport, other embedders may ping on-stream; nothing in this workspace
  does anymore.
- **Path:** (1) switch the pinger's two send sites; (2) fail-first
  engine-net test: connect a `NetClient` to a `NetServer`, poll until
  `client.server_offset_micros()` is `Some` (clock sync converged) AND
  assert the client's `metrics().frames_out` counter recorded no ctrl-ping
  stream frames — concretely: after convergence, the client's stream
  `frames_out` equals exactly the app frames the test itself sent (0),
  proving every ping traveled as a datagram (before this step the pinger's
  stream frames make `frames_out` ≥ 8 and the assertion fails); (3) run the
  existing impaired clock tests
  (`windowed_minimum_tracks_drift_past_an_early_lucky_sample` is pure;
  e2e/soak clock behavior is exercised by the vordar suites) — full
  workspace green.

### 4. Snapshot states + intent ack ride datagrams; AOI identity rides the stream (protocol v14)

- **Evidence:** `game/vordar-protocol/src/lib.rs:67-76` —
  `ServerMsg::Snapshot { tick, last_processed_seq, enters, leaves, states }`
  is one message on the one stream. Server:
  `server/vordar-server/src/net_plugin.rs:1287-1293` encodes it per-conn and
  `state.server.send(conn, ...)`s it. Client:
  `client/vordar-client/src/net.rs:295-297` → `apply_snapshot`
  (net.rs:507-615) processes enters, leaves, hp, lerps, and reconciliation
  from the single message. Bot: `server/vordar-server/tests/common/mod.rs:318-355`
  mirrors it. The loss probe (BASELINE.md:204-209) proved a lost packet
  stalls every later snapshot on the stream; e2e.rs:1202
  (`crowd_snapshot_fits_datagram_budget`) already gates the full-budget
  `states` frame at ≤ 1100 B for exactly this step.
- **Ideal:** Protocol v14: `ServerMsg::Snapshot` becomes
  `{ tick, last_processed_seq, states }` and travels via
  `NetServer::send_datagram` every snapshot interval; new
  `ServerMsg::AoiDelta { tick, enters, leaves }` travels via the stream and
  only when `enters`/`leaves` is non-empty (steady state sends no stream
  traffic at all). Client keeps `latest_state_tick: u64` in
  `NetClientState`, reset in `teardown_replicated_world`; a `Snapshot`
  whose tick is not strictly greater is dropped before any field is read
  (ack included). States referencing unknown ids are skipped (existing
  behavior). `PrefabTable`-before-first-enter ordering still holds (both on
  the stream). Bot mirrors all of it: `AoiDelta` maintains
  `last_snapshot`/`prefabs`/`last_hp` enter/leave bookkeeping; `Snapshot`
  (tick-guarded) drives `last_ack`, `snapshot_ticks`, `snapshot_at`,
  `snapshot_bytes`, `last_states`, positions, hp. `PROTOCOL_VERSION` = 14.
- **Gap:** Snapshots are reliable-ordered by construction, so one lost
  packet re-delivers superseded positions late and stalls everything behind
  it — the head-of-line blocking this whole rework exists to remove.
- **Suggestion:** Server (`SnapshotBroadcastSystem::run`, net_plugin.rs
  ~1233-1294): build `enters`/`leaves` exactly as today; if either is
  non-empty, `state.server.send(conn, encode(&ServerMsg::AoiDelta { tick,
  enters, leaves }))`; always `state.server.send_datagram(conn,
  encode(&ServerMsg::Snapshot { tick, last_processed_seq, states }))`.
  Client (`net.rs`): split `apply_snapshot` into `apply_aoi_delta`
  (enters/leaves — spawn/despawn, hp seed, `known` insert/remove) and
  `apply_states` (tick guard, hp, NetLerp/NetMotion, `reconcile_own`, ack);
  both operate on `NetClientState.entities`. Add the two match arms in
  `NetReceiveSystem`. Update the `bench` module's `apply_snapshot` seam to
  the split (benchmarks/benches call sites follow). Protocol tests:
  update `server_msg_roundtrip`/quantization/size tests to the new shapes
  and add an `AoiDelta` roundtrip. Keep `SNAPSHOT_HZ`, stagger, budget,
  `select_states` untouched.
- **Path:** (1) protocol change + version bump in
  `game/vordar-protocol/src/lib.rs` with updated roundtrip tests; (2) server
  send-site split in `net_plugin.rs`; (3) client split + `latest_state_tick`
  guard in `client/vordar-client/src/net.rs` (including
  `teardown_replicated_world` reset and the bench seam); (4) Bot split in
  `tests/common/mod.rs` — `snapshot_bytes`/`snapshot_at` now measure the
  datagram `Snapshot` (which is precisely what the loss probe and the e2e
  size gate are about; update the gate test's comments — its 1100 B
  assertion now literally measures the datagram payload, and the "initial
  enters wave" settle logic still holds since enters moved to `AoiDelta`);
  (5) fail-first client unit test: through the real receive path
  (`bench`/unit seam), apply a `Snapshot` at tick 20 placing an entity at
  P2, then a stale `Snapshot` at tick 10 with position P1 and a LOWER
  `last_processed_seq` — assert the entity's lerp target stays P2 and the
  pending-intent queue was not re-inflated (fails without the tick guard);
  (6) full test suite: e2e.rs, zones, soak smoke, client integration tests
  (`kicked_connection_reconnects_and_relogs_in`,
  `onslaught_dash_replay_never_snaps_at_150ms_rtt` exercise the full
  split path end-to-end) — workspace green.

### 5. Move intents ride datagrams with last-3 redundancy (protocol v15)

- **Evidence:** `game/vordar-protocol/src/lib.rs:32` —
  `ClientMsg::MoveIntent { seq, t_server_micros, dir }`, one per Input tick
  on the stream (`client/vordar-client/src/net.rs:1058`,
  `tests/common/mod.rs:413`). Server:
  `net_plugin.rs:559-583` validates and queues one intent per message;
  `validate_intent` (net_plugin.rs:908-935) treats `seq <= last_seq` as a
  reject and its caller calls `metrics().record_reject()`. The upstream loss
  probe (`loss.rs:118`) measured the resulting applied-intent lag under
  upstream loss — the client-felt rubber-banding the finding's Gap names.
- **Ideal:** Protocol v15: `ClientMsg::MoveIntent` is removed; new
  `ClientMsg::MoveIntents { intents: Vec<MoveIntentEntry> }` where
  `MoveIntentEntry { seq: u32, t_server_micros: u64, dir: Vec2 }`, carrying
  this tick's intent plus up to the two previous (ascending seq), sent via
  `NetClient::send_datagram` each Input tick. The server iterates entries in
  order: `seq <= pc.last_seq` entries are skipped silently (expected
  redundancy — no `record_reject`, no log); newer entries run the full
  `validate_intent` + dir-cap checks and enqueue exactly as today. A lost
  datagram is fully recovered by the next tick's batch; two consecutive
  losses cost at most one applied tick, which reconciliation absorbs.
  `CastIntent` and `Login` remain on the stream unchanged.
- **Gap:** Upstream loss stalls the ordered intent stream, so the server
  integrates stale movement while QUIC retransmits — rubber-banding scales
  with RTT; and intents queue behind `Login`/`CastIntent` frames.
- **Suggestion:** Server: extract a helper
  `fn queue_move_intents(pc: &mut PlayerConn, entries: &[MoveIntentEntry],
  recv_micros: u64, rtt: u64, metrics: &NetMetrics)` (pure over `PlayerConn`
  — unit-testable like `validate_intent`) that implements
  skip-silently/validate/queue per entry, and call it from a
  `ClientMsg::MoveIntents` arm replacing the `MoveIntent` arm. Client
  (`NetSendInputSystem`): keep a 3-deep ring of the last sent
  `MoveIntentEntry`s in `NetClientState` (cleared in
  `teardown_replicated_world` alongside `seq`), push this tick's entry,
  send the ring via `send_datagram`; the local prediction/pending-intent
  bookkeeping is untouched (one new PendingIntent per tick, exactly as
  today). Bot (`send_move`): same 3-deep ring, cleared on
  `follow_redirect`/reconnect construction, sent via
  `client.send_datagram`. `PROTOCOL_VERSION` = 15.
- **Path:** (1) protocol change + version bump + roundtrip test; (2) server
  arm + `queue_move_intents` helper; (3) fail-first server unit test (in
  net_plugin.rs's test module, PlayerConn constructed like
  `zero_seq_is_always_rejected`): a batch [5,6,7] queues three intents and
  sets `last_seq` = 7; a following batch [6,7,8] queues ONLY seq 8, and the
  metrics reject counter stays untouched by the 6/7 duplicates (fails if
  duplicates route through the reject path or re-queue); a batch containing
  a genuinely invalid entry (future stamp) still rejects that entry with
  `record_reject`; (4) client + Bot ring buffers and send-site switch;
  (5) new always-on e2e (e2e.rs): a bot with
  `Impairment { rtt: 40ms, upstream_loss: 0.3, .. }` walks +X for ~3 s
  sending one batch per 16 ms tick; assert its replicated displacement is
  ≥ 85% of the no-loss ideal (`applied ticks × speed × TICK_DT` measured by
  a parallel unimpaired control bot, or against the analytic distance) —
  fails without redundancy (~70% of intents apply) and passes with it
  (~97%+); (6) full workspace green, then optionally re-run the upstream
  loss probe to see the lag collapse (recorded properly in step 6).

### 6. After-evidence and documentation: re-probe, BASELINE, online-play diagram, queue note

- **Evidence:** Step 1 of this plan recorded the stream-snapshot WAN
  numbers in `docs/benchmarks/BASELINE.md`'s loss-probe section; the queue
  note in `docs/reviews/networking/reworks-networking-2026-07-11.md:30-33` requires
  every plan that changes the online-play flow to update
  `docs/online-play.mmd` + SVG (the diagram's edge labels at the bottom —
  "move · cast intents", "snapshots (prefab table indices) · welcome · hit
  results" — no longer describe the lanes); the memory rule
  "reworks-queue-mark-done" requires striking rework 3 in the queue note
  when the plan completes.
- **Ideal:** BASELINE.md shows the before/after matrix at 50 ms and 200 ms
  RTT proving datagram snapshots keep p99 gaps inside the 250 ms gate at
  WAN RTT where stream snapshots breached it, plus the upstream
  intent-lag improvement; the downstream probe gains a WAN-RTT assertion so
  a regression to stream-bound snapshots fails the probe; the online-play
  diagram names the two lanes and which messages ride each; the queue note
  strikes 3.
- **Gap:** After steps 2–5 the evidence and documentation still describe
  the single-stream world.
- **Suggestion:** Run both probes
  (`cargo test -p vordar-server --release --test loss -- --ignored
  --nocapture`), then edit docs. In `loss_probe_inter_snapshot_gaps`, add
  the decision-gate assertion for the datagram era: at every (rtt, loss)
  cell up to 5% loss, `p99 <= 250.0` ms — fails against pre-step-4 code at
  200 ms RTT by the recorded before-numbers, passes now (gaps become
  cadence multiples, p99 ≈ 100–200 ms). Update the mermaid diagram with the
  `mermaid-diagrams` skill and regenerate `docs/online-play.svg`: label the
  client→server intent edge "move intents · datagrams, last-3 redundancy"
  with a separate stream edge for "login · cast intents", and split the
  server→client edges into "state snapshots + ack · datagrams, latest-wins"
  vs "AOI enters/leaves · welcome · prefab table · hit results · reliable
  stream"; keep the diagram technology-agnostic in wording elsewhere.
- **Path:** (1) run both probes in release, capture tables; (2) add the
  ≤ 250 ms p99 gate assertion to the downstream probe; (3) rewrite
  BASELINE.md's loss-probe section as before/after (keeping the historical
  50 ms 2026-07-11 table for provenance) and update its "packet loss up to
  5%" bullet in the budget list; update the WEAKPOINTS #4 deferral note
  wherever BASELINE.md:204-209 recorded it ("the datagram snapshot path was
  **not built**" is now false); (4) update `docs/online-play.mmd` and
  regenerate the SVG; (5) strike rework 3 in the queue note of
  `docs/reviews/networking/reworks-networking-2026-07-11.md` with the plan filename
  and step count, following the existing strike format; (6) workspace green
  (probe remains `--ignored`; everything else untouched).
