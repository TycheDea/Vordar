# Code Hygiene Audit (Reworks) — 2026-07-15

Rework-scale companion to `audit-hygiene-2026-07-15.md`: findings that need a design
pass before anyone writes code. Consumed by /plan-rework, which turns one rework into
a plan of fix-sized steps for /implement-finding.

## Ideal end state

Every source file in the two remaining large-module hotspots — engine-net's client
transport and the server's receive system — has one responsibility its name predicts,
with the seams that today live as comment sections or inline blocks promoted to
modules and functions a newcomer finds by name.

## Findings (implementation order)

Cross-type queue (mirrored verbatim from `audit-hygiene-2026-07-15.md`):

> **finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 → finding 8 → finding 9 → finding 10 → finding 11 → finding 12 →
> finding 13 → finding 14 → finding 15 → finding 16 → finding 17 → finding 18 →
> ~~rework 1~~ → ~~rework 2~~.**
>
> Findings 1–5 (comment work) go first: they edit only comments, and doing them before
> the placement findings and reworks means moved code carries clean comments (clean
> once). Finding 2 before finding 10: both edit `bot.rs`; purge the comments before the
> constructor dedup reshapes the file. Findings 3 and 6 before finding 14: they fix
> `gltf_import.rs` comments before the file splits. Finding 4 before rework 2: the one
> `receive.rs` comment fix lands before the file is restructured. Finding 5 before
> finding 11: `class.rs`'s backward-compat comment dies with the re-export it annotates.
> Everything from finding 6 on is ordered by impact; the reworks close because nothing
> depends on them.

### 1. engine-net decomposition: clock filter out of client.rs, impairment unified into impair.rs

- **Evidence:** `smirk/engine-net/src/client.rs` (736 lines) carries three
  responsibilities its name doesn't predict: (1) the clock-sync filter (`ClockSample`,
  `ClockSync`, `on_pong`, `drift_rate`, slew — L221-314, pure and unit-tested), (2) a
  full network conditioner (`Impairment` + WAN profiles, `Jitter`,
  `Pending`/`delay_reorder`, `skewed_micros` — L47-219, testing-only), and (3) the
  actual `NetClient` transport + `client_main` task graph. Meanwhile
  `src/impair.rs` — the file whose *name* is impairment — holds only the lossy UDP
  socket: the machinery is split across two files, and the one named for it holds a
  third of it. Related second-order seam, recorded for the plan but potentially
  descoped: `src/server.rs` (989 lines, ~627 non-test) is cohesive to "the server" but
  bundles connection-cap/IP tracking, token-bucket rate limiting, writer-queue
  backpressure, and the datagram lane all inside `handle_connection`.
- **Ideal:** `clock.rs` owns the sync filter; `impair.rs` owns *all* impairment
  (socket + `Impairment` + `Jitter` + `delay_reorder` + `skewed_micros`); `client.rs`
  is the transport and task graph its name promises. `server.rs`'s
  `handle_connection` reads as named stages even if they stay in-file.
- **Gap:** the crate's two biggest files each hide a testing subsystem inside a
  production one; a reader hunting impairment behavior must know to look in
  `client.rs`.
- **Suggestion:** structure-only decomposition in the style of hygiene reworks 1–3:
  code moves verbatim, re-exports preserve every existing path
  (`engine_net::client::Impairment` etc. keep compiling), unit tests move with their
  code. The clock filter and the conditioner are self-contained (the sweep verified
  no entangled state); the risky part is only the import churn in `client_main`.
  Whether `handle_connection` gets staged in the same rework or parked is the
  planner's call — it is behavior-critical transport code and may deserve its own
  pass.
- **Path:** /plan-rework this finding. Constraints for the plan: (1) every step green
  (`cargo nextest run --workspace` + doc-tests at unchanged counts — the engine-net
  unit tests, impairment/wan_profiles/handshake/flood_control/crowd_snapshot binaries,
  and the server/client e2e suites all exercise this code); (2) moves verbatim,
  byte-identical behavior, re-exports keep external paths working; (3) comments arrive
  already clean (fixes finding 3 runs first); (4) the `Impairment`-construction sites
  in test-support (`bot.rs` presets) must compile unchanged — they are the crate's
  real external consumers.

### 2. receive.rs: promote the five seams inside NetReceiveSystem::run

- **Evidence:** `server/vordar-server/src/net/receive.rs` (699 lines) is dominated by
  one `NetReceiveSystem::run` of ~465 lines (L53-519). The body has five separable
  responsibility seams the length hides: (1) full login handling — rate limit, name
  validation, session takeover, stale-load eviction, `db.login` — L103-197; (2) cast
  dispatch with three near-identical effect arms (Scheduled/Projectile/Leap) —
  L233-338; (3) DB-load completion: redirect vs spawn+Welcome+PrefabTable+WorldClock —
  L355-468; (4) dead-player respawn — L474-495; (5) one-intent-per-tick drain —
  L501-518. The file already demonstrates the target shape: `validate_intent` and
  `queue_move_intents` are extracted free functions.
- **Ideal:** `run` reads as the tick's receive pipeline — a short dispatcher calling
  named functions (`handle_login`, `dispatch_cast`, `complete_db_load`,
  `respawn_dead`, `drain_intents`), each with the borrow scope it actually needs.
- **Gap:** the hottest system on the server is navigable only by scrolling; every
  change to login or casting edits the same 465-line function, and the borrow
  structure that makes the extraction non-trivial is exactly why it needs a design
  pass rather than a blind cut.
- **Suggestion:** extraction-only rework: same file (or a `receive/` family if the
  planner judges the file should split), free functions with explicit parameters, zero
  behavior change. The planner must map which resources/queries each seam actually
  touches (the current body interleaves `NetServerState`, world queries, and the DB
  handle) and choose signatures that don't fight hecs borrows — that mapping is the
  design work.
- **Path:** /plan-rework this finding, after fixes finding 4 has cleaned the file's
  one comment straggler. Constraints: (1) every step green
  (`cargo nextest run --workspace` at unchanged counts — every e2e binary drives this
  system; e2e_security's reject-counter and login tests pin the login seam, e2e_combat
  pins the cast arms, e2e_persistence pins load-completion and respawn); (2) moves
  verbatim where possible, no logic edits; (3) `SystemOrder` anchors and the
  `bench-internals` seam untouched.

## Carried forward from previous report

None — reworks 1–4 of 2026-07-14 all landed (see the fixes file's "Resolved since
last report").

## Resolved since last report

- **Rework 1 (2026-07-14): client net.rs decomposition** — landed; `net/` module
  family verified clean this sweep.
- **Rework 2 (2026-07-14): server net_plugin.rs decomposition** — landed; `net/`
  module family verified clean this sweep (its `receive.rs` residue is rework 2 of
  this report, a seam the original rework consciously deferred by moving the system
  whole).
- **Rework 3 (2026-07-14): renderer lib.rs/mesh.rs decomposition** — landed; the
  remaining long-function seams are fixes finding 17 of this report.
- **Rework 4 (2026-07-14): testing/test-support crate** — landed; the crate's own
  hygiene debts (inherited comments, constructor duplication, util grab-bag) are
  findings 2, 10, and 18 of this report.
