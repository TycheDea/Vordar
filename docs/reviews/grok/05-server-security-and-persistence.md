# Expert Review: Server Security, Authority & Persistence
**Reviewer persona:** Principal Multiplayer Security & Backend Engineer  
**Date:** 2026-07-27  
**Scope:** vordar-server, auth, DB, authority, anti-cheat

## Executive summary

Vordar’s server is a serious, MMO-shaped **authoritative** design for a single-process multi-zone pack: clients send intents (never positions), the protocol shape encodes authority rules, QUIC framing/rate limits/connection caps exist, and character persistence is carefully ordered (FIFO DB worker, save-before-redirect, migrations ladder, hashed tokens at rest). For a **dev / single-player pack**, this is above average.

It is **not production-ready multiplayer security**. The binding constraint is the auth model: **trust-on-first-use (TOFU) bearer tokens** with no registration, no password/KDF, no account service, and no session tickets. Combined with **self-signed TLS + client `SkipServerVerification`**, any MITM or network-adjacent attacker on a non-localhost deployment can mint identities, steal sessions, and read/modify traffic. Zone isolation is thread-level only (shared process + shared SQLite), portal redirects are address-only (no transfer tickets), and several DESIGN.md anti-cheat commitments (RTT-variance monitoring, collision-validated movement recomputation beyond dir caps) are incomplete or soft. Security tests cover important login/takeover/rate-limit paths but leave large gaps (MITM/cert, cross-zone race, transfer forgery, health/xp integrity, timing-safe compare).

Treat the current stack as a **local authoritative sim with good bones**. Before any public multiplayer: real accounts + gateway, proper TLS trust, transfer tickets, hardened token compare/hashing, FK/integrity PRAGMAs, and process-level zone isolation (or at least failure-domain isolation of the DB).

---

## Findings

### F1. [SEVERITY: Critical] Transport authentication is effectively off (self-signed + skip-verify)
- **Where:** `smirk/engine-net/src/common.rs` — `server_crypto()` (`rcgen::generate_simple_self_signed`), `client_crypto()` (`dangerous().with_custom_certificate_verifier(SkipServerVerification)`); server bind default `127.0.0.1:5151` in `server/vordar-server/src/main.rs`.
- **What:** Every server process mints a fresh self-signed cert for `"localhost"`. The client accepts **any** server certificate without name, chain, or pinning checks. Encryption (TLS 1.3 / QUIC) is on; **server identity is not**.
- **Why it matters:** On any non-loopback path (LAN party, remote host, VPN, public IP), an on-path attacker can MITM the QUIC connection, present their own cert, observe/forge `Login` tokens and all game traffic, and impersonate either peer. This is the single largest blocker to any “real multiplayer” claim.
- **Recommendation:** Keep skip-verify behind an explicit `dev` feature only. For anything beyond localhost: load PEM cert/key (or ACME), verify hostname + chain on the client, pin or use a private CA for fleet internal links. Refuse to bind non-loopback when skip-verify is compiled in (or hard-warn + require env override).

### F2. [SEVERITY: Critical] TOFU account tokens are bearer credentials, not real auth
- **Where:** `game/vordar-protocol` `AccountToken = [u8; 32]`; client mint `client/vordar-client/src/credentials.rs` (`getrandom` → RON file); server verify `server/vordar-server/src/db.rs` `login()` — first claim stores `sha256(token)`, later logins compare hash; session takeover in `net/receive.rs` `handle_login`.
- **What:** There is no registration, password, email, or account service. Whoever first presents a name **claims** it forever with a 32-byte secret. The secret is a pure bearer token: possession = ownership. Client stores it in plaintext hex RON (`vordar-credentials.ron`). Server stores only SHA-256 (no salt/pepper/KDF). DESIGN.md §8 defers real accounts to a future gateway — code is still TOFU end-to-end.
- **Why it matters:** Name squatting, offline token theft (disk malware, shared machines, backups), and any MITM (F1) permanently owns characters. SHA-256 of a random 32-byte secret is fine against brute force of the hash, but there is no human-factor recovery, no rotation, no revocation list, no multi-device flow.
- **Recommendation:** Introduce a gateway/login service (as DESIGN.md already sketches): password (argon2id) or OAuth → short-lived **session ticket** (HMAC/JWT with server key, audience = zone, exp, character id). Tokens for zone join should be single-use or short TTL. Keep TOFU only as a `dev` profile. Never ship long-lived bearer tokens in a world-readable RON file without OS keychain / DPAPI.

### F3. [SEVERITY: High] Token compare is not constant-time
- **Where:** `server/vordar-server/src/db.rs` — `if claimed != hash`; `net/receive.rs` — `old_token != token` / `stale_token != token` for session takeover gates.
- **What:** Equality uses ordinary slice/`!=` comparison. No `subtle::ConstantTimeEq` (or equivalent) anywhere in server/engine-net.
- **Why it matters:** Classic remote timing side channel on auth material. Practical exploitability over QUIC/WAN is non-trivial but this is table-stakes for auth code review and cheap to fix. Session-path compares the **raw** 32-byte token in cleartext in process memory on every same-name login attempt.
- **Recommendation:** Constant-time compare for `token_hash` and for in-memory session token checks. Prefer comparing only hashes server-side (hash the presented token once, compare to stored hash and to `sha256(session_token)`).

### F4. [SEVERITY: High] Zone portal Redirect has no transfer ticket / proof
- **Where:** `server/vordar-server/src/net/transfer.rs` — save target zone + `Redirect { zone, addr }`; `net/receive.rs` `complete_db_load` also Redirects by DB zone ownership; DESIGN.md §8 describes coordinator `TransferTicket` (not implemented).
- **What:** Handoff is: write character row → tell client an IP:port → client reconnects and logs in again with the **same long-lived account token**. There is no signed, single-use ticket binding (character, source zone, dest zone, expiry, nonce). Directory is a static `base_port + zone_index` map in-process.
- **Why it matters:** Any client that knows a name+token can connect directly to any zone listener and will be redirected or granted based solely on DB state — fine for TOFU, but when real auth arrives, open zone ports + bare Login become a bypass of the gateway. Spoofed/stale Redirect addresses (if ever client-influenced or MITM’d) send players to attacker hosts (amplifies F1). No proof the transfer was authorized by the source zone at time T.
- **Recommendation:** Implement DESIGN’s ticket path: source zone requests ticket from coordinator (or signs with shared key); target validates ticket before Welcome; tickets single-use, short TTL, bound to character id + dest. Eventually close direct zone Login from the public internet (gateway only).

### F5. [SEVERITY: High] Cross-zone duplicate session is not globally enforced
- **Where:** `net/receive.rs` `handle_login` — same-name takeover only within **one** zone’s `NetServerState.conns` / `loading`; multi-zone `main.rs` runs one thread per zone, **shared** `DbWorker` but **no** shared session registry.
- **What:** Two connections can hold the same character name in two different zones concurrently until DB/save races resolve. Takeover token check is per-zone memory only. Login does not “kick global other sessions.”
- **Why it matters:** Duplication / twinning: act in zone A while a stale session remains in zone B; last writer to SQLite wins on fields; economy/combat invariants break once those exist. Also a grief vector if tokens leak.
- **Recommendation:** Session lease table (character_id → conn/zone/epoch) in DB or coordinator; login must CAS-acquire lease; heartbeats; old zone observes revocation and force-saves+kicks. At minimum, on grant, broadcast “evict name X” across zones in-process before Welcome.

### F6. [SEVERITY: High] Shared-process zone “isolation” is not a security boundary
- **Where:** `server/vordar-server/src/main.rs` + `supervisor.rs` — one OS process, N zone threads, shared `DbWorker`, shared `world_origin`; panic supervision restarts a zone in-thread up to `MAX_ZONE_RESTARTS`.
- **What:** A panic in one zone is caught and may restart that zone; other zones keep running. But: same address space (memory corruption / `unsafe` elsewhere in the process would be shared), same SQLite file, same credentials, no cgroup/seccomp, no per-zone privilege drop. After restart budget exhaustion, other zones still Redirect into a **dead listener** (explicitly logged in `join_zone_threads`).
- **Why it matters:** Crash isolation ≠ security isolation. One zone logic bug or content-driven panic loop degrades the fleet unevenly; a future unsafe/soundness hole is process-wide. Stale Redirect after permanent zone death strands/ confuses clients (availability / support incident, mild abuse if combined with social engineering).
- **Recommendation:** Long-term: one zone (or shard) per process as DESIGN topology implies. Short-term: coordinator health so Redirect targets are live; circuit-break directory entries; consider `catch_unwind` is **not** a substitute for process isolation.

### F7. [SEVERITY: Medium] DESIGN.md anti-cheat vs implementation gaps
- **Where:** DESIGN.md §3 vs `server/vordar-server/src/net/receive.rs` `validate_intent` / `queue_move_intents` / `dispatch_cast`; `mechanics.rs` rewind; `lib.rs` comments.
- **What is implemented (good):**
  - Clients cannot send positions — only dirs / cast targets (`vordar-protocol`).
  - Monotonic seq + timestamp; seq=0 rejected; future slack 50 ms; arrival deadline `max(RTT, MAX_REWIND=200ms) + 100ms` margin.
  - Move dir unit-length clamp; non-finite rejected; intent queue cap 16 (flood → latency, not speed).
  - Cast: class ability table, cooldown map, range checks, finite target; mechanics scheduled in **server** time, not client T.
  - Mechanic resolve rewinds through **applied** velocities (incl. leap), capped by `MAX_REWIND_MICROS`; AOI filters telegraph radar.
- **What DESIGN requires but code does not fully deliver:**
  - **RTT-variance monitoring** / lag-switch flagging — **absent** (only smoothed RTT used as deadline floor).
  - “Server recomputes positions from inputs (**max speed, collision validated**)” — speed comes from server `Player` component (good), but there is **no** per-intent collision audit beyond normal sim integration; history rewind uses `step` + `PlayRadius`, not full static collision re-sim of the compensation window.
  - Botting / vision assists correctly called out as residual — no server-side behavioral detection hooks.
- **Why it matters:** The architecture is sound for scheduled-snapshot fairness, but lag-switch and pathological RTT manipulation are only partially mitigated by the 200 ms hard rewind cap. Operators have metrics rejects but no automated abuse signals.
- **Recommendation:** Track RTT EWMA + variance per conn; soft-flag / tighten rewind when variance spikes around mechanic resolve; optional server-side movement sanity (max displacement per window vs speed). Keep botting as a later policy layer.

### F8. [SEVERITY: Medium] Persistence integrity holes (health/xp/cooldowns/FK/sync)
- **Where:** `server/vordar-server/src/db.rs` schema + `save`/`load_or_create`/`complete_db_load`; PRAGMA journal WAL + `synchronous=NORMAL`; no `PRAGMA foreign_keys=ON`.
- **What:**
  - `record.health` from DB is applied with `hp.current = record.health` with **no clamp** to prefab max / non-negative.
  - `xp` loaded/saved as `u32` with no server-side anti-rollback beyond “last save wins.”
  - Cooldown RON blob: parse failure → empty map (fails open to **reset** CDs, not fail closed).
  - `accounts.token_hash` and `characters.account_id` FK exist in DDL but SQLite FK enforcement is off unless pragma enabled — orphans possible via manual DB edit or partial migration bugs.
  - `synchronous=NORMAL` + WAL: good perf; crash can lose the last uncheckpointed frames of writes (accepted durability tradeoff — document for ops).
  - Save is fire-and-forget over mpsc; worker logs errors but sim does not retry or surface to player.
- **Why it matters:** DB file tampering (local cheat on the single-player pack, or compromised host) can grant god health/xp. Fail-open cooldowns are an exploit if an attacker can corrupt the cooldowns column. Ops needs to know durability class of NORMAL.
- **Recommendation:** Clamp health to `[0, max_hp]` on load; validate xp monotonic policies if needed; on cooldown parse error keep last-known or full CD lockout; `PRAGMA foreign_keys=ON`; consider `synchronous=FULL` for production character DB or external Postgres with fsync guarantees; save acks for critical transfers.

### F9. [SEVERITY: Medium] DoS / abuse surface residual
- **Where:** `smirk/engine-net` — `MAX_CONNECTIONS=4096`, `MAX_CONNECTIONS_PER_IP=8`, stream/datagram token buckets (120/s, burst 128), `MAX_FRAME_IN=1024`, writer queue kick at 128, QUIC address validation retry, idle 30s, max 1 bidi / 0 uni streams; `net/login.rs` — 5 failures / 10s / IP; `receive.rs` — pre-login messages dropped until Welcome path.
- **What:** Solid baseline flood control. Residual issues:
  - Failed-login limiter is **per-IP only** and does not slow **successful** logins (intentional for soak tests) — distributed credential stuffing across many IPs is unchecked; successful name-claim spam can still create unbounded `accounts`/`characters` rows (DB growth DoS) until disk fills.
  - No captcha/pow/connection cost for fresh TOFU claims.
  - `CastIntent` skill is an unbounded `String` (frame-capped at 1 KiB) — unknown skills only log+return; still burns decode/lookup.
  - Global accept path work before login (crypto handshake) is mitigated by caps but still CPU-costly under distributed attack.
  - LoginDenied leaves connection open (client must close) — attacker can hold slots until idle timeout (bounded by per-IP conn cap).
- **Recommendation:** Rate-limit **account creations** per IP/day; max accounts table growth alarms; optional proof-of-work on Hello; server-close after LoginDenied; cap skill id length explicitly; consider shrinking idle timeout for unauthenticated conns.

### F10. [SEVERITY: Medium] Secrets hygiene: credentials file & DB layout
- **Where:** `./vordar-credentials.ron` (gitignored; structure `{"player": <hex token>}`); `./vordar.db` gitignored; `.gitignore` entries present; `Cargo.toml` comments document sha2/getrandom intent.
- **What:** Good: credentials and DB are **not** tracked in git; tokens hashed at rest in SQLite; client mints via CSPRNG. Gaps: credentials are **world-readable plaintext** on disk by default (mode depends on OS umask); no encryption-at-rest for DB; no secret server key yet (when tickets arrive, key management is greenfield); `deny.toml` targets **Windows only** (`x86_64-pc-windows-msvc`) — Linux server deploy graph is not cargo-deny’d yet; `unknown-git = "deny"` is good supply-chain posture.
- **Why it matters:** Local malware or multi-user machines steal bearer tokens (F2). Shipping a Linux gameserver without deny coverage misses advisory/license bans on the real host triple.
- **Recommendation:** Document credentials as secret; optional OS secure storage; restrict file ACL on write; add Linux triple to `deny.toml` before hosting; plan KMS/file perms for future signing keys; never log tokens (currently logs names only — keep it that way).

### F11. [SEVERITY: Medium] Zone crash restart can drop in-flight durability / session consistency
- **Where:** `supervisor.rs` `supervise_zone`; `main.rs` rebuild closure uses `handle.fork()`; `NetServer` Drop closes endpoint; DB replies to dead App’s channel are isolated by fork design (good).
- **What:** Panic mid-tick: in-memory entities lost; last durable state is last autosave (~30s stagger) or last disconnect/transfer save. Autosave is best-effort. No crash-consistent snapshot of the zone sim. Restart rebinds same port (good) but characters may roll back. Loading map entries for conns that died with the App are gone; clients see disconnect.
- **Why it matters:** Progress loss and rare duplication if a transfer save was in the channel but panic ordering is misunderstood — current FIFO+fork design is thoughtful, but **not** formally proven under panic at every await point. Ops needs RPO expectations (~30s).
- **Recommendation:** Document RPO/RTO; flush critical saves (transfer, shutdown) with sync ack before Redirect; consider more frequent dirty autosave for players in combat; chaos-test panic during transfer.

### F12. [SEVERITY: Low] Unauthenticated connection window & duplicate Login handling
- **Where:** `receive.rs` — `Connected` waits for Login; duplicate Login on same conn ignored; messages before `PlayerConn` dropped for moves/casts.
- **What:** Generally correct. Duplicate Login silently ignored (no deny). Pre-auth conn still occupies per-IP/global slots. No explicit timeout faster than transport idle for “connected but never logged in.”
- **Recommendation:** Authenticate deadline (e.g. 5–10s) then kick; respond to duplicate Login with a control error for cleaner clients.

### F13. [SEVERITY: Low] Supply chain / unsafe posture
- **Where:** Workspace `deny.toml` (advisories yanked=deny, unknown registry/git deny, license allowlist); `server/vordar-server` and `engine-net` / `vordar-protocol` / `vordar-game` — **no `unsafe`** in these crates (spot check). Quinn/rustls/ring stack is standard. `rusqlite` bundled.
- **What:** Healthy for a game workspace. Residual: deny graph not built for Linux server target (F10); `multiple-versions = "warn"` only; no automated CI mention verified in this pass; rcgen/quinn must stay updated for TLS/QUIC CVEs.
- **Recommendation:** CI job `cargo deny check` on Windows + Linux triples; track quinn/rustls advisories; keep `unsafe` forbidden in server/protocol via crate-level `forbid(unsafe_code)` where feasible.

### F14. [SEVERITY: Low] Ops: shutdown, migrations, observability
- **Where:** `net/shutdown.rs` + `main.rs` ctrlc (second signal force-exit); `db.rs` append-only `MIGRATIONS` + refuse newer `user_version`; broadcast metrics log ~10s.
- **What:** Graceful path saves all connected players then exits App; DB worker drains on Drop — solid. Force-exit on second signal can skip drain (documented tradeoff). Migrations are disciplined (append-only, transactional version bump). No backup/restore tooling; no admin auth endpoint (good — nothing exposed); metrics are logs not a secure authenticated endpoint (fine for now).
- **Recommendation:** Runbook: backup `vordar.db` (+ WAL checkpoint) before upgrades; never force-kill on first Ctrl+C; add migrate dry-run tooling; when admin HTTP appears, authz it separately from game tokens.

### F15. [SEVERITY: Low] Protocol trust boundaries — mostly clean, residual spoofables
- **Where:** `game/vordar-protocol/src/lib.rs`; encode/decode postcard; `PROTOCOL_VERSION = 15` handshake in engine-net.
- **What:** Strong: no client position/state authority; hp on wire is cosmetic; Redirect/LoginDenied intentionally client-closed to avoid races. Residual spoofable **inputs** (by design, must be validated): movement dir, cast skill id, cast target, timestamps, seq, login name/token. Server validates the important ones; skill string and name rules are minimal. Version mismatch gets explicit Reject (good).
- **Recommendation:** Keep authority comments adjacent to message defs; fuzz `decode::<ClientMsg>` ; add property tests that no ServerMsg variant is accepted on the client→server path (type separation already helps).

### F16. [SEVERITY: Info] Security test coverage — good cores, missing fleet-class cases
- **Where:** `tests/e2e_security.rs` (reject metrics, wrong-token cannot kick, login failure rate limit); `db.rs` unit tests (token mismatch leaves character untouched, legacy claim, migrations); `engine-net` flood_control / handshake; persistence/zones/shutdown e2e.
- **What:** Excellent coverage for the TOFU session-takeover footgun and basic flood/reject paths. **Missing:** constant-time/auth timing (hard); cert verification mode tests; cross-zone twin session; transfer without ticket acceptance policy; health clamp on malicious DB row; account-creation flood; unauthenticated connection timeout; panic during portal transfer durability; MITM regression once real TLS lands.
- **Recommendation:** Grow `e2e_security` as a living abuse suite; add a “hostile DB row” test; multi-zone same-name concurrent login test expecting single winner.

### F17. [SEVERITY: Info] Default bind and multi-zone port arithmetic
- **Where:** `main.rs` default `127.0.0.1:5151`, zone i → `base_port + i`.
- **What:** Localhost default is the right dev posture. Port arithmetic is predictable (scannable). When operators pass `0.0.0.0`, all zones become WAN-facing with F1–F4 fully exposed.
- **Recommendation:** README / startup banner: refuse or warn on non-loopback without `VORDAR_I_ACCEPT_INSECURE_DEV_NET=1` until TLS+gateway exist.

### F18. [SEVERITY: Info] Credentials file structure (no values)
- **Where:** path `vordar-credentials.ron` (workspace root; gitignored; size ~77 bytes observed in dev tree).
- **What:** RON map shape: single-level object of **character name → hex-encoded 32-byte token** (64 hex chars). Example structure only: `{ "player": <REDACTED> }`. Type: local plaintext secret store for TOFU bearer tokens. Not a server secret; not an API key. Server never reads this file — only the client does.
- **Why it matters:** Ops/security reviews must know where bearer secrets live without circulating them.
- **Recommendation:** Keep gitignore; treat as password-equivalent; add to secret-scanning allowlist/deny paths in CI.

---

## Strengths worth preserving

1. **Authority-shaped protocol** — clients send intents only; DESIGN.md rules are reflected in types and server validation, not just docs.
2. **Lag-compensation discipline** — 200 ms rewind cap, arrival deadline, monotonic seq/time, applied-velocity history (including leaps) for favor-the-defender snapshot tests.
3. **Transport flood hygiene** — QUIC retry address validation, stream caps, frame size caps, reader token buckets, writer backlog kick, connection and per-IP caps.
4. **Login session-takeover footgun fixed** — same-name kick requires token match; e2e test locks it; mismatched token does not touch victim.
5. **DB architecture** — single worker FIFO ordering across zones, batched transactions, WAL, migration ladder with refuse-newer, tokens hashed at rest, save-before-redirect, shutdown bulk save, handle `fork()` for panic rebuild reply isolation.
6. **AOI-scoped sensitive combat meta** — telegraphs/hits not zone-wide radar.
7. **Supply-chain defaults** — `cargo deny` unknown git/registry denied; no `unsafe` in server/net/protocol/game layers reviewed.
8. **Test culture** — dedicated `e2e_security`, persistence, zones, shutdown, flood tests; unit tests for validate_intent edge cases (seq=0, move redundancy).

---

## Suggested priority order

1. **Before any non-localhost multiplayer:** real TLS trust (fix F1) + kill skip-verify outside dev; warn/refuse insecure bind (F17).
2. **Replace TOFU for networked play:** gateway accounts + short-lived zone tickets (F2, F4); constant-time compares (F3).
3. **Global session uniqueness** across zones (F5) — required before economies or ranked play.
4. **Transfer tickets + live directory** (F4, F6 stale redirect) aligned with DESIGN §8 coordinator.
5. **Persistence clamps + FK + creation rate limits** (F8, F9) — cheap hardening against local tamper and DB growth abuse.
6. **Anti-cheat telemetry** — RTT variance / reject heuristics (F7); expand security e2e (F16).
7. **Ops durability story** — RPO docs, backup/migrate runbooks, Linux `cargo deny`, optional stronger SQLite sync (F10, F11, F14).
8. **Process isolation roadmap** — multi-process zones when scale/security demand it (F6).

---

*Evidence basis: `README.md`, `Cargo.toml`, `deny.toml`, `.claude/DESIGN.md` §3/§8, `vordar-credentials.ron` structure only, `server/vordar-server/**`, `game/vordar-protocol`, `smirk/engine-net` TLS/accept/limits, `client/vordar-client/src/credentials.rs`, server e2e security/persistence/zones/shutdown tests. No secret values reproduced.*
