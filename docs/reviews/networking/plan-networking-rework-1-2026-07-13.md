# Plan: Account identity, auth tokens, and combat-state persistence — 2026-07-13

Source: docs/reviews/networking/reworks-networking-2026-07-11.md finding 1.

## Ideal end state

`Login` carries a 32-byte account token that the server verifies against an
`accounts` table before any session action happens — knowing a character name
no longer lets anyone kick its player, take over its session, or learn which
zone it lives in. First login claims a name (trust-on-first-use: zero UX, the
dev-mode answer to "auth deliberately deferred"), and the same table is the
substrate real registration lands on later. Cooldown state is persisted as
per-skill remainders on the character record, so a relog or zone transfer
restores the exact remaining cooldowns instead of the pessimistic
full-cooldown reset from audit finding 8 step 1. Failed logins are
rate-limited per source IP, and `docs/online-play.mmd` + SVG reflect the new
login flow.

## Design decisions

**Trust-on-first-use tokens instead of passwords.** The project decision
(dev runs as a single-player pack, auth deferred, MMO architecture stays)
rules out password UX now. The client mints a random 32-byte token once,
persists it locally (`vordar-credentials.ron`), and presents it in every
`Login`. Server side, the first login for a name stores `sha256(token)` in
`accounts.token_hash` (claiming the name); later logins must match. This is
exactly an API-key model: it closes kick-by-name and session takeover today
with zero interaction, and real registration later is "add a password column
and mint the token server-side" — the `Login` shape, the accounts table, and
every verification seam survive that upgrade unchanged.

**No separate zone-transfer handoff token — the finding's step 5 is
subsumed, deliberately.** The original path called for one-time handoff
tokens on `Redirect`. In this architecture every zone shares one SQLite
credential store through the FIFO `DbWorker`, and after this rework *every*
login — including the relogin a `Redirect` triggers — is token-verified
against it. A replayed `Redirect` grants nothing (the character's `zone`
column is authoritative; logging into the wrong zone just re-redirects), and
a hijacked one fails token verification at the target. A parallel one-time
credential channel would duplicate what account verification already
guarantees — a second path, not a defense. This is recorded as superseding
finding 8's original Path step 5.

**Takeover is gated synchronously against the connected victim's token.**
Token verification lives in the DB worker (the hash is in SQLite), but the
session-takeover kick must not wait for it — and must not fire before it.
Resolution: a `PlayerConn` (and each in-flight `loading` entry) keeps the
token it logged in with; a same-name login is compared against that token
*synchronously* in `NetReceiveSystem`. Mismatch → deny the new connection,
victim untouched, no DB roundtrip. Match → today's save-then-kick takeover,
then the DB login proceeds (the worker re-verifies against the table — the
in-memory token was itself verified, so the two always agree). This
preserves the load-behind-save FIFO ordering the takeover path depends on.

**Cooldowns as `ready_at`, persisted as remainders.** `PlayerConn.last_cast`
(server-time stamp of last cast) becomes `cooldown_ready` (server time the
skill is ready again). That single representation change makes persistence
library-independent: at save time `remaining = ready_at − now` needs no
`ClassLibrary` lookup, at load time `ready_at = spawn_now + remaining` has no
underflow cases, and the pessimistic-seed block at spawn is deleted rather
than worked around. Remainders are frozen while offline (a relog can only
delay a cooldown, never shorten it) — wall-clock semantics are impossible
anyway because both server clocks and the shared world origin reset per
process. Stored as a RON-encoded `HashMap<String, u64>` in a TEXT column
(`ron` is already a workspace dependency; human-readable in a DB the e2e
suite inspects directly), never as a child table — the map is a handful of
entries and rides the existing single-row UPDATE.

**Denials are messages, not kicks.** New `ServerMsg::LoginDenied { reason }`
(`BadCredentials` | `RateLimited`). The server sends it and leaves the
connection open — the CLIENT closes, the same lesson as `Redirect` and the
Phase-6 takeover (a server-side kick races the frame off the wire;
`Connection::close` does not flush pending writes). The honest client needs
this message to stop its reconnect loop from hot-retrying bad credentials. A
lingering denied connection is bounded by the existing per-IP connection cap,
idle timeout, and message token bucket.

**Rate limiting counts failures only, per IP.** Every multi-bot test, the
200-bot soak, and the dev single-player pack log in from 127.0.0.1 — a limit
on successful logins would need config plumbing through every server
constructor just to keep the workspace green. Limiting *failures* (5 per
10 s per IP → all further logins from that IP denied `RateLimited` until the
window drains) closes token brute-force and name-probing with zero impact on
legitimate flows. Account-creation flooding (TOFU makes creation = a
successful login) is consciously deferred until real registration exists.
Requires one small engine-net addition: `NetServer::peer_ip(ConnId)`,
maintained exactly like the existing `rtts` map.

**One protocol bump, once.** `PROTOCOL_VERSION` 8 → 9 in the step that
changes the wire (`Login` token + `LoginDenied`, including the `RateLimited`
reason ahead of its use). engine-net hard-rejects version mismatches at
handshake and client+server ship together, so there is no dual-format
compat code anywhere — old clients are simply refused.

**Schema changes ride the rework-8 migration ladder.** Two appended
`MIGRATIONS` entries: cooldowns column (→ user_version 2), accounts table +
`characters.account_id` + backfill (→ user_version 3). Existing characters
get unclaimed account rows (NULL `token_hash`) claimed on their next login.

**New dependencies:** `sha2` (server: token hashing at rest), `getrandom`
(client: token minting). Both tiny and standard; tests use deterministic
name-derived tokens and need neither.

**Deferred, named:** the client's cosmetic `CastState` action bar does not
learn restored cooldowns on relog (the server rejects early casts exactly as
it does today for any client-side misprediction). Fixing the display means
carrying remainders in `Welcome`; that is cosmetic-only and out of this
rework's scope — a future fix-sized finding.

## Findings (execution order)

### 1. Persist cooldown remainders; replace the pessimistic relog reset

- **Evidence:** `server/vordar-server/src/net_plugin.rs:164` —
  `PlayerConn.last_cast: HashMap<String, u64>` stores the server-time stamp
  of the last accepted cast per skill; the cooldown check at
  `net_plugin.rs:420-425` computes `now - last < def.cooldown_micros`.
  `net_plugin.rs:586-600` pessimistically seeds every ability of the class on
  full cooldown at spawn (audit finding 8 step 1, commit `4a49adb`).
  `server/vordar-server/src/db.rs:69-73` — `CharacterRecord { zone, pos,
  health }` has no cooldown state; `db.rs:30-40` `MIGRATIONS` has one entry
  (user_version 1); `DbHandle::save(name, zone, pos, health)` (`db.rs:177`)
  is called from five sites in net_plugin.rs: disconnect (~line 308),
  takeover (~347), `ZoneTransferSystem` (~872), `AutosaveSystem` (~1101),
  `ShutdownSystem` (~1134). Tests that wait out the pessimistic reset:
  `server/vordar-server/tests/e2e.rs:585` (8.2 s sleep), `tests/zones.rs:213`
  (3.2 s sleep), `client/vordar-client/src/net.rs:1449` (8.3 s wait), and the
  pessimistic-semantics e2e `e2e.rs:798 finding8_relog_does_not_reset_cooldown`.
- **Ideal:** Cooldown state is a `ready_at` map (`cooldown_ready:
  HashMap<String, u64>`, server micros when the skill is next castable),
  persisted at every save as *remainders* (`ready_at − now`, entries > 0
  only) in a new `cooldowns` TEXT column (RON map), and restored at spawn as
  `spawn_now + remaining`. Relog and zone transfer restore the exact
  remaining cooldowns; remainders freeze while offline (relog can only
  delay, never shorten). The pessimistic seeding block is deleted.
- **Gap:** Cooldowns live only in server memory; relog restores an
  approximation (full cooldown on everything) instead of the true state, and
  four test files carry multi-second sleeps that exist only to wait out that
  approximation.
- **Suggestion:** In `net_plugin.rs`: rename `last_cast` → `cooldown_ready`;
  cast gate becomes `now < ready_at`; on accepted cast insert `now +
  def.cooldown_micros` (three insertion sites: Scheduled, Projectile, Leap
  arms). Add pure `fn cooldown_remainders(ready: &HashMap<String, u64>, now:
  u64) -> HashMap<String, u64>`. In `db.rs`: `CharacterRecord` gains
  `cooldowns: HashMap<String, u64>` (remaining micros); append `MIGRATIONS`
  entry `ALTER TABLE characters ADD COLUMN cooldowns TEXT NOT NULL DEFAULT
  '{}';`; change `DbHandle::save` to `save(name: String, record:
  CharacterRecord)` and include the column in the worker's UPDATE and
  SELECT (RON-encode on write; on read, a parse failure logs an error and
  yields an empty map). Add `ron = { workspace = true }` to
  `server/vordar-server/Cargo.toml`. All five save sites build a
  `CharacterRecord` with `cooldown_remainders(&pc.cooldown_ready,
  state.server.now_micros())`; the transfer site keeps its target-zone/
  portal-pos override. Spawn (the `DbLoaded` arm) replaces the pessimistic
  block with `cooldown_ready = record.cooldowns → spawn_now + remaining`;
  the login `defaults` record uses an empty map. Update the
  `bench-internals` `PlayerConn` literal (`net_plugin.rs:1174-1186`) for the
  rename. Delete the three obsolete pessimistic-cooldown sleeps and fix
  their comments (e2e.rs:581-585, zones.rs:210-213, client net.rs:1445-1450).
- **Path:** (1) Fail-first: rewrite
  `finding8_relog_does_not_reset_cooldown` (e2e.rs) as
  `relog_restores_exact_cooldown_remainder`: bot logs in (fresh character —
  castable immediately), casts "onslaught" (8 s cooldown, target own
  position, within its 12-unit range), stays connected 4 s, disconnects,
  sleeps 500 ms (disconnect-save flush), relogs; assert (a) an immediate
  recast is rejected (pump 400 ms, no new mechanic — cooldown not reset to
  zero) and (b) the recast succeeds within 6 s of the re-Welcome (remainder
  ≈ 3–4 s; the pessimistic implementation needs the full 8 s, so this
  assertion fails before the fix — the fail-first proof). (2) Unit tests:
  `cooldown_remainders` drops expired entries and subtracts correctly
  (net_plugin.rs tests); `db.rs` round-trip test — save a record with a
  non-empty cooldowns map, reopen the file, load, assert the map survives;
  fresh-db `user_version` test now expects `MIGRATIONS.len()` (unchanged
  assertion, new length 2); legacy-adoption test still passes (column added
  by migration with default `'{}'`). (3) Implement as in Suggestion.
  (4) Full workspace green: `cargo test --workspace` (the e2e/zones/client
  suites prove no other flow regressed).

### 2. Accounts table, token hashing, and worker-side login verification (no wire change)

- **Evidence:** `server/vordar-server/src/db.rs:30-40` — `MIGRATIONS` ladder
  (rework 8 step 5) with `user_version`-driven `migrate()` at `db.rs:48-66`;
  the module comment (`db.rs:14-16`) already predicts "an accounts table
  later is `ALTER TABLE characters ADD COLUMN account_id`". `DbRequest`
  (`db.rs:75-84`) has `LoadOrCreate`/`Save`; `DbLoaded` (`db.rs:87-91`)
  always carries a record — there is no way for the worker to refuse a
  login. No hashing or account concept exists anywhere.
- **Ideal:** The database owns account identity: an `accounts` table
  (`id, name UNIQUE, token_hash BLOB NULL`), every character linked via
  `account_id`, and a worker-side `Login` request that verifies
  `sha256(token)` against the account (creating and claiming on first use,
  claiming legacy NULL-hash rows, denying mismatches) before loading or
  creating the character. Purely additive this step: the sim still uses the
  old trusting path, behavior is unchanged, the workspace is green.
- **Gap:** No schema, no hash dependency, no verification code, and
  `DbLoaded` cannot express denial.
- **Suggestion:** Add `pub type AccountToken = [u8; 32];` to
  `game/vordar-protocol/src/lib.rs` (type alias only — no message change,
  no version bump). Add `sha2 = "0.10"` to the workspace `[dependencies]`
  table and `sha2 = { workspace = true }` to vordar-server. Append
  `MIGRATIONS` entry (→ user_version 3), one `execute_batch` transaction:
  `CREATE TABLE accounts (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE,
  token_hash BLOB); ALTER TABLE characters ADD COLUMN account_id INTEGER
  REFERENCES accounts(id); INSERT INTO accounts (name) SELECT name FROM
  characters; UPDATE characters SET account_id = (SELECT id FROM accounts
  WHERE accounts.name = characters.name);`. In db.rs: reshape `DbLoaded` to
  `{ conn, name, outcome: DbLoginOutcome }` with `pub enum DbLoginOutcome {
  Granted(CharacterRecord), BadToken }`; add `DbRequest::Login { conn, name,
  token, defaults, reply }` and `DbHandle::login(conn, name, token,
  defaults)` alongside the existing `LoadOrCreate` (which always replies
  `Granted` — it is deleted next step). Worker login flow: SELECT account by
  name → missing: INSERT claimed with `sha256(token)`; `token_hash` NULL
  (legacy backfill): UPDATE to claim; mismatch: reply `BadToken` (no
  character touched). On success, `load_or_create` the character with
  `account_id` set on INSERT, and self-heal `UPDATE characters SET
  account_id = ?1 WHERE name = ?2 AND account_id IS NULL` for characters
  created by the old path. net_plugin's `DbLoaded` consumer matches
  `Granted(record)` (a `BadToken` from the old path is unreachable —
  `log::error!` it).
- **Path:** (1) Fail-first unit tests in db.rs: fresh-name `login` creates a
  claimed account and replies `Granted(defaults)`; a second `login` with the
  same token is `Granted`; with a different token is `BadToken` and the
  character row is unchanged; a legacy file (characters table + rows, no
  accounts) migrated on `spawn` has one unclaimed account per character with
  `account_id` linked and `user_version == 3`, and its first `login` claims
  it; `newer_schema_version_is_refused_not_silently_run` still passes.
  (2) Implement migration + worker verification as in Suggestion. (3) Green:
  `cargo test -p vordar-server` (e2e suites untouched — the sim still calls
  `load_or_create`).

### 3. Protocol v9: token-bearing Login end to end, denial message, takeover gated on token

- **Evidence:** `game/vordar-protocol/src/lib.rs:16` `PROTOCOL_VERSION: u8 =
  8`; `lib.rs:35` `Login { name: String }`; no denial variant in `ServerMsg`
  (`lib.rs:38-91`). Server: `net_plugin.rs:322-371` handles Login — name
  validation, takeover kick by pure name match (`:337-352`), stale-loading
  kick by name (`:357-360`), `db.load_or_create` (`:370`); `PlayerConn`
  (`:140-168`) stores no token; `loading: HashMap<ConnId, String>`
  (`:189-190`). Client: `client/vordar-client/src/net.rs:251` sends
  `Login { name }` on `Connected`; `NetClientState` (`:176-200`) has `user`
  but no token; `handle_disconnected` (`:369-377`) always schedules a
  redial, so a rejected credential would hot-loop; `bin/vordar.rs:34` reads
  `VORDAR_USER`. Test harness: `tests/common/mod.rs:180,269` bots send
  `Login { name }`; client tests build `NetClientState` literals and a raw
  kicker (`client net.rs:1228-1240, 1257-1264`; also the dash test around
  `:1338`). Bot names are unique per test (`common/mod.rs:87`);
  `phase6_login_takeover` (e2e.rs:711) uses two bots with the same name.
- **Ideal:** `PROTOCOL_VERSION = 9`. `Login { name: String, token:
  AccountToken }`; `ServerMsg::LoginDenied { reason: LoginDenyReason }` with
  `enum LoginDenyReason { BadCredentials, RateLimited }` (`RateLimited`
  wired next step — declared now so the wire never bumps twice). Same-name
  takeover and stale-loading eviction happen only when the presented token
  equals the connected/loading session's token; mismatch sends
  `LoginDenied(BadCredentials)` and leaves the connection open (the client
  closes — the Redirect/takeover lesson; no server-side kick that could
  race the frame). The DB worker independently verifies via
  `DbHandle::login`; a `BadToken` outcome also answers `LoginDenied`. The
  real client mints its token once via `getrandom`, persists it in
  `vordar-credentials.ron` (cwd, name → 64-char hex; `VORDAR_CREDENTIALS`
  overrides the path), stops reconnecting after a denial, and all bots use
  deterministic name-derived tokens so same-name tests share credentials
  automatically. `load_or_create` (the trusting path) is deleted.
- **Gap:** The wire carries no token, the server can express no denial,
  takeover still kicks by bare name, and no speaker (client, bots, tests)
  knows about credentials.
- **Suggestion:** Protocol: bump to 9, extend `Login`, add `LoginDenied` +
  reason enum, update `login_roundtrip` and add a `LoginDenied` roundtrip.
  Server (`net_plugin.rs`): `PlayerConn.token: AccountToken`; `loading:
  HashMap<ConnId, (String, AccountToken)>`; Login arm — invalid name now
  answers `LoginDenied(BadCredentials)` instead of a silent `continue`;
  same-name-connected and same-name-loading compare tokens (mismatch →
  denied, match → existing save+kick takeover / stale-forget path);
  enqueue `state.db.login(conn, name, token, defaults)`; `DbLoaded` arm
  matches `Granted(record)` → unchanged routing/spawn/Welcome (token stored
  into the new `PlayerConn`), `BadToken` → `LoginDenied(BadCredentials)`.
  Delete `DbRequest::LoadOrCreate` + `DbHandle::load_or_create`; update the
  db.rs tests that used it to `login`. Update the bench `PlayerConn`
  literal. Client: new `client/vordar-client/src/credentials.rs` —
  `pub fn load_or_mint(path: &Path, name: &str) -> [u8; 32]` (RON map of
  name → hex, created/extended on demand, `getrandom` for fresh tokens,
  tiny local hex helpers, unit tests against a temp path: minted token is
  returned verbatim on the second call; distinct names get distinct
  tokens). `getrandom = "0.3"` in workspace + client Cargo.toml.
  `NetClientPlugin.token: AccountToken` (bin/vordar.rs calls
  `credentials::load_or_mint`), `NetClientState.token`, send it in the
  `Connected` handler, handle `LoginDenied`: `log::error!` + set
  `login_denied` so `handle_disconnected` (and `maybe_reconnect`) stop
  scheduling redials. Tests: `tests/common/mod.rs` — `pub fn
  name_token(name: &str) -> AccountToken` (name bytes zero-padded/truncated
  into 32), `Bot.token` (set from name in every constructor), token included
  at the three Login send sites, `Bot.denied: Option<LoginDenyReason>`
  captured in `pump`; client test literals gain `token` (kicker uses the
  same padded token as its victim, mirroring `name_token`).
- **Path:** (1) Fail-first e2e (`tests/e2e.rs`),
  `wrong_token_cannot_kick_or_impersonate`: bot A logs in as "guarded"
  (claims the name), walks a little; attacker connects and sends
  `Login { "guarded", different token }`; assert the attacker receives
  `LoginDenied(BadCredentials)` and never a Welcome, and that A keeps
  receiving snapshots and never observes `disconnected` — impossible to
  pass before this step (the attacker's login kicks A today, and
  `LoginDenied` doesn't exist). (2) Protocol change + roundtrip tests.
  (3) Server + db.rs changes. (4) Client credentials module (unit tests) +
  wiring. (5) Bot/harness changes. (6) Green: `cargo test --workspace` —
  `phase6_login_takeover` (same name, same derived token → takeover still
  works), zones/loss/soak/shutdown/watchdog suites, and the client
  reconnect/dash tests all pass unmodified in behavior.

### 4. Per-IP failed-login rate limiting

- **Evidence:** `smirk/engine-net/src/server.rs` — the accept loop knows
  `incoming.remote_address().ip()` (`:350`) and tracks per-IP connection
  counts, but `NetServer` exposes no per-connection peer address (public
  surface at `:169-237`: `poll/send/broadcast/disconnect/now_micros/
  rtt_micros/local_addr/metrics`); the `rtts` map (`:128`, insert `:463`,
  remove `:380`) is the exact pattern to mirror. `net_plugin.rs` Login arm
  (after step 3) denies bad credentials but nothing throttles attempts —
  a client can probe names/tokens as fast as the message token bucket
  allows (`MSG_REFILL_PER_SEC = 120.0`, server.rs:212).
- **Ideal:** `NetServer::peer_ip(conn) -> Option<IpAddr>`, and in
  `NetServerState` a failure ledger: 5 failed logins (invalid name, token
  mismatch — sync or from `DbLoginOutcome::BadToken`) within 10 s from one
  IP cause every further Login from that IP to be answered
  `LoginDenied(RateLimited)` until the window drains. Successful logins are
  never throttled (multi-bot tests, the 200-bot soak, and the dev pack all
  share 127.0.0.1 — see Design decisions).
- **Gap:** No peer-address accessor exists in engine-net, and the sim has
  no notion of login-attempt history.
- **Suggestion:** engine-net: add a `peers: Arc<Mutex<HashMap<ConnId,
  IpAddr>>>` alongside `rtts` (insert in `handle_connection` next to the
  rtt insert, remove in the same cleanup that removes the rtt entry) and
  `pub fn peer_ip(&self, conn: ConnId) -> Option<IpAddr>`. net_plugin: a
  small `LoginFailures` struct on `NetServerState` — `HashMap<IpAddr,
  VecDeque<u64>>` of failure stamps with `record(ip, now)` and
  `is_limited(ip, now)` (prune stamps older than
  `LOGIN_FAIL_WINDOW_MICROS = 10_000_000`; limited when
  `len >= MAX_LOGIN_FAILURES = 5`; drop empty entries so the map cannot
  grow unboundedly). In the Login arm: resolve `peer_ip` first; if limited
  → `LoginDenied(RateLimited)`, skip everything else; record a failure at
  each `BadCredentials` denial site, including the async `BadToken` arm
  (where `peer_ip` may already be `None` if the conn dropped — skip
  recording then).
- **Path:** (1) Fail-first engine-net test (server.rs tests module,
  existing connect-and-poll pattern at `:573`): after
  `ServerEvent::Connected(id)`, `peer_ip(id)` is `Some(127.0.0.1)`; after
  the client drops and `Disconnected` fires, it becomes `None`.
  (2) Fail-first unit tests for `LoginFailures` with fabricated timestamps:
  4 failures → not limited; 5 → limited; advancing `now` past the window →
  not limited again and the IP's entry is gone (no 10 s sleeps anywhere).
  (3) Fail-first e2e (`tests/e2e.rs`), `login_failures_are_rate_limited`:
  bot "keeper" claims its name; six raw `engine_net::NetClient`s in
  sequence each send `Login { "keeper", wrong token }` — the first five
  answers are `BadCredentials`, the sixth is `RateLimited`, and "keeper"
  stays connected throughout. (4) Implement engine-net accessor + limiter.
  (5) Green: `cargo test --workspace` (soak and multi-bot suites prove
  successful logins are untouched).

### 5. Update docs/online-play.mmd + SVG for the new login flow

- **Evidence:** `docs/online-play.mmd:48-58` — the `SLOGIN` subgraph reads
  "identify connection by name (session takeover: same name already
  connected → save + disconnect the old conn)" and `SW` spawns from "saved
  position/health"; `docs/online-play.mmd:67` `SPERSIST` lists the save
  contents implicitly. Nothing mentions tokens, denial, rate limiting, or
  cooldown restoration. The reworks queue header
  (`docs/reviews/networking/reworks-networking-2026-07-11.md:27-30`) mandates a
  diagram step whenever a plan changes the online-play flow — this rework
  changes the entire login half. `scripts/render-mmd.sh` renders
  `docs/online-play.mmd` → `docs/online-play.svg` via mermaid-cli.
- **Ideal:** The diagram tells the truth again: login presents name +
  account token; a rate-limit gate precedes verification; verification is
  TOFU (first login claims, mismatch → `LoginDenied`, client closes — no
  server kick); takeover only on token match; spawn restores saved
  position/health *and cooldown remainders*; saves persist cooldowns.
- **Gap:** The rendered flow still shows pre-rework identity-by-name and
  health/position-only persistence.
- **Suggestion:** Rework the `SLOGIN` subgraph: entry node "login: name +
  account token"; decision "IP over failure budget?" → deny (RateLimited);
  node "verify token vs accounts table (first login claims the name)" →
  deny (BadCredentials, client closes) / continue; takeover node condition
  becomes "same name online AND token matches → save + kick old conn";
  `SW` text gains "restore cooldown remainders"; `SPERSIST` text gains
  cooldowns. Add one edge from the deny path back to the client side
  ("LoginDenied → client stops redialing"). Keep node ids stable where the
  text merely changes.
- **Path:** (1) Edit `docs/online-play.mmd` as above (Mermaid flowchart
  syntax — no parentheses/brackets inside node labels except via quoted
  strings, consistent with the existing file). (2) `bash
  scripts/render-mmd.sh docs/online-play.mmd` — the render must exit 0 and
  regenerate `docs/online-play.svg`; commit both. (3) Verification for a
  docs step: the script's clean exit is the parse test; eyeball the SVG for
  the new login nodes.
