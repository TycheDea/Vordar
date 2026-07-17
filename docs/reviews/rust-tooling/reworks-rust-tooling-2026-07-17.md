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

> **finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 (user-decides — ask at loop launch) → finding 8 → finding 9 →
> finding 10 → finding 11 (docs-only, micro) → finding 12.**
>
> Finding 1 goes first because it is the infrastructure every later diff is
> verified against. 2–3 shrink the tree before anything recompiles it
> repeatedly. 7 precedes 9 (edition work should run on the newest toolchain)
> and 12 (a toolchain bump shifts codegen, so re-baselining before it would be
> wasted). 12 is last because findings 2, 3, 7, 9, and 10 each move numbers.

## Carried forward from previous report

None — first rust-tooling audit.

## Resolved since last report

None — first rust-tooling audit.
