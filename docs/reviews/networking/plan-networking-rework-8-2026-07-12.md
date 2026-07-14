# Plan: Persistence lifecycle: schema migrations, graceful shutdown, durability classes — 2026-07-12

Source: docs/reviews/networking/reworks-networking-2026-07-11.md finding 8.

## Ideal end state

Stopping the server is a deliberate sequence instead of a kill: a SIGINT/SIGTERM
(or Windows Ctrl+C/console-close) sets one shared flag; every zone thread saves
all of its connected players, exits its tick loop, closes its QUIC endpoint
(releasing the port and telling clients why), and joins its network thread; main
then joins the zone threads and the DbWorker drains every queued save before the
process exits. `NetServer` gains a real close path (today its accept loop runs
forever and its thread is detached — `smirk/engine-net/src/server.rs:132-144,299`),
which is also the primitive reworks finding 10 (zone watchdog restart) is blocked
on. Schema changes become append-a-migration instead of hand-editing databases:
`db.rs` runs a `PRAGMA user_version`-driven migration ladder at open. Durability
classes are explicitly deferred until the first transactional feature exists.

## Design decisions

**Shutdown ownership and ordering.** `main` owns the signal. It shares one
`Arc<AtomicBool>` with a signal handler and inserts it into each zone App as a
`ShutdownFlag` resource. Each zone drains *itself*, in-simulation: a
`ShutdownSystem` observes the flag during a normal tick, enqueues a save for
every connected player (the same `db.save` the disconnect path uses,
`server/vordar-server/src/net_plugin.rs:296-308`), and requests app exit. From
there the teardown is pure existing drop order, no coordinator object and no new
channels: the zone thread returns from `run_headless`, the App drops, and
`NetServerState`'s field order (`server` before `db` before `_db_owner`,
`net_plugin.rs:166-172`) closes the endpoint first (no new events), then releases
the DB handle. `main` joins the zone threads (`join_zone_threads`, already
panic-tolerant) and finally drops the `DbWorker`, whose existing `Drop`
(`db.rs:138-148`) drains all queued saves before joining. The flush step of
"drain-save-flush-exit" is therefore already built — the plan wires the drain
and save steps in front of it. Alternative rejected: a dedicated shutdown
orchestrator that reaches into zone Apps from outside — Apps cannot move across
threads and their internals are resource-owned, so the in-simulation system is
both simpler and the only design that doesn't fight the App model.

**`NetServer` close path is Drop-based.** `bind_with_limits` currently discards
the network thread's `JoinHandle` and `server_main`'s accept loop has no exit
(`server.rs:132-144, 299-364`). The design: keep the `JoinHandle`, pass a
`tokio::sync::oneshot::Receiver<()>` into `server_main`, `tokio::select!` it
against `endpoint.accept()`, and on signal `endpoint.close()` (closes every
connection with a "server shutdown" reason and unblocks accept) then
`endpoint.wait_idle()` under a short timeout so close frames reach the wire.
`impl Drop for NetServer` sends the signal and joins the thread — bounded by the
timeout, so no drop can hang. Drop (rather than an explicit `shutdown()` method)
is chosen because every existing owner (`NetServerState`, tests, benches)
already drops the value at exactly the right time; finding 10's future watchdog
restarts a zone by dropping and rebuilding, which is the same path. No
speculative public API.

**App exit is a resource, set by a system.** `run_headless`'s only exit today is
`max_ticks` (`smirk/engine-app/src/app.rs:255-276`). The engine gains an
`AppExit(bool)` resource, inserted by `App::new` and checked by `run_headless`
after each tick. Crucially the *system* sets it, so save-pass and loop-exit are
causally ordered — a design where `run_headless` polled the shared shutdown
`AtomicBool` directly was rejected because the loop could observe the flag after
`Phase::Input` already ran that tick and exit before the save pass ever
executed. Signature of `run_headless` is unchanged (30 call sites across tests
stay untouched).

**Signal capture via the `ctrlc` crate** (`features = ["termination"]`), a new
small dependency of `vordar-server` only: one `set_handler` call covers SIGINT +
SIGTERM on Unix and Ctrl+C/console-close on Windows. Alternative rejected:
enabling tokio's `signal` feature and running a mini current-thread runtime on a
watcher thread — vordar-server does not even depend on tokio directly today
(`server/vordar-server/Cargo.toml`), SIGTERM would still need unix-cfg'd code,
and the result is ~20 lines doing what `ctrlc` does in 3.

**Migrations: a `user_version` ladder.** `SCHEMA` (`db.rs:24-34`) becomes
`MIGRATIONS: &[&str]`; at open, read `PRAGMA user_version`, apply every
migration above it — each in its own transaction with the version bump committed
atomically alongside its DDL. Migration 1 is the current baseline and *keeps*
`CREATE TABLE IF NOT EXISTS`, so every pre-versioning database in the wild
(`user_version = 0`, table already present) adopts the ladder losslessly; later
migrations use plain DDL. A database whose `user_version` exceeds the ladder is
from a newer build: `DbWorker::spawn` refuses it with an error rather than
running against an unknown schema. Alternative rejected: a `migrations` table
with named entries — `user_version` is a single header read, transactional, and
sufficient for a linear single-file history.

**Durability classes: deferred, deliberately.** The finding's own Path gates
step 5 on "the first transactional feature", and no items/trades exist yet
(pre-content foundation stage). Building a synchronous-confirmed write class now
would be speculative machinery with no caller. The taxonomy for that day, for
the record: class A (fire-and-forget, batched, `synchronous=NORMAL`) — position/
health autosaves, exactly today's behavior; class B (confirmed) — a request
variant carrying a reply sender answered only after `tx.commit()` returns, the
same post-commit-reply pattern `LoadOrCreate` already uses (`db.rs:169-198`),
plus a WAL checkpoint or `synchronous=FULL` for the commit if power-loss
durability (not just process-crash durability) is required. Nothing in this plan
blocks that design. This is an engineering judgment consistent with the
finding, not a product choice needing the user.

## Findings (execution order)

### 1. `NetServer` gains a shutdown path: accept loop exits, endpoint closes, network thread joins on Drop

- **Evidence:** `smirk/engine-net/src/server.rs:132-144` — `bind_with_limits`
  spawns the `engine-net-server` thread and discards its `JoinHandle`; the
  thread runs `rt.block_on(server_main(...))` where the accept loop
  (`server.rs:299` `while let Some(incoming) = endpoint.accept().await`) runs
  forever. Dropping a `NetServer` today closes the event/outgoing channels but
  leaks the thread and never releases the listening UDP socket. `NetServer`'s
  fields are at `server.rs:52-59`; it has no `Drop` impl.
- **Ideal:** Dropping a `NetServer` deterministically stops the accept loop,
  closes the QUIC endpoint (every connected client sees a close with reason
  "server shutdown"), waits briefly for close frames to flush, joins the
  network thread, and releases the socket so the same address can be rebound
  immediately.
- **Gap:** No signal into `server_main`, no `endpoint.close()` call anywhere,
  no retained `JoinHandle`, no `Drop` impl.
- **Suggestion:** In `bind_with_limits`: create a
  `tokio::sync::oneshot::channel::<()>()`, pass the receiver into
  `server_main`, and keep the sender plus the `JoinHandle` as new
  `NetServer` fields (`shutdown_tx: Option<oneshot::Sender<()>>`,
  `thread: Option<std::thread::JoinHandle<()>>`). In `server_main`, replace the
  `while let` accept loop with a `loop { tokio::select! { ... } }` over
  `endpoint.accept()` (break on `None`) and `&mut shutdown` (break on
  completion — a dropped sender also completes it, so a leaked/forgotten
  `NetServer` can never wedge the thread the other way). After the loop:
  `endpoint.close(0u32.into(), b"server shutdown")` then
  `tokio::time::timeout(Duration::from_secs(3), endpoint.wait_idle()).await`.
  `impl Drop for NetServer`: send on `shutdown_tx.take()`, then join
  `thread.take()` — bounded by the wait_idle timeout, so Drop cannot hang.
  Existing behavior for all current callers is unchanged while running; only
  teardown becomes real (tests that previously leaked the thread now join it).
- **Path:** (1) Write the failing test first in `server.rs`'s `mod tests`
  (async, like the existing `stalled_reader_is_kicked_and_backlog_drains`):
  bind a `NetServer` on `127.0.0.1:0`, record `local_addr()`, connect an
  `engine_net::NetClient` (same crate — `client.rs`; its `poll()` yields
  `ClientEvent::Disconnected`), wait for the server to report `Connected`,
  then `drop(server)` and assert (a) the client observes
  `ClientEvent::Disconnected` within a deadline, and (b)
  `NetServer::bind(recorded_addr, 1)` succeeds immediately — the socket was
  released. Before the fix (b) fails: the leaked endpoint still owns the port.
  (2) Implement the select loop + close + Drop as above. (3) Run the full
  engine-net test suite — the existing tests now exercise Drop-join at scope
  exit and must stay green with no hangs; `cargo build` workspace-wide, zero
  new warnings.

### 2. `run_headless` honors an `AppExit` resource so a system can end the server loop

- **Evidence:** `smirk/engine-app/src/app.rs:255-276` — `run_headless(hz,
  max_ticks)`'s only exit is the tick budget; `main.rs` passes `None` and runs
  forever. `App::new` (`app.rs:54-89`) inserts the default resources
  (`SpawnQueue`, `EventBus`, `Time`, …) that systems rely on being present.
- **Ideal:** A system running inside the App can request loop exit; the loop
  breaks after the tick in which the request was made, so everything that
  system did that tick (e.g. enqueue saves) has already happened when the loop
  ends. `max_ticks` behavior is unchanged; no call-site churn.
- **Gap:** No exit mechanism exists other than the tick budget; a shutdown
  system would have nothing to set.
- **Suggestion:** Add `pub struct AppExit(pub bool)` in `engine-app` (in
  `app.rs` next to `App`, exported from the crate root the same way other
  resources are). `App::new` inserts `AppExit(false)` alongside the other
  defaults. In `run_headless`, after `self.tick(delta)` and alongside the
  `max_ticks` check: break when
  `self.resources.get::<AppExit>().is_some_and(|e| e.0)`. Do not touch
  `run()` (windowed) or `run_ticks` (bench seam) — surgical.
- **Path:** (1) Failing test in `app.rs`'s test module (add one if absent): a
  small `System` that increments a shared `Arc<AtomicU64>` tick counter and
  sets `AppExit.0 = true` on the 5th run; `App::new()`, add the system in
  `Phase::Update`, call `run_headless(1000.0, None)` on the test thread — the
  call must *return* (before the fix it never does; use a high hz so the run
  is fast) and the counter must read exactly 5, proving the loop broke after
  (not before, not one tick beyond) the tick that requested exit. (2)
  Implement `AppExit` + the check. (3) Workspace build + full test run green,
  zero new warnings.

### 3. `ShutdownSystem`: on the shared flag, save every connected player and request app exit

- **Evidence:** `server/vordar-server/src/net_plugin.rs:108-134` — `install()`
  registers every zone system; the disconnect path at `net_plugin.rs:296-308`
  shows the exact save a leaving player gets (`state.db.save(name, zone,
  transform.position, health.current)` before despawn). `AutosaveSystem`
  (`net_plugin.rs:1087-1101`) shows how a system iterates `state.conns` and
  reads `Transform`/`Health` per player. There is no code path that saves
  *all* players and stops the loop: killing the process today loses up to a
  full autosave window (~30 s) of every online player's state.
- **Ideal:** When the process-wide shutdown flag flips, each zone — within one
  tick — enqueues a save for every connected player and sets `AppExit`, so
  `run_headless` returns with all final saves already in the DbWorker's queue
  (drained by its existing `Drop`, `db.rs:138-148`).
- **Gap:** No `ShutdownFlag` resource type, no system observing it.
- **Suggestion:** In `net_plugin.rs`: add
  `pub struct ShutdownFlag(pub Arc<AtomicBool>)` and a `ShutdownSystem`
  registered unconditionally by `install()` (`Phase::Input`,
  `SystemOrder::Default`). Each run: if `resources.get::<ShutdownFlag>()` is
  absent (every existing test/bench) or the flag is false, no-op. Otherwise:
  for each `(conn, pc)` in `state.conns`, read `Transform` + `Health` for
  `pc.entity` and `state.db.save(pc.name.clone(), state.zone.name.clone(),
  pos, health)` (identical to the disconnect save); log one info line; set
  `resources.get_mut::<AppExit>().unwrap().0 = true`. Players still in
  `state.loading` have no entity yet — nothing to save. No client
  notification is needed here: `NetServer`'s Drop (finding 1) closes every
  connection with a reason when the App drops moments later.
- **Path:** (1) Failing behavioral test in a new
  `server/vordar-server/tests/shutdown.rs` (uses the existing `mod common`
  `Bot` helpers and `temp_db`, like `e2e.rs`): build
  `vordar_server::build_server_app(addr, &file_db)`, insert
  `ShutdownFlag(flag.clone())`, spawn a thread running
  `app.run_headless(60.0, None)`; `Bot::connect_as(addr, "walker")`, wait for
  Welcome, walk east for ~1 s so the position moves off spawn; set the flag;
  `join()` the server thread with a deadline (before the fix it never
  returns); then open the db file with rusqlite directly (the e2e pattern)
  and assert `walker`'s saved `pos_x` matches the bot's last observed
  position within tolerance — proving the final save landed without any
  disconnect or autosave having fired; also assert the bot observes
  `Disconnected`. (2) Implement `ShutdownFlag` + `ShutdownSystem` and
  register in `install()`. (3) Full workspace tests green (the system no-ops
  everywhere the resource is absent), zero new warnings.

### 4. `main.rs`: signal handler wires the flag; a multi-zone test proves drain-save-flush-exit end to end

- **Evidence:** `server/vordar-server/src/main.rs:29-82` — no signal handling;
  zone threads get `run_headless(TICK_HZ, None)` (line 72) and
  `join_zone_threads(handles)` (line 81) blocks forever; the `DbWorker` (line
  47) would drain on drop at end of `main`, but `main` never ends.
  `server/vordar-server/Cargo.toml` has no signal-handling dependency.
  `tests/zones.rs:49-68` shows the multi-zone test harness shape — and its
  `std::mem::forget(worker)` workaround exists precisely because zone threads
  currently cannot be told to stop.
- **Ideal:** `vordar-server` stops cleanly on SIGINT/SIGTERM (Unix) and
  Ctrl+C/console-close (Windows): signal → every zone saves and exits (finding
  3) → zone threads joined → `DbWorker` dropped and drained → process exit 0.
  A second signal force-exits for the stuck-shutdown case.
- **Gap:** Nothing sets the `ShutdownFlag`; no zone App receives it; `main`
  can only be killed.
- **Suggestion:** Add `ctrlc = { version = "3", features = ["termination"] }`
  to `[workspace.dependencies]` and to `vordar-server`. In `main`: create
  `let shutdown = Arc::new(AtomicBool::new(false));` before spawning zones;
  `ctrlc::set_handler` stores `true` (and on a second invocation —
  handler-local `AtomicBool` — logs and `std::process::exit(1)`). In each
  zone-thread closure (after `build_zone_app`, next to the existing
  `insert_resource` call at `main.rs:70`):
  `app.insert_resource(vordar_server::net_plugin::ShutdownFlag(shutdown.clone()))`.
  Everything after that is existing code: `join_zone_threads` returns once
  every zone exits, and the `DbWorker` drop at end of `main` drains the final
  saves.
- **Path:** (1) Failing test in `tests/shutdown.rs`, mirroring `main`'s
  topology exactly the way `tests/zones.rs::spawn_zone_server` does (two
  zones via `build_zone_app`, one shared `DbWorker`, one shared
  `world_origin`) but keeping the zone `JoinHandle`s and inserting one shared
  `ShutdownFlag` into both apps — the harness this test builds *is* the wiring
  `main` needs, minus the OS signal: connect one bot per zone (the east bot
  logs into east after a portal transfer or is created there via a direct
  save row — simplest: reuse the zones.rs walk-into-portal helper), move
  both, flip the flag once, assert both zone threads join within a deadline,
  then `drop(worker)` (no `mem::forget` — its Drop must return, proving the
  request channel actually closed because every zone's `DbHandle` dropped),
  reopen the db and assert both characters' final positions persisted. (2)
  Wire `main.rs` + Cargo as above — the untestable residue is three lines of
  `ctrlc` glue around the exact mechanism the test exercises. (3) Workspace
  build + tests green, zero new warnings; manual smoke: `cargo run -p
  vordar-server`, Ctrl+C, observe clean exit logs (headless check only).
- **Note:** `tests/zones.rs`'s existing `mem::forget(worker)` stays untouched
  (surgical change; those tests don't need shutdown), but the new test
  documents the non-leaking pattern.

### 5. Migration runner: `PRAGMA user_version` ladder replaces the bare `CREATE TABLE IF NOT EXISTS`

- **Evidence:** `server/vordar-server/src/db.rs:24-34` — `SCHEMA` is a single
  `CREATE TABLE IF NOT EXISTS characters (...)` executed unconditionally at
  `db.rs:95` inside `DbWorker::spawn`. `user_version` is never read or
  written: any future column addition means hand-editing every existing
  database file (dev machines, the shipped `vordar.db` in the repo root).
- **Ideal:** Schema history is an append-only ladder in code: opening a
  database applies exactly the migrations it is missing, atomically, and
  stamps `user_version`; a fresh file, a pre-versioning file (version 0,
  table already present), and an up-to-date file all converge on the same
  schema with data intact; a file from a *newer* build is refused loudly.
- **Gap:** No version read, no ladder, no newer-file guard.
- **Suggestion:** In `db.rs`, replace `SCHEMA` with
  `const MIGRATIONS: &[&str]` whose entry 0 (bringing a db to version 1) is
  the current statement — deliberately keeping `IF NOT EXISTS` so version-0
  files with the table already present adopt cleanly; later entries will use
  plain DDL. Add `fn migrate(db: &mut Connection) -> rusqlite::Result<()>`:
  read `PRAGMA user_version`; if it exceeds `MIGRATIONS.len()`, return an
  error (surface it from `spawn` so startup fails synchronously, matching the
  existing open-failure behavior at `main.rs:47`); otherwise for each pending
  entry open a transaction, `execute_batch` the DDL,
  `pragma_update(None, "user_version", i + 1)` inside the same transaction
  (`user_version` is header state and commits atomically with it), commit.
  Call `migrate` from `spawn` where `execute_batch(SCHEMA)` is today
  (`db.rs:95`, after the PRAGMAs).
- **Path:** (1) Failing tests in `db.rs`'s existing `mod tests` (file-backed
  via the `temp_db` helper, `db.rs:226-230`): (a) *fresh file*: after
  `DbWorker::spawn`, an independent `Connection::open` (the
  `spawn_enables_wal_journal_mode` pattern, `db.rs:330-338`) reads
  `PRAGMA user_version == MIGRATIONS.len() as i64` — fails today (always 0);
  (b) *legacy adoption*: hand-create a db containing the characters table and
  one row (raw rusqlite, no version stamp — exactly what every existing
  database looks like), then `DbWorker::spawn` on it and `load_or_create`
  that name through the worker: the row's data comes back intact and
  `user_version` now reads `MIGRATIONS.len()`; (c) *newer-file guard*:
  hand-create a db with `PRAGMA user_version = 99`; `DbWorker::spawn` must
  return `Err`, not silently run. (2) Implement the ladder + guard. (3) Full
  `vordar-server` test suite green — every existing db test (roundtrip,
  batching, WAL) rides the new open path unchanged; workspace build with zero
  new warnings.
