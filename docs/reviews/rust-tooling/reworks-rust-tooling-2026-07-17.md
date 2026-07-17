# Rust & Tooling Audit (reworks) — 2026-07-17

First audit in this domain — no prior reports to carry forward.

## Ideal end state

Same as `audit-rust-tooling-2026-07-17.md`: a lint-gated, dead-weight-free,
current-toolchain workspace whose manifests, panics, and bench baselines all
tell the truth.

## Findings (implementation order)

**No rework-scale findings this audit.** Everything found is a bounded diff a
worker can land surgically — the sweep specifically checked the usual
rework-scale suspects and cleared them:

- **Workspace architecture:** the engine→game dependency direction holds in
  every manifest (no engine crate depends on a game crate;
  test-support's dev-cycle back into vordar-server is documented as
  intentional in server/vordar-server/Cargo.toml:44-46). No crate wants
  splitting or merging — engine-audio is an empty stub, but its cost is one
  dead dep (fixes finding 2), not a structure problem.
- **Ownership/borrowing:** no `Rc/RefCell` papering, no allocation-to-dodge-
  lifetimes pattern found on the hot paths read (prefab compile-once plan,
  snapshot broadcast Arc payloads, mem::take/swap idioms in apply/gather are
  all the right shape).
- **Async architecture:** tokio is built with exactly the features it uses —
  `new_current_thread` runtimes on dedicated threads
  (smirk/engine-net/src/server.rs:152-155, client.rs:80-83) match the
  minimal `rt` feature set; no runtime redesign warranted.
- **Error handling:** the panic-heavy files' runtime sites decompose into
  mutex-poisoning locks, startup asserts, and the missing-resource idiom —
  the last is fixes finding 8, an API addition, not a redesign.

Cross-type queue (mirrored verbatim from `audit-rust-tooling-2026-07-17.md`):

> **~~finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 (user-decides — ask at loop launch) → finding 8 → finding 9 →
> finding 10 → finding 11 (docs-only, micro) → finding 12~~.**
>
> Finding 1 goes first because it is the infrastructure every later diff is
> verified against. 2–3 shrink the tree before anything recompiles it
> repeatedly. 7 precedes 9 (edition work should run on the newest toolchain)
> and 12 (a toolchain bump shifts codegen, so re-baselining before it would be
> wasted). 12 is last because findings 2, 3, 7, 9, and 10 each move numbers.
>
> Findings 1–12 done 2026-07-18 (one commit each; finding 7 also fixed a
> direct rustc-1.97.1-caused clippy regression it introduced; finding 10
> measured dev-profile dep opt-level and reverted — see
> `audit-rust-tooling-2026-07-17.md`'s Measured note; finding 12 also fixed
> a pre-existing, unrelated bench-registry break; loop-final gate 408/408).
> Findings 13 and 14 below were filed during finding 7's proof pass — not
> in this queue, not yet planned/positioned.

### 13. Toolchain bump to 1.97.1 (landed by finding 7) trips 6 new `float_literal_f32_fallback` errors in vordar-client under `-D warnings`

- **Evidence:** finding 7's Path step 1 moved the active toolchain from
  rustc 1.94.0 to 1.97.1. A bounded check (`git stash` the finding-7 diff,
  `cargo clippy -p vordar-client --all-targets -- -D warnings`) reproduces
  the same 6 errors with zero finding-7 changes applied, so this is a
  toolchain-only regression, unrelated to the rusqlite/criterion bump.
  `rustc`'s new `float_literal_f32_fallback` future-incompatible lint now
  fires (deny under `-D warnings`) at: `client/vordar-client/src/ui/action_bar.rs:106`
  (`Stroke::new(1.5, border)`) and `client/vordar-client/src/ui/minimap.rs:153,156,182,192,201`
  (`Stroke::new(1.5, ...)`, `Stroke::new(0.5, ...)`, `Stroke::new(1.0, ...)`,
  `Stroke::new(2.0, ...)` ×2) — each passes an untyped float literal where
  `Stroke::new` wants `f32` and infers `f64` first.
- **Ideal:** `cargo clippy --workspace --all-targets -- -D warnings` exits 0
  on the current toolchain.
- **Gap:** six call sites still rely on the pre-1.97 fallback-to-f32
  behavior that rustc is phasing out; the gate now denies them.
- **Suggestion:** suffix each literal with `_f32` (e.g. `1.5_f32`) at the
  six sites — mechanical, no behavior change, matches rustc's own
  suggestion in the diagnostic.
- **Path:** (1) edit the 6 literals in action_bar.rs and minimap.rs to
  `_f32` suffixes; (2) `cargo clippy --workspace --all-targets -- -D
  warnings` exits 0; (3) `cargo test -p vordar-client` (or the relevant
  nextest scope) still green.

### 14. `cargo clippy -p vordar-benches --all-targets -- -D warnings` fails on pre-existing dead code in engine-renderer

- **Evidence:** discovered while checking finding 7's "full gate" proof.
  `cargo clippy -p vordar-benches --all-targets -- -D warnings` fails with
  4 `dead_code` errors in `smirk/engine-renderer/src/camera.rs:186`
  (`write_viewport` never used) and `smirk/engine-renderer/src/ssao.rs:177-186,243,249`
  (`SsaoTargets` fields `width`/`height`/`blurred_ao` never read, `WhiteAo`
  never constructed, `WhiteAo::new` never used). A bounded check
  (`git stash` every finding-7 change, rerun the same command) reproduces
  the identical 4 errors, so this predates finding 7 and the rustc 1.97.1
  bump — it is a pre-existing gap in how vordar-benches pulls in
  engine-renderer, not a regression from either.
- **Ideal:** `cargo clippy -p vordar-benches --all-targets -- -D warnings`
  exits 0.
- **Gap:** these engine-renderer items are apparently only reachable
  through a feature/config combination vordar-benches doesn't enable
  (likely a vordar-client-only code path); no one has run this exact
  scoped-clippy command as a gate before.
- **Suggestion:** determine why `write_viewport`/`SsaoTargets`
  fields/`WhiteAo` are unreachable under vordar-benches' feature set —
  either gate them behind the right `cfg`/feature so they compile out
  cleanly, or confirm they are genuinely dead and remove them.
- **Path:** (1) `cargo clippy -p vordar-benches --all-targets -- -D
  warnings` to reproduce; (2) trace each item's reachability against
  engine-renderer's feature flags vs. what vordar-client enables that
  vordar-benches doesn't; (3) fix (cfg-gate or delete); (4) rerun the
  same clippy command plus `cargo check --workspace --all-targets` green.

## Carried forward from previous report

None — first rust-tooling audit.

## Resolved since last report

None — first rust-tooling audit.
