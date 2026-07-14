# Plan: Zone-thread watchdog recovery (restart or directory pull) — 2026-07-12

Source: docs/reviews/networking/reworks-networking-2026-07-11.md finding 10.

## Ideal end state

A zone whose App panics recovers without operator intervention: the zone
thread catches the panic, lets the unwind release everything the dead App
owned (the `NetServer` Drop landed by rework 8 closes the QUIC endpoint, joins
the network thread, and frees the port — `smirk/engine-net/src/server.rs:240-253`),
rebuilds the App on the same address, and keeps serving. Disconnected players
come back on their own: the client reconnect state machine from audit finding 7
already redials the same address with backoff (`client/vordar-client/src/net.rs:57-109`).
A hot crash loop is bounded: after a small number of consecutive fast failures
the supervisor gives up by re-raising the panic, so `join_zone_threads`'s
existing loud logging (`server/vordar-server/src/lib.rs:87-98`, audit finding
18) fires exactly as today. Clean shutdown via `ShutdownFlag` is never treated
as a failure and never restarted.

## Design decisions

**Restart-on-same-address, not directory pull.** The finding names two
recovery models and suggests restart once `NetServer` can shut down cleanly —
that prerequisite landed (rework 8 step 1, commit `3deb6cb`: oneshot-signaled
select in the accept loop, `endpoint.close()` + bounded `wait_idle`, network
thread joined on Drop). Restart needs no protocol change, no redirect-routing
change, and no conversion of the per-thread-owned immutable
`directory: HashMap<String, SocketAddr>` (`main.rs:42-47`, cloned once per
zone thread) into shared mutable state read at both redirect sites. Directory
pull is strictly more machinery for a strictly worse outcome (the zone stays
down); rejected.

**The supervisor lives inside the zone thread.** Each zone thread wraps its
build-and-run closure in a `std::panic::catch_unwind` loop
(`supervise_zone` in `vordar-server/src/lib.rs`). The panic unwind itself is
the teardown: the App is a local of the closure, so unwinding drops it, and
`NetServerState`'s field order drops the `NetServer` first — its Drop closes
the endpoint and joins the network thread synchronously (bounded by the 3 s
`wait_idle` timeout), so by the time `catch_unwind` returns the port is free
and rebinding the same address succeeds. The workspace uses the default
`panic = "unwind"` (no profile overrides in the root `Cargo.toml`).
Alternative rejected: a main-thread supervisor that detects which zone handle
finished and respawns the thread — it needs completion-detection machinery
across `JoinHandle`s (polling or a channel) and moves zone identity out of the
thread that owns it, all to do what a loop inside the thread does in ten
lines. `AssertUnwindSafe` on the closure is justified: every iteration builds
a fresh App from scratch; the only state crossing the boundary is designed
for cross-thread sharing already (the DbWorker's mpsc sender, the shutdown
`AtomicBool`, an immutable directory clone, `Instant`, `ZoneDef` — re-cloned
per build).

**Give-up re-raises the panic.** When the restart budget is exhausted (or the
shutdown flag is set), the supervisor calls
`std::panic::resume_unwind(payload)` with the last caught payload. The zone
thread then terminates panicked, and `join_zone_threads` logs it loudly with
`panic_message` — audit finding 18's visibility contract is preserved
unchanged, with zero new logging paths at the give-up site.

**Bounded restarts = bounded *consecutive fast* failures.** The budget exists
to stop hot crash loops (bad content, corrupt state that panics on the first
tick), not to cap how many times a long-lived server may ever recover. So:
`MAX_ZONE_RESTARTS = 3` consecutive strikes, where a run that survived
`HEALTHY_UPTIME` (60 s) resets the strike count to zero before the new panic
is counted. The reset decision is a pure function (`next_strikes(prev,
ran_for)`) so it is unit-testable with any `Duration` — no time injection, no
60 s test. Alternative rejected: a fixed per-process lifetime budget — after
three unrelated panics days apart, the fourth would kill recovery forever,
which inverts the feature's purpose. No inter-restart delay is needed:
teardown is synchronous (see above), and a panic-on-first-tick loop is exactly
what the strike budget bounds.

**Shutdown always wins.** If the process-wide shutdown flag
(`ShutdownFlag`'s `Arc<AtomicBool>`, wired by rework 8 step 4) is set when a
panic is caught, the supervisor does not restart — it re-raises immediately.
A clean return from the run closure (ShutdownSystem set `AppExit`) ends
supervision with no restart. A restart that races the signal is harmless: the
fresh App sees the flag on its first tick and drains normally.

**Fresh `DbHandle` per rebuild via an explicit `fork()`.** `build_zone_app`
consumes a `DbHandle` (`lib.rs:61-76`) and `DbHandle` is deliberately not
`Clone` — each handle owns a private reply channel so loads return only to
the zone that asked (`db.rs:101-109`). The rebuilt App must not inherit the
dead App's reply channel (in-flight load replies addressed to the panicked
App must die with it), so the supervisor's build closure mints a sibling with
a new method `DbHandle::fork()`: same cloned request sender, fresh private
reply pair. An explicit named method rather than `impl Clone` because the
copy is intentionally not identical — a `Clone` that silently swaps the reply
channel would be a semantic trap.

**No `docs/online-play.mmd` change.** The queue note requires a diagram step
when a plan changes the online-play flow. This rework changes none of it: no
protocol message, no redirect route, no login step. The diagram
(`docs/online-play.mmd`) has no zone-lifecycle or reconnect lane to update —
the client-visible experience of a zone panic (disconnect → client redials →
Welcome) is composed entirely of flows that already exist. The one document
that does go stale is `join_zone_threads`'s doc comment in
`server/vordar-server/src/lib.rs:78-86` ("its listener is now dead — other
zones will keep redirecting players into a stale address until the process is
restarted"), which step 3 updates.

## Findings (execution order)

### 1. `DbHandle::fork()` — mint a sibling handle with a fresh private reply channel

- **Evidence:** `server/vordar-server/src/db.rs:103-109` — `DbHandle` is
  `{ tx: mpsc::Sender<DbRequest>, reply_tx, reply_rx: Mutex<Receiver<..>> }`
  and is not `Clone`; the only way to mint one is `DbWorker::handle(&self)`
  (`db.rs:136-143`), but the `DbWorker` lives on the main thread while zone
  rebuilds happen on zone threads. `build_zone_app`
  (`server/vordar-server/src/lib.rs:61-76`) consumes a `DbHandle` by value,
  so a supervisor that rebuilds a zone's App needs a new handle per build
  without reaching back to main.
- **Ideal:** `handle.fork()` yields an independent sibling: it shares the
  worker's request channel (saves and loads flow to the same `DbWorker`,
  keeping the channel alive until every fork drops), but owns a fresh private
  reply channel, so a load issued through the fork replies only to the fork —
  and replies addressed to a dropped original can never leak into a fork.
- **Gap:** No such method; nothing can create a `DbHandle` away from the
  `DbWorker`.
- **Suggestion:** In `db.rs`, add to `impl DbHandle`:
  `pub fn fork(&self) -> DbHandle` — clone `self.tx`, create a fresh
  `mpsc::channel()` for `(reply_tx, reply_rx)`, wrap the receiver in a
  `Mutex`, exactly mirroring `DbWorker::handle`'s body. Doc comment must say
  why it is not `Clone`: the reply channel is deliberately private per
  handle. No other code changes.
- **Path:** (1) Failing test first in `db.rs`'s existing `mod tests` (uses
  the `temp_db` helper there): spawn a `DbWorker` on a temp file, take
  `h1 = worker.handle()`, `h2 = h1.fork()`; issue
  `h2.load_or_create(1, "forked".into(), defaults)` and poll until the reply
  arrives — assert it arrives on `h2.poll()` and that `h1.poll()` stays empty
  throughout; then `h2.save("forked", ...)` a moved position, drop both
  handles and the worker (its Drop drains), reopen the file with rusqlite and
  assert the saved row — proving the fork reaches the same worker end to end.
  (Fails first trivially: `fork` does not exist; the behavioral assertions
  then pin its semantics.) (2) Implement `fork()`. (3) Full `vordar-server`
  test suite green, workspace build with zero new warnings.

### 2. `supervise_zone`: catch_unwind restart loop with a consecutive-fast-failure budget

- **Evidence:** `server/vordar-server/src/lib.rs:87-98` — `join_zone_threads`
  logs a panicked zone loudly but nothing recovers it; `main.rs:79-95` runs
  each zone as a bare thread closure with no supervision. The teardown
  primitive exists: a panic unwinding out of `run_headless` drops the App,
  whose `NetServerState` drops the `NetServer` first, and `NetServer`'s Drop
  (`smirk/engine-net/src/server.rs:240-253`) closes the endpoint, joins the
  network thread (bounded by the 3 s `wait_idle` timeout in `server_main`),
  and frees the port — so a same-address rebind immediately after
  `catch_unwind` returns is safe. The workspace uses default
  `panic = "unwind"` (no profile overrides in the root `Cargo.toml`).
  `panic_message` (`lib.rs:103-109`) already extracts readable text from a
  caught payload.
- **Ideal:** A public `supervise_zone` in `vordar-server`'s `lib.rs` that any
  zone thread runs: call the build-and-run closure; on clean return, return
  (shutdown drain finished); on panic, log the payload and rerun — unless the
  shutdown flag is set or the consecutive-fast-failure budget is spent, in
  which case re-raise the payload so `join_zone_threads` reports it exactly
  as today.
- **Gap:** No supervisor function exists anywhere; a zone panic today
  terminates its thread permanently.
- **Suggestion:** In `server/vordar-server/src/lib.rs` add:
  - `pub const MAX_ZONE_RESTARTS: u32 = 3;` — max consecutive fast failures.
  - `const HEALTHY_UPTIME: Duration = Duration::from_secs(60);`
  - `fn next_strikes(prev: u32, ran_for: Duration) -> u32` — pure:
    `if ran_for >= HEALTHY_UPTIME { 1 } else { prev + 1 }` (a long healthy
    run forgives earlier strikes; the new panic itself always counts as one).
  - `pub fn supervise_zone(name: &str, shutdown: &std::sync::atomic::AtomicBool, mut run_zone: impl FnMut())`:
    loop `{ let started = Instant::now(); match catch_unwind(AssertUnwindSafe(&mut run_zone)) { Ok(()) => return, Err(payload) => { strikes = next_strikes(strikes, started.elapsed()); if shutdown.load(Relaxed) || strikes > MAX_ZONE_RESTARTS { std::panic::resume_unwind(payload); } log::error!("zone '{name}' panicked ({}); restarting (strike {strikes}/{MAX_ZONE_RESTARTS})", panic_message(&payload)); } } }`.
    `AssertUnwindSafe` is sound here: each iteration builds a fresh App; the
    closure's captures are all rebuild-from state (documented at the call).
  - No caller changes yet — `main.rs` is untouched in this step, so the
    workspace stays green with the function only exercised by its tests.
- **Path:** (1) Failing tests first in `lib.rs`'s existing `mod tests`
  (they fail to compile until the API exists; the assertions then pin
  behavior): (a) *restart*: a closure over an `Arc<AtomicUsize>` that panics
  on the first call and returns on the second; `supervise_zone("t",
  &AtomicBool::new(false), closure)` must return normally with the counter at
  exactly 2. (b) *budget*: a closure that always panics immediately, run via
  `std::thread::spawn`; the thread's `join()` must return `Err` whose
  `panic_message` is the original panic text (payload preserved through
  `resume_unwind`), and the attempt counter must read exactly
  `MAX_ZONE_RESTARTS + 1`. (c) *shutdown wins*: flag already true, closure
  panics; `join()` is `Err` and the counter reads exactly 1 — no restart.
  (d) *forgiveness*: `next_strikes(3, Duration::from_secs(61)) == 1` and
  `next_strikes(1, Duration::from_millis(100)) == 2`. (2) Implement.
  (3) Full workspace build + tests green, zero new warnings.

### 3. Wire `main.rs` through the supervisor; e2e test: a panicked zone restarts on the same address and a fresh connection succeeds

- **Evidence:** `server/vordar-server/src/main.rs:70-99` — each zone thread
  closure builds the App once (`build_zone_app` at line 85, chapter install,
  `events.ron` + `ShutdownFlag` resources) and runs
  `app.run_headless(TICK_HZ, None)` once; a panic anywhere terminates the
  zone forever. `join_zone_threads`'s doc comment
  (`server/vordar-server/src/lib.rs:78-86`) still documents the dead-listener
  behavior and defers to this finding. `ZoneDef` is `Clone`
  (`game/vordar-game/src/world/zones.rs:15-16`).
  `server/vordar-server/tests/shutdown.rs:98-202` shows the exact
  main-mirroring multi-zone harness shape (shared `DbWorker`, shared
  `world_origin`, `test_zones()` from `tests/common/mod.rs:35-62`, per-zone
  `ShutdownFlag`, deadline joins, `drop(worker)` at the end);
  `tests/common/mod.rs` has the `Bot` helpers (`connect_as`, `wait_for`,
  `disconnected` flag) and shows a custom `System` registered on a server App
  (`PopulateSystem`, `mod.rs:320-337`).
- **Ideal:** `vordar-server`'s zone threads run under `supervise_zone`: the
  build-and-run closure (App build, chapter install, resources,
  `run_headless`) is re-runnable, minting a fresh `DbHandle` per build via
  `fork()` (step 1) and re-cloning `ZoneDef`/directory. An e2e test proves
  the whole loop: a zone panics mid-session, its player is disconnected, the
  watchdog rebuilds the zone on the same address, a fresh connection to that
  address is Welcomed, a second zone is completely unaffected, and the shared
  shutdown flag still drains everything cleanly afterward.
- **Gap:** `main.rs` never calls `supervise_zone`; no test anywhere kills and
  revives a live zone.
- **Suggestion:** In `main.rs`, restructure only the zone-thread closure
  (lines 79-95): the spawned thread becomes
  `supervise_zone(&name, &zone_shutdown, move || { ... })` where the inner
  `FnMut` closure does exactly what the thread body does today, per call:
  clone `zone` (moved into the outer closure) for `build_zone_app(addr,
  handle.fork(), zone.clone(), directory.clone(), world_origin)`, install the
  chapter (`chapters().install(...)` — the registry is rebuilt per call, as
  `chapters()` is already a constructor fn), insert the world-events resource
  and `ShutdownFlag(zone_shutdown.clone())`, log "zone listening on {addr}",
  `app.run_headless(TICK_HZ, None)`. `handle` is the `DbHandle` captured at
  spawn; `fork()` per build keeps the dead App's reply channel from leaking
  into the rebuilt one. Update `join_zone_threads`'s doc comment
  (`lib.rs:78-86`): a panicked zone now only reaches that log after the
  supervisor's restart budget is spent. In `tests/common/mod.rs`, add
  `Bot::try_connect_as(addr, name) -> Option<Bot>` (same body as
  `connect_as` but propagating `NetClient::connect_impaired`'s `Err` as
  `None`) so the test can poll for the rebound endpoint instead of racing
  the ≤3 s teardown window.
- **Path:** (1) Failing e2e test first, new file
  `server/vordar-server/tests/watchdog.rs` (uses `mod common`; fresh ports,
  e.g. 25301/25302, distinct from `shutdown.rs`'s 2520x and `zones.rs`'s):
  mirror `main`'s topology like `shutdown.rs::shared_flag_drains_both_zones...`
  does — `test_zones()`, one shared `DbWorker`, shared `world_origin`, one
  shared shutdown `Arc<AtomicBool>` — but run each zone thread as
  `supervise_zone(&zone.name, &flag_clone, build_closure)` where the build
  closure is `build_zone_app(addr, handle.fork(), zone.clone(),
  directory.clone(), world_origin)` plus
  `app.insert_resource(ShutdownFlag(...))` plus, for the *start* zone only, a
  test-local `PanicOnce(Arc<AtomicBool>)` system (`engine_app::scheduler::
  {Phase, System, SystemOrder}`; `run` body:
  `if self.0.swap(false, SeqCst) { panic!("test-induced zone panic") }` —
  the swap guarantees exactly one panic across rebuilds) registered via
  `app.add_system(.., Phase::Update, SystemOrder::Default)`. Scenario:
  connect `Bot::connect_as(start_addr, "victim")` and one bot to east (via
  the portal like shutdown.rs's rover, or `Bot::connect_as(east_addr, ..)`
  directly — direct is simpler and sufficient); both reach Welcome +
  first snapshot. Set the panic trigger; `victim.wait_for("disconnected",
  ..., |b| b.disconnected)` — the unwinding App's `NetServer` Drop closes the
  wire. Then poll `Bot::try_connect_as(start_addr, "victim")` every 100 ms
  under a 10 s deadline until `Some`, and drive that new bot to Welcome +
  first snapshot — **this is the assertion the finding names: a fresh
  connection to the same address succeeds after the watchdog restart**
  (before the fix the supervisor doesn't exist, the zone thread is dead, and
  every retry times out). Pump the east bot throughout and assert its
  `disconnected` stays false — the panic and restart were isolated to one
  zone. Finally flip the shared shutdown flag: both supervisor threads must
  join within a deadline (clean return, no restart — proving supervision and
  drain-shutdown compose), `drop(worker)` must return, and the db file must
  contain both characters. (2) Wire `main.rs` + the doc comment +
  `try_connect_as` as above — the test's per-zone supervisor call IS the
  wiring `main` needs. (3) Full workspace build + test run green
  (`shutdown.rs`, `zones.rs`, `e2e.rs` unchanged and passing), zero new
  warnings.
