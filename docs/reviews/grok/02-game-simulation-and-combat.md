# Expert Review: Game Simulation & Combat Systems
**Reviewer persona:** Principal Gameplay Systems Architect
**Date:** 2026-07-27
**Scope:** vordar-game, chapters, content, shared sim

## Executive summary

Vordar’s shared simulation is unusually disciplined for an early MMO-shaped action RPG. Ownership is clean (`player/`, `enemies/`, `combat/`, `motion/`, `world/`), intent events are the only input seam, and several hard problems that usually rot into rollback chaos are already solved with pure functions: movement velocity, play-radius clamp, leap integration, damage formula, camp slot scatter, world-event windows, and telegraph fill. The scheduled-snapshot combat model from DESIGN.md is real on the server (absolute `resolve_at_micros`, stamp-based player rewind capped at 200 ms, applied-velocity history that survives dashes), and client prediction deliberately runs only the player movement/leap slice of the sim rather than a full dual-world.

The critical product gap is not architecture competence — it is **content/runtime wiring and combat completeness relative to the design thesis**. Shipped zone topology currently sets both `start` and `east` to `chapter: None`, so the carefully authored chapter-01 camps, chapter-02 town, and blood-moon wave pressure do not populate the live multi-zone server path that clients actually join. Scheduled mechanics exist only as horizontal circles; FF14-style fight data (`MechanicDef` timelines, cones/rects, target rules) is not yet a content schema. Contact damage has no faction gate (unlike projectiles), sandbox offline casting cannot resolve Scheduled/Leap damage, and a handful of class-specific combat rules (Ravager rage/finisher) are hardcoded rather than data-driven. Density is still RO-field scale (~21 camp slots + event waves), not raid or city scale — which is appropriate for now, but the empty-zone default means the “living world” loop is mostly latent.

Net: foundation quality is high and largely DESIGN.md-faithful; the next work should reconnect authored content to live zones, close combat fairness/side-rule holes, and grow the mechanic schema before inventing new systems.

## Findings

### F1. [SEVERITY: Critical] Live zones ship with `chapter: None` — camps and town content never install on the multi-zone server

- **Where:** `content/zones/zones.ron` (`start` and `east` both `chapter: None`); `server/vordar-server/src/main.rs` only calls `ChapterRegistry::install` when `zone.chapter` is `Some`; chapter content lives in `content/chapters/chapter01|02/` and `game/chapter-0{1,2}/`.
- **What:** Chapter-01 camps (grunts, mossbacks, imps, sentinels) and chapter-02 Emberwood Rest (buildings, NPCs, outer camps) are fully authored and tested, but the production zone table deliberately disables chapter install. World events still load (`events.ron` → `WorldEventSystem`), and those waves reference `"grunt"`, which only exists if a chapter prefab dir was installed — so blood-moon pressure also fails unless something else adds those prefabs (e2e tests do this manually via `Chapter01ContentPlugin`).
- **Why it matters:** The shared sim’s population systems (`CampSystem`, `ChapterSetupSystem`, chapter prefabs) are dark on the path players run. Design promise of a populated RO-style field and a first town is not delivered by the default binary configuration. This is the single largest sim/content divergence.
- **Recommendation:** Point `start` → `chapter01` and `east` → `chapter02` (or an equivalent content pack id), keep `requires` for prefab inheritance, and add a boot assertion that every zone with world-event prefab refs can resolve those prefab names. Treat “empty zone” as an explicit test fixture, not the default.

### F2. [SEVERITY: High] Scheduled-snapshot combat is implemented, but only as circle AoE — DESIGN.md fight-data model is still aspirational

- **Where:** `game/vordar-game/src/combat/mechanic.rs` (`Mechanic { radius, ... }` only); `player/skills.rs` `AbilityEffect::Scheduled` / `Leap`; server `net/mechanics.rs` + `net/receive.rs` cast arm; DESIGN.md §3–§4 (`MechanicDef` with telegraph prefab, target rule, snapshot delay, shape variety).
- **What:** Resolve is correct in spirit: absolute server time, T = telegraph completion, player rewind via applied-intent history, NPC position at resolve tick, `HitResult` broadcast, despawn via queue. But the hit test is exclusively `distance_squared <= radius²` on the XZ plane. There is no cone/rect/donut, no boss timeline resource, no multi-step choreography beyond “cast one ability.”
- **Why it matters:** The core innovation is “telegraphed ground mechanics resolve at a clock-synced T.” Without richer shapes and data-driven encounter timelines, the model only covers player skill blasts — not FF14-style boss fights that justify the architecture investment.
- **Recommendation:** Promote `Mechanic` into a small shape enum (circle/cone/rect) with shared pure `contains(pos) -> bool`, keep resolve/rewind identical, and introduce a RON `EncounterDef` / `MechanicDef` timeline consumed by a server-only director system. Do not invent rollback to get more action feel.

### F3. [SEVERITY: High] Networked client does not run CoreGamePlugin — correct authority split, but offline sandbox and online combat diverge hard

- **Where:** `client/vordar-client/src/bin/vordar.rs` uses `GameComponentsPlugin` + `install_all_content` only; `client/.../net/mod.rs` optionally registers `PlayerMovementSystem` + `LeapSystem` + `MovementSystem` for prediction; `client/.../sandbox.rs` runs full `CoreGamePlugin` but Scheduled/Leap damage is explicitly a no-op offline; server `build_zone_app` runs full `CoreGamePlugin` + net plugins.
- **What:** Online client is display + prediction slice: no enemy AI, camps, contact damage, projectile TTL/hits, death, or mechanic resolve. Sandbox can fire projectiles locally but cannot exercise the scheduled-snapshot path that is the game’s combat identity. Design comments acknowledge this.
- **Why it matters:** Iteration on the primary combat fantasy (telegraphs, resolve fairness, leap arrival) requires a live server. Sandbox is a movement/VFX toy for the default class kit, not a combat lab. That slows content authoring and raises the cost of every combat regression.
- **Recommendation:** Add a “local authority” sandbox mode that runs `CoreGamePlugin` *and* a minimal in-process `MechanicResolveSystem` (no net), or a single-process listen-server tool. Keep the thin networked client as-is for MMO correctness.

### F4. [SEVERITY: High] Contact damage has no side/faction filter — unlike projectiles

- **Where:** `combat/projectile.rs` enforces `hits_players` / Player-vs-Enemy; `combat/contact_damage.rs` damages any `Health` on `CollisionStarted`, both directions if both bear `ContactDamage`.
- **What:** Grunts with `ContactDamage` will correctly hurt players, but two Solid+ContactDamage entities touching each other (enemy packs packing in, future pet/summon, player-with-thorns, etc.) will mutual-damage without faction checks. Players currently lack ContactDamage, so today’s PvE is mostly safe — the hole is structural.
- **Why it matters:** As density rises and new archetypes appear, silent friendly fire / enemy self-damage will corrupt AI packs and any future collision-based skills. Projectiles already solved this; contact did not.
- **Recommendation:** Mirror projectile side rules (or a small `Team`/`Faction` component) inside `ContactDamageSystem`. Unit-test enemy-enemy and player-player pairs as pass-through.

### F5. [SEVERITY: Medium] Mechanic resolve rewinds players but not NPCs — fairness is asymmetric under latency

- **Where:** `server/vordar-server/src/net/mechanics.rs` (`rewound_position` only when entity maps to a `PlayerConn`; else current `Transform`).
- **What:** DESIGN.md prioritizes favor-the-defender for players (inputs stamped ≤ T). Enemies and other NPCs are tested at “now” on the first 10 Hz resolve tick past T, which can be up to ~100 ms after T plus sim phase skew.
- **Why it matters:** For player-cast AoE into kiting packs this is usually attacker-favorable (enemies keep walking into the circle). For enemy-cast telegraphs later, it becomes defender-hostile for NPCs and inconsistent with the “position at T” slogan. Acceptable as an intentional simplification only if documented and capped.
- **Recommendation:** Document the NPC rule as intentional v1. When enemy telegraphs land, either (a) freeze/slow casters during cast, or (b) store short NPC position history from the same `step` integration path. Prefer (a) for cost.

### F6. [SEVERITY: Medium] Resolve cadence is 10 Hz self-gated on PostUpdate — snapshot T can lag the design instant

- **Where:** `server/.../net/mod.rs` `STAGGER = POST_HZ / SNAPSHOT_HZ`; `MechanicResolveSystem` returns early unless `ticks % STAGGER == 0`; resolve still uses `now >= resolve_at_micros` then rewinds players to `t_eff`.
- **What:** Damage decision runs on the first staggered tick after T, not necessarily at T. Player rewind closes the fairness gap for movers; telegraph visuals complete at T on clients via synced clock (`TelegraphFillSystem`). Hit application and `HitResult` can arrive a slice later.
- **Why it matters:** Within DESIGN.md this is mostly fine (cast times 0.3–2.0 s dwarf the slice). It does mean death/XP/rage from mechanics are phase-coupled to PostUpdate, and very short cast times (Rend at 300 ms) have a larger relative quantization error.
- **Recommendation:** Keep 10 Hz if CPU-bound, but assert `cast_micros >= 2 * STAGGER period` in content lint for Scheduled/Leap abilities. Consider resolving due mechanics every PostUpdate while keeping snapshot stagger separate.

### F7. [SEVERITY: Medium] Determinism is strong on pure paths, weaker where HashSet order meets multi-contact physics

- **Where:** Strengths: `compute_damage` seeded crits, `movement_velocity`, `motion::step`, `leap_velocity`, `camp_slot_pos`, `active_event`, `day_night_light`, intent “one per tick” drain. Weakness: `SeparationSystem` accumulates MTV from `ActivePairs` (`HashSet`) iteration; `motion/mod.rs` tests explicitly allow `1e-5` drift with two overlapping anchored walls vs `predict_step`’s fixed slice order.
- **What:** Single-static prediction matches live bit-for-bit; multi-static contact is tolerance-matched. Projectile multi-hit uses `HashSet<Entity>` for spent bolts (first valid contact depends on event order). DESIGN.md §6 bans wall clocks and local RNG in gameplay — respected inside `vordar-game` systems.
- **Why it matters:** Client reconciliation trusts `predict_step`; residual multi-wall error falls into smooth/snap bands (`TRUST_DISTANCE` 0.3, `SNAP` 1.0). At town density (chapter-02 cottages) this is the main prediction footgun. Not a desync of authority (server wins), but a feel/bandwidth issue if snaps become common.
- **Recommendation:** Sort `ActivePairs` (or correction keys) by entity id before MTV accumulation so live separation matches `predict_step` order. Same for projectile collision event processing if multi-target fairness matters.

### F8. [SEVERITY: Medium] Class combat passives are hardcoded; ability pipeline is data-driven — mixed maturity

- **Where:** Data: `content/classes/*.ron` → `AbilityDef` / `AbilityEffect`; server `dispatch_cast` validates class, cooldown, range, spawns Mechanic/Projectile/LeapImpulse. Hardcoded: `combat/buff.rs` (`ravager_mods`, rage constants, finishing blow threshold), `RavagerRageSystem` registered only on server net plugin; `PLAYER_PREFAB = "ravager"` constant in `receive.rs`.
- **What:** The cast pipeline is the right shape for an ARPG (intent → validate → schedule). Progression of *effects* beyond three enums and one class’s passives requires Rust changes. No general buff framework by design (commented), which is fine until a second melee class needs stacks.
- **Why it matters:** Content authors can retune Rend/Cleave/Onslaught without recompiling, but cannot express “new passive” or “new effect kind” without engineering. Forced Ravager prefab blocks the shipped Human kit in multiplayer.
- **Recommendation:** Keep passives code-backed until a third class needs the framework; immediately make `PLAYER_PREFAB` (or DB class field) data-selected so Human bolt/blast is playable online. Content-lint ability ids against VFX/anim names (partially present).

### F9. [SEVERITY: Medium] Enemy AI is clean and scalable, but engagement is nearest-player only with no leashing/nav

- **Where:** `enemies/mod.rs` `EnemyAISystem`; `enemies/behavior.rs` data-driven Melee/Ranged + `BehaviorRegistry`; chapter prefabs set speed/aggro/attack; chapter-02 comments document “no navmesh — enemies beeline” placement discipline.
- **What:** Spatial-grid aggro path kicks in at ≥64 players; provoked passives work; cooldowns are dt-accumulated on the component (deterministic). There is no leash-to-camp, no pathfinding around Solid buildings, no pack roles, no mechanic telegraphs from AI.
- **Why it matters:** Chapter-02 placement carefully keeps aggro bubbles off building hitboxes because AI will otherwise embed in cottages. That is content compensating for missing navigation. Fine for field packs; brittle for towns, chokepoints, and world bosses.
- **Recommendation:** Add camp leash (max distance from `CampMember` slot origin → drop Provoked / walk home) before adding navmesh. Keep `EnemyBehavior` trait as the chapter override seam (already unused by chapters — still the right hook).

### F10. [SEVERITY: Medium] World clock / events are well-built; population authority depends on prefab presence and empty chapters

- **Where:** `world/mod.rs` `WorldTime`, `WorldEventSystem`, pure `active_event` / `day_night_light`; server publishes `WorldTime` each Input tick from shared `world_origin`; clients tint from clock; camps use `CampMember` occupancy rather than cached entity ids.
- **What:** Synchronization-by-construction works. Wave caps, no burst catch-up, window-end reap, and mid-join via AOI replication are all correct. Coupled with F1, production events that spawn `"grunt"` are one missing prefab dir away from silent `log::error` spawn failures.
- **Why it matters:** Living-world fantasy is implemented in code and RON but not reliably activated end-to-end in the default server topology.
- **Recommendation:** After re-enabling chapters, add a startup prefab-resolution check for every `WorldEventsDef` / `ChapterDef` prefab string. Fail boot on missing content (same panic policy as `load_chapter`).

### F11. [SEVERITY: Medium] Death/respawn is placeholder — instant ring respawn, no death state machine

- **Where:** `combat/death.rs` (Health≤0 → events → despawn); `server/.../receive.rs` `respawn_dead` + `XpCarrySystem`; e2e covers respawn and XP carry.
- **What:** No downed state, no release-to-graveyard, no durability loss, no invuln frames on respawn, no client death presentation contract beyond entity leave + new Welcome. Combat can kill; the world pretends you never died beyond XP retention.
- **Why it matters:** Action-RPG stakes and anti-suicide-into-camp gameplay need at least a short respawn lock and spawn safety. Current behavior is fine for netcode bring-up, not for shippable combat.
- **Recommendation:** Minimal state: corpse timer → respawn at checkpoint with brief i-frames; keep XP carry. Do not block on full FF14 duty wipe logic yet.

### F12. [SEVERITY: Low] ECS ownership and plugin split are exemplary — preserve them

- **Where:** `vordar-game/src/lib.rs` ownership map; `CoreGamePlugin` vs `GameComponentsPlugin`; chapter `install` vs `install_content`; server-only systems (`MechanicResolveSystem`, `RavagerRageSystem`, net receive/broadcast) stay out of the shared crate where appropriate (`Mechanic` component is shared; resolve is server).
- **What:** Networked clients get loaders/prefabs without sim; chapters contribute content without engine edits; EventBus carries `MoveIntent`, `DamageDealt`, `Killed`, `HealthDepleted`, collisions. This matches DESIGN.md §5–§6 and the architecture diagram (shared game rules under both apps).
- **Why it matters:** Most MMO prototypes collapse this boundary and then cannot host dedicated servers. Vordar already has the hard split.
- **Recommendation:** Keep resolve/net/persistence out of `vordar-game`. If sandbox needs resolve, put a thin `vordar-game` optional test helper or a tiny `local_authority` module — do not drag QUIC into the shared crate.

### F13. [SEVERITY: Low] Movement invariants are explicit and shared — residual gaps are dynamic colliders and Y

- **Where:** `player::movement_velocity`, `motion::step` + `PlayRadius(65)`, `leap_velocity` / `LeapSystem` order (between intent and integrate), client `predict_step` = step + `anchored_push`, server history stores applied velocity (dash-truth rewind test in `mechanics.rs`).
- **What:** Own-player prediction intentionally collides only with **anchored** statics, not other movers. Sim is ground-plane (Y ignored in leap/dir). Boundary clamp prevents scenic-hill burial. Intent validation caps speed via unit dir × server speed; positions never trusted from client.
- **Why it matters:** Invariants are the right ones for zone ARPG. Player-player shove and non-anchored props will desync prediction until included or explicitly excluded forever.
- **Recommendation:** Freeze the contract in a short `docs/` gameplay netcode note: “prediction solids = Anchored+Solid only.” Add a unit test that two players overlapping do not expect prediction match.

### F14. [SEVERITY: Low] Chapter modularity works; density is sparse and intentional

- **Where:** `ChapterRegistry` deps (`chapter02` requires `chapter01`); camps golden-angle slots; chapter-01 ~21 residents + respawn timers; chapter-02 town initial_spawns + 10 camp slots; AOI 40; blood-moon +8 wave cap.
- **What:** Architecture supports linked content modules and per-zone chapter install. Current density is a quiet starter field, not RO Prontera. Enemy AI grid thresholds anticipate hundreds of players before O(E·P) hurts.
- **Why it matters:** Networking/persistence will hit limits before ECS does (DESIGN.md §7) — agreed. Content density is currently so low that many systems (AOI stagger, grid aggro, wave caps) are under-exercised outside soak/e2e harnesses.
- **Recommendation:** One “dense camp” stress chapter or scripted spawn table for CI soak, separate from hand-authored starter pacing.

### F15. [SEVERITY: Low] Test coverage is strong on pure mechanics and e2e net combat; thin on full-sim integration inside vordar-game

- **Where:** In-crate tests: damage triangle/crits, projectiles, rage, leap, movement clamp, separation, camps, world events, zones validate, enemy AI, XP attribution, `predict_step` parity. Server e2e: scheduled AoE, rend kills camp enemy, onslaught dash+resolve, blood moon, AOI, persistence, security. Client: prediction/reconciliation e2e, content_lint for visuals.
- **What:** Almost no multi-system “full CoreGamePlugin tick headless” test that drives Input→Update→CollisionResolve→PostUpdate with a cast intent fixture outside server binaries. Mechanic resolve unit tests cover despawn queue and dash rewind, not damage application matrix.
- **Why it matters:** Regressions in system order (e.g. rage after mechanics, death after contact) are only caught at e2e latency. Pure tests are excellent anchors; pipeline tests are the missing belt.
- **Recommendation:** One headless app test in `vordar-game` or server lib: spawn player+enemy, inject `Mechanic` due now, run resolve+death+xp, assert health/XP/despawn. Keep it clock-injected (no sleeps).

### F16. [SEVERITY: Info] Anti-cheat and intent protocol match DESIGN.md closely

- **Where:** `receive.rs` `validate_intent` (monotonic seq/t, arrival deadline `max(RTT, MAX_REWIND)+margin`, future slack), move dir finite/≤1+eps, cast cooldown server-side, range checks for Scheduled/Leap, positions from integration only; `MAX_REWIND_MICROS = 200_000`.
- **What:** Caps are in protocol path from day one as required. Client clock cannot award hits. Remaining cheat classes (bots, ESP) correctly treated as out-of-scope for netcode.
- **Why it matters:** Retrofitting rewind caps later is how action MMOs get lag-switch disasters. This is a preserved strength.
- **Recommendation:** Metrics already record rejects — expose a simple rate dashboard before public tests. No architecture change.

### F17. [SEVERITY: Info] Projectile combat is intentionally not snapshot-fair

- **Where:** `projectile.rs` header; collision at broadphase/narrowphase “now”; favor-the-shooter; human bolt + enemy ranged profiles.
- **What:** DESIGN.md allows this for slow dodgeable bolts and reserves snapshot fairness for telegraphed areas. Implementation matches. TTL and side filters are tested.
- **Why it matters:** Hybrid model is coherent if content keeps projectile speeds readable and puts high-stakes damage on Scheduled/Leap.
- **Recommendation:** Content guideline: lethal boss damage = Scheduled; filler = Projectile/Contact. Lint optional max damage×speed product later.

## Strengths worth preserving

1. **Ownership-shaped modules** — entity types own components/behaviors; generic systems stay generic (`lib.rs` contract is accurate).
2. **Intent-only input** — `MoveIntent` / cast messages; no keyboard reads in `vordar-game`.
3. **Pure shared movement math** — `movement_velocity`, `step`, `leap_velocity`, `predict_step` used by server, sandbox, and client replay; dash history stores applied velocity (not WASD), with a regression test proving the distinction.
4. **Scheduled snapshot spine** — absolute T broadcast, client telegraph fill from synced time, server rewind cap, AOI-scoped `MechanicScheduled` / `HitResult`.
5. **Deterministic damage** — seeded crits, type triangle, True damage escape, optional `CombatStats` passthrough for untyped content.
6. **Chapter registry with content vs sim install** — correct client/server split; `requires` chain for prefab inheritance.
7. **World events by clock math** — no event-state replication; wave caps without burst debt; day/night pure function.
8. **Camps as resident populations** — slot components, golden-angle positions, respawn timers; world exists without players.
9. **Enemy behavior seam** — data-driven default + registry override without forking AI system.
10. **Evidence-heavy tests** — unit purity + multiplayer e2e combat/persistence/security is above typical indie MMO prototype standard.

## Suggested priority order

1. **Reconnect content to live zones (F1, F10)** — set chapter ids on `zones.ron`, boot-check prefabs for events/camps. Without this, sim quality is academic.
2. **Local combat iteration path (F3)** — sandbox or listen-server that resolves mechanics offline so Scheduled/Leap is authorable without full net bring-up.
3. **Contact faction rules (F4)** — close the side-filter hole before new collision archetypes ship.
4. **Mechanic shape + encounter data (F2)** — grow the snapshot model toward DESIGN.md boss timelines; keep resolve/rewind.
5. **Separation/prediction order stability (F7)** — sort pair processing; reduce town-wall snaps.
6. **Leash + death minimal loops (F9, F11)** — stop AI museum-quality embedding and empty-stakes deaths.
7. **Playable class selection (F8)** — stop hardcoding `PLAYER_PREFAB = "ravager"`; keep passives code-backed until needed.
8. **Pipeline integration test (F15)** and cast-time vs resolve-cadence lint (F6).
9. **NPC rewind policy decision (F5)** — document or cheap-freeze during enemy telegraphs when those exist.
10. **Density stress content (F14)** — exercise AOI/AI grid paths continuously in CI.

---

*Evidence base: `README.md`, `.claude/DESIGN.md`, `docs/architecture.mmd`, `game/vordar-game/src/**`, `game/chapter-01|02`, `content/{chapters,classes,prefabs,zones,vfx}`, `server/vordar-server/src/{lib,main,net/**}`, `client/vordar-client/src/{net/**,cast,telegraph,sandbox}`, and associated unit/e2e tests. `docs/reviews/**` was not consulted.*
