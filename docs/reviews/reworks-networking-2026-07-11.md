# Networking & Server Reworks — 2026-07-11

Rework-scale companion to `audit-networking-2026-07-11.md`: findings that need a
design pass before implementation. Consumed by /plan-rework, which turns one
rework into a plan of fix-sized steps for /implement-finding. Created
retroactively from the deferred remainders of implemented findings 7 and 8.

## Findings (ranked by impact)

### 1. Account identity, auth tokens, and combat-state persistence (deferred from audit finding 8, Path steps 2–6)

- **Evidence:** `Login` is a bare character name (`vordar-protocol` `ClientMsg::Login { name }`,
  validated only as ≤ 32 printable ASCII in `net_plugin.rs`). Session takeover kicks purely
  by name match — anyone who knows a character name can kick its player and take the
  session. `PlayerConn.last_cast` lives only in server memory; `CharacterRecord` persists
  only zone/pos/health. The pessimistic-cooldowns fix (finding 8 step 1, commit `4a49adb`)
  closed the relog-reset exploit with an approximation, not with real persistence.
- **Ideal:** Account-based identity: an accounts table, a token-bearing `Login` the server
  verifies, zone-transfer handoff tokens so a `Redirect` can't be replayed or hijacked,
  and login rate limiting. Cooldown remainders persisted with the character so a relog
  restores the exact combat state rather than a pessimistic reset.
- **Gap:** Identity is spoofable and kick-by-name is open griefing; combat state survives
  relog only via the pessimistic approximation; nothing rate-limits login attempts.
- **Suggestion:** Design this as one coherent auth + persistence rework: schema (accounts,
  session tokens), protocol (versioned `Login` carrying a token), the transfer handoff
  flow between zones, login rate limiting, and cooldown-remainder columns on
  `CharacterRecord` — the pieces interlock, so ordering and protocol versioning need a
  plan before any code.
- **Path:** From finding 8's original Path: (2) persist cooldown remainders; (3) accounts
  table; (4) token-bearing `Login`; (5) transfer handoff tokens; (6) login rate limiting.
  A design pass must fix the ordering, the schema migration story, and how dev-mode
  (auth deliberately deferred, see project decision) coexists with the real flow.

### 2. QUIC connection migration for seamless network switching (deferred from audit finding 7, Path step 5)

- **Evidence:** The client reconnect state machine (finding 7 steps 1–4, commit `04fc276`)
  treats every connection loss the same way: teardown of the replicated world, backoff
  redial, relogin, full resync. A mere network path change (Wi-Fi → cellular, NAT rebind)
  goes through that whole cycle even though QUIC supports migrating a live connection.
- **Ideal:** quinn's connection migration keeps the session alive across client address
  changes — no relogin, no world teardown, no visible interruption beyond a latency blip.
- **Gap:** Every path change costs a full disconnect/reconnect cycle and its gameplay
  interruption; on mobile-style networks that is frequent, not exceptional.
- **Suggestion:** Design pass on enabling and validating quinn's migration support
  server-side (path validation, anti-amplification interplay with the finding-4 retry
  gate) and on the session-identity implications — this interacts directly with rework 1's
  tokens (a migrated path must not become a session-hijack vector).
- **Path:** (1) design: quinn migration config + security analysis against the finding-4
  flood controls; (2) impairment-layer knob for mid-session address switching (relates to
  audit finding 17); (3) e2e test migrating a session mid-combat with no relogin.
