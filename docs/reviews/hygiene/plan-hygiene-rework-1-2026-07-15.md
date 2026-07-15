# Plan: engine-net decomposition — clock filter out of client.rs, impairment unified into impair.rs — 2026-07-15

Source: docs/reviews/hygiene/reworks-hygiene-2026-07-15.md finding 1.

## Ideal end state

`smirk/engine-net/src/client.rs` is the client transport and task graph its name
promises: `NetClient`, `ClientEvent`, and `client_main`. A new
`smirk/engine-net/src/clock.rs` owns the clock-sync filter (`ClockSample`,
`ClockSync`, the sync cadence/window/slew constants) with its two unit tests.
`smirk/engine-net/src/impair.rs` owns *all* impairment — the `Impairment` knob set
and WAN profiles, `skewed_micros`, `Jitter`, `Pending`/`delay_reorder`, and the
existing below-QUIC lossy socket. `engine_net::Impairment` keeps resolving at the
crate root, so every external consumer (test-support's `Bot` presets, the
engine-net integration binaries, client net tests) compiles unchanged. In
`server.rs`, `handle_connection` reads as named stages, with the handshake — its
one linear, parameter-light seam — extracted to a named function.

## Design decisions

- **Compat surface is the crate root only.** The finding's constraint says
  "`engine_net::client::Impairment` etc. keep compiling", but `mod client;` is
  private in `lib.rs:40` — no `engine_net::client::*` path has ever been visible
  outside the crate. The only public path is the root re-export
  (`lib.rs:45 pub use client::{ClientEvent, Impairment, NetClient};`). So the
  whole compat story is: keep `engine_net::Impairment` resolving at the root by
  switching its re-export source to `impair`. No forwarding re-export inside
  `client.rs` is needed or wanted. Verified: every consumer
  (testing/test-support/src/bot.rs:1, smirk/engine-net/tests/{impairment,
  wan_profiles}.rs, server/vordar-server/tests/e2e_wireformat.rs:9,
  client/vordar-client/src/net/e2e.rs:380) imports via `engine_net::Impairment`.
- **Sync cadence constants move to clock.rs with the filter.** `SYNC_BURST_PINGS`,
  `SYNC_BURST_INTERVAL`, `SYNC_INTERVAL` (client.rs:28-32) are clock-sync policy,
  and the drift unit test uses `SYNC_INTERVAL` for its realistic cadence — keeping
  them in client.rs would force the moved test to reach back across modules.
  clock.rs owns the whole sync parameterization (`SYNC_WINDOW`, `MAX_SLEW_PPM`
  included); client.rs's pinger imports the cadence via `pub(crate)`.
  `HANDSHAKE_TIMEOUT` stays in client.rs — it is transport, not sync.
  Alternative rejected: cadence constants staying in client.rs (splits the sync
  design across two files and entangles clock.rs's test with client.rs).
- **Everything moves verbatim; only visibility, imports, and stale cross-file
  comment references change.** `ClockSync`/`Jitter`/`delay_reorder`/`skewed_micros`
  become `pub(crate)` (they were module-private siblings before; the module
  boundary now crosses them). Two rustdoc lines inside moved code name `impair.rs`
  as another file (client.rs:57-58 "(see `impair.rs`)", client.rs:126-128 "same
  LCG technique as `impair.rs`") — after the move those are self-references and
  must be repointed at `lossy_client_endpoint`/`LossySocket`, or they'd be stale
  claims. The impair.rs module header (impair.rs:1-7) currently describes only the
  lossy socket; it broadens to cover the whole conditioner. Both edits are the
  move's own consequence, not opportunistic comment work; the fixes-report comment
  findings (1-5) run independently and this rework does not depend on them —
  moved blocks carry whatever comments they have at execution time.
- **`handle_connection` gets the handshake extracted, and only that; the deeper
  staging is parked.** The finding leaves this seam to the planner. Reading
  server.rs:456-627: the writer task, datagram task, and reader loop are each
  `tokio::spawn`/loop blocks that capture 4-6 cloned `Arc` handles; extracting
  them to free functions trades one scrolling read for three parameter-soup
  signatures without reducing any coupling, on behavior-critical transport code —
  net negative. They already read as named stages via their bindings (`writer`,
  `datagram_task`, the reader `loop`) and section comments. The handshake
  (server.rs:475-499) is the one linear seam with a two-parameter signature and a
  subtle flush-before-close dance worth naming — it is extracted verbatim.
  The duplicated token-bucket logic (stream reader server.rs:598-615 vs datagram
  lane server.rs:558-572) is a real dedup candidate but a refactor, not a move —
  out of scope here; it belongs to a future fixes finding with flood_control.rs
  as its harness.
- **Step order: clock first, then impairment, then server.** The two client.rs
  extractions are independent (verified: no shared state; `skewed_micros` is used
  by the pinger and pong handler, the cadence constants by the pinger, but each
  import lands in its own step). Clock first matches the finding's listing and
  shrinks client.rs before the larger impairment move. The server step is last
  and independently landable — if it is descoped later, steps 1-2 still complete
  the finding's primary Ideal.
- **Green gate per step:** `cargo nextest run --workspace` at unchanged pass
  counts (last known 285/285; measure before the diff and require the same total
  after), `cargo test --doc --workspace` unchanged, and
  `cargo clippy -p engine-net --all-targets` introducing zero new warnings.
  The relevant behavioral pins: engine-net unit tests (2 clock, 3 impair,
  server tests), the impairment/wan_profiles/handshake/flood_control/
  crowd_snapshot integration binaries, server e2e suites (clock-sync waits in
  every binary, loss.rs's impaired observers), and client net e2e
  (`connect_impaired` observer at net/e2e.rs:380).

## Findings (execution order)

### 1. Move the clock-sync filter and its constants from client.rs to a new clock.rs

- **Evidence:** `smirk/engine-net/src/client.rs` holds the clock-sync filter as
  private siblings of the transport: cadence constants `SYNC_BURST_PINGS`,
  `SYNC_BURST_INTERVAL`, `SYNC_INTERVAL` (L28-32), filter constants `SYNC_WINDOW`
  (L35-40) and `MAX_SLEW_PPM` (L41-45), `struct ClockSample` (L221-228),
  `struct ClockSync` + impl (`new`, `on_pong`, `drift_rate`, `offset`, `rtt`,
  L230-314), and the two unit tests
  `windowed_minimum_tracks_drift_past_an_early_lucky_sample` and
  `offset_corrections_are_slewed_not_stepped` (L661-736, the entire
  `#[cfg(test)] mod tests`). Consumers inside client.rs: `NetClient.clock:
  Arc<Mutex<ClockSync>>` (L323), `connect_impaired` (L361), `client_main`'s
  pinger task (L569-583, uses the three cadence constants) and pong handler
  (L633-640). `smirk/engine-net/src/lib.rs:39-43` declares
  `mod common; mod client; mod impair; mod metrics; mod server;` — no clock
  module exists. Nothing outside client.rs references `ClockSync` (verified by
  workspace grep).
- **Ideal:** `smirk/engine-net/src/clock.rs` owns the sync filter and every sync
  constant; client.rs drives it through `pub(crate)` imports; the two unit tests
  live beside the filter they pin; client.rs no longer has a `mod tests`.
- **Gap:** the pure, unit-tested filter is buried in a 736-line transport file a
  reader would never open when hunting clock-sync behavior.
- **Suggestion:** verbatim move into a new module; visibility widened to
  `pub(crate)` only where client.rs consumes it; no logic edits, no comment
  rewording inside moved blocks except none is needed here (the moved doc
  comments reference nothing that changes meaning across the move).
- **Path:**
  1. Baseline: run `cargo nextest run --workspace` and record the total pass
     count (expected 285); run `cargo test --doc --workspace` and record its
     count.
  2. Create `smirk/engine-net/src/clock.rs` with this module header, then the
     moved code:
     ```
     // Clock-sync filter behind NetClient's published server-time offset —
     // windowed-minimum RTT sample selection, a least-squares drift-rate
     // estimate, and a slew limiter on the published offset. Pure and
     // network-free: client.rs feeds it Pong samples and reads the offset.
     // The sync cadence (initial burst, steady-state re-check) lives here
     // with the filter it parameterizes; client.rs's pinger drives it.
     ```
     Move verbatim from client.rs, in this order: `SYNC_BURST_PINGS`,
     `SYNC_BURST_INTERVAL`, `SYNC_INTERVAL` (each becomes `pub(crate) const`,
     doc comments unchanged), `SYNC_WINDOW`, `MAX_SLEW_PPM` (stay private),
     `ClockSample` (stays private), `ClockSync` + impl (`pub(crate) struct
     ClockSync`; `pub(crate) fn new/on_pong/offset/rtt`; `drift_rate` stays
     private), then the entire `#[cfg(test)] mod tests` block with both tests.
     Imports needed at the top of clock.rs: `use std::collections::VecDeque;`
     and `use std::time::Duration;`.
  3. In `smirk/engine-net/src/lib.rs`, add `mod clock;` after `mod client;`.
  4. In client.rs: delete the moved blocks; add
     `use crate::clock::{ClockSync, SYNC_BURST_INTERVAL, SYNC_BURST_PINGS, SYNC_INTERVAL};`;
     change `use std::collections::{BinaryHeap, VecDeque};` to
     `use std::collections::BinaryHeap;` (VecDeque was only ClockSync's).
     Update the now-stale file header (L1-2) to:
     `// NetClient — connecting side. Mirrors NetServer's thread/channel layout
     // and drives the clock-sync filter (clock.rs) from its ping/pong tasks.`
  5. Test (behavioral, existing): the moved unit tests now run as
     `engine_net clock::tests::windowed_minimum_tracks_drift_past_an_early_lucky_sample`
     and `clock::tests::offset_corrections_are_slewed_not_stepped` — they
     exercise `ClockSync` through its real API exactly as before. Live-path pin:
     `latency_reflected_in_rtt` in `server/vordar-server/tests/e2e.rs` (clock
     sync over a real connection under 150 ms simulated RTT) must still pass.
  6. Gate: `cargo nextest run --workspace` at the step-1 baseline count,
     `cargo test --doc --workspace` unchanged,
     `cargo clippy -p engine-net --all-targets` with zero new warnings. If any
     count differs, a test was dropped or renamed beyond the two expected
     module-path changes — stop and fix before proceeding.

### 2. Move the network conditioner (Impairment, skewed_micros, Jitter, Pending, delay_reorder) into impair.rs; re-export Impairment from impair

- **Evidence:** After step 1, `smirk/engine-net/src/client.rs` still holds the
  conditioner as siblings of the transport (pre-move line refs):
  `pub struct Impairment` + docs (L47-71), `impl Impairment` with `latency`,
  `wifi`, `four_g`, `satellite` (L73-112), `fn skewed_micros` (L114-124),
  `struct Jitter` + impl (`with_seed`, `sample`, L126-150), `struct Pending<T>`
  + `PartialEq`/`Eq`/`PartialOrd`/`Ord` impls (L152-177), and
  `async fn delay_reorder` (L179-219). `smirk/engine-net/src/impair.rs` (205
  lines) holds only `lossy_client_endpoint` + `LossySocket` + 3 unit tests; its
  header (L1-7) describes only the below-QUIC drop rationale.
  `lib.rs:45` re-exports `pub use client::{ClientEvent, Impairment, NetClient};`.
  client.rs consumers of the moved items: `connect`/`connect_with_latency`/
  `connect_impaired` signatures and `NetClient.clock_skew_ppm` (Impairment),
  `local_micros` L422-424 and the pinger/pong handler (skewed_micros), three
  `delay_reorder` spawns (L513, L542, L590), five `Jitter::with_seed` calls
  (L526, L556, L570, L602, L615), and
  `crate::impair::lossy_client_endpoint(...)` at L466 (already points at
  impair.rs — unchanged). Two moved doc comments reference impair.rs as another
  file: L57-58 "(see `impair.rs`)" and L126-128 "(same LCG technique as
  `impair.rs`)". External consumers all use the root path `engine_net::Impairment`:
  `testing/test-support/src/bot.rs:1,161,204,214,220-221`,
  `smirk/engine-net/tests/impairment.rs:13`, `tests/wan_profiles.rs:10`,
  `server/vordar-server/tests/e2e_wireformat.rs:9,213`,
  `client/vordar-client/src/net/e2e.rs:380`.
- **Ideal:** impair.rs owns all impairment; client.rs imports the four items it
  uses; `engine_net::Impairment` still resolves at the crate root, so every
  external construction site compiles with zero edits.
- **Gap:** the file *named* impairment holds a third of the machinery; the
  conditioner knobs and pipeline hide in the transport file.
- **Suggestion:** verbatim move; `Impairment` stays `pub`; `skewed_micros`,
  `Jitter` (+ its two methods), and `delay_reorder` become `pub(crate)`;
  `Pending` and its impls stay private (only `delay_reorder` uses them). Fix the
  two self-reference doc lines and broaden the module header — nothing else in
  the moved comments changes.
- **Path:**
  1. Baseline: record `cargo nextest run --workspace` and
     `cargo test --doc --workspace` counts.
  2. In `smirk/engine-net/src/impair.rs`, replace the L1-7 header with:
     ```
     // Network impairment for testing — the whole conditioner lives here:
     // the `Impairment` knob set with named WAN profiles, the `Jitter` +
     // `delay_reorder` delay/reorder pipeline stage client.rs inserts on each
     // lane, `skewed_micros` clock scaling for the skew harness, and a client
     // endpoint whose UDP send/receive paths drop datagrams BELOW QUIC.
     // Dropping at the UDP layer exercises the real retransmission machinery —
     // a dropped stream frame stalls the stream until QUIC retransmits it,
     // which is exactly the head-of-line phenomenon the loss probes measure.
     // (Dropping frames above QUIC, after reliable delivery, cannot reproduce
     // that.) Client-side receive drop == server→client loss; client-side send
     // drop == client→server loss.
     ```
  3. Move verbatim from client.rs into impair.rs, placed after the header and
     before `lossy_client_endpoint`, in this order: `Impairment` struct + docs,
     `impl Impairment`, `skewed_micros`, `Jitter` + impl, `Pending<T>` + its four
     trait impls, `delay_reorder`. Visibility: `pub struct Impairment` and its
     `pub fn` constructors unchanged; `pub(crate) fn skewed_micros`;
     `pub(crate) struct Jitter` with `pub(crate) fn with_seed` and
     `pub(crate) fn sample`; `Pending` private; `pub(crate) async fn
     delay_reorder`. Two doc-comment repointings inside the moved blocks (stale
     after the move): on `downstream_loss`, "(see `impair.rs`)" becomes
     "(see `lossy_client_endpoint`)"; on `Jitter`, "(same LCG technique as
     `impair.rs`)" becomes "(same LCG technique as `LossySocket`)".
  4. Add to impair.rs's imports: `use std::cmp::Ordering;`,
     `use std::collections::BinaryHeap;`, `use std::time::Duration;`, and
     `use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};`. Note: the
     existing `mod tests` imports `std::sync::atomic::Ordering` explicitly —
     that explicit import shadows the glob-imported `std::cmp::Ordering` from
     `use super::*`, so there is no ambiguity error; leave the test module
     untouched.
  5. In client.rs: delete the moved blocks; add
     `use crate::impair::{delay_reorder, skewed_micros, Impairment, Jitter};`;
     remove the now-unused `use std::cmp::Ordering;` and
     `use std::collections::BinaryHeap;` imports.
  6. In `smirk/engine-net/src/lib.rs`, change
     `pub use client::{ClientEvent, Impairment, NetClient};` to
     `pub use client::{ClientEvent, NetClient};` and add
     `pub use impair::Impairment;` on the next line.
  7. Test (behavioral, existing): the impairment integration binary
     (`smirk/engine-net/tests/impairment.rs` — jitter/reorder and
     `clock_skew_harness_skews_reported_local_time` drive `Jitter`,
     `delay_reorder`, and `skewed_micros` through a live `connect_impaired`
     connection) and `tests/wan_profiles.rs` (constructs `Impairment::wifi/
     four_g/satellite` via the root re-export) must pass unchanged. Constraint-4
     pin: `testing/test-support/src/bot.rs` compiles with zero edits (its
     `Impairment { .. }` literals at L204/L214 and `use engine_net::Impairment`
     at L1 are the crate's real external consumers).
  8. Gate: `cargo nextest run --workspace` at the baseline count,
     `cargo test --doc --workspace` unchanged,
     `cargo clippy -p engine-net --all-targets` with zero new warnings, and
     `git diff --stat` confirms no file outside `smirk/engine-net/src/`
     changed.

### 3. Name the handshake stage of server.rs handle_connection as an extracted function

- **Evidence:** `smirk/engine-net/src/server.rs` `handle_connection`
  (L456-627) runs six stages in one body: accept (L469-473), handshake
  (L475-499: read the first frame, require `Ctrl::Hello` with a matching
  version, reply `HelloAck`, or on mismatch send `Ctrl::Reject` and flush it
  with `finish()` + bounded `stopped()` before returning), registration
  (L501-513), the writer task (L515-527), the datagram task (L529-577), and the
  stream reader loop (L579-622) + teardown. The handshake is the only linear,
  parameter-light seam; the three task blocks each capture 4-6 `Arc` handles
  and already read as named stages via their bindings and section comments —
  extracting them is parked (see the plan's Design decisions; this step is the
  entire in-scope server.rs work).
- **Ideal:** `handle_connection` opens with
  `handshake(&mut send, &mut recv, version).await?;` and the Hello/HelloAck/
  Reject dance — including the flush-before-close subtlety — lives in a named,
  documented function a reader finds without scrolling the connection body.
- **Gap:** the handshake's version-mismatch Reject path (with its non-obvious
  "a bare return would drop the connection before the Reject frame reaches the
  wire" constraint) is inline noise ahead of every other stage.
- **Suggestion:** verbatim extraction, byte-identical behavior: move L476-499
  into a free `async fn handshake` placed directly above `handle_connection`;
  the existing inline comment block at L475 becomes the function's doc comment;
  all inner comments move verbatim.
- **Path:**
  1. Baseline: record `cargo nextest run --workspace` count.
  2. In `smirk/engine-net/src/server.rs`, add directly above
     `handle_connection`:
     ```rust
     /// Handshake: the first frame must be `Hello` with a matching version —
     /// reply `HelloAck`, or send `Reject` (flushed before returning, so the
     /// reason reaches the client instead of being discarded by the close)
     /// on a mismatch.
     async fn handshake(
         send: &mut quinn::SendStream,
         recv: &mut quinn::RecvStream,
         version: u8,
     ) -> Result<(), NetError> {
         ...
     }
     ```
     The body is L476-499 moved verbatim: the `read_frame_in(&mut recv).await?`
     bind becomes `read_frame_in(recv).await?` and the two `write_frame(&mut
     send, ...)` calls become `write_frame(send, ...)` (the parameters are
     already `&mut`); `send.finish()` / `send.stopped()` compile unchanged
     through the `&mut` binding; every inner comment (the Reject rationale and
     the finish/stopped explanation) moves with its code. The `// Handshake:
     first frame must be Hello with a matching version.` line is subsumed by
     the doc comment and does not remain at the call site.
  3. In `handle_connection`, replace the moved lines with
     `handshake(&mut send, &mut recv, version).await?;`.
  4. Test (behavioral, existing):
     `version_mismatch_is_rejected_with_a_reason_not_a_silent_close` in
     `smirk/engine-net/tests/handshake.rs` drives the extracted function's both
     arms through a real connection — the Reject must still carry its reason to
     the client (proving the flush survived the extraction) and the server must
     never emit `Connected` for the failed handshake. The in-file unit test
     `stalled_reader_is_kicked_and_backlog_drains` (server.rs:643) performs the
     Hello/HelloAck dance raw and pins the success arm.
  5. Gate: `cargo nextest run --workspace` at the baseline count,
     `cargo clippy -p engine-net --all-targets` with zero new warnings. If the
     borrow of `send`/`recv` fails to compile in this shape (it should not —
     both are locals and the extracted function borrows them only for the
     await), do not restructure `handle_connection`'s stream ownership to force
     it; park the step and report, since any ownership change would exceed the
     verbatim-move contract.
