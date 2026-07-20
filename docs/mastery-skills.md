# Skills to master for this repo

This repo is a from-scratch Rust game engine (`smirk`) plus an MMO-architecture
game (`vordar`) built on it — no Bevy, no Unity, everything hand-rolled.

## Core language & tooling

1. **Advanced Rust** — ownership across long-lived game state, trait design for
   engine/game boundaries, workspace organization (13 crates with shared
   `workspace.dependencies`), feature flags, profiling release vs. bench profiles.
2. **Cargo workspace discipline** — how `smirk/*` (engine) stays decoupled from
   `game/*`, `client/`, `server/`; where a new system belongs.
3. **Criterion benchmarking & profiling** — headless benchmarks guide foundation
   fixes before content lands; flamegraph symbols are deliberately kept in bench
   builds.

## Rendering & graphics

4. **wgpu (v29) and the modern GPU pipeline** — render passes, bind groups,
   buffers, WGSL shaders. The renderer is hand-written in `engine-renderer`.
5. **Skeletal animation & skinning** — CPU/GPU skinning, joint hierarchies,
   animation clip latching, grounding probes that keep soles on the floor.
6. **glTF internals** — the importer uses the `gltf` crate with KHR extensions;
   the character pipeline preprocesses meshes with gltf-transform.
7. **egui integration** — the renderer draws it; the game registers `UiLayers`
   callbacks.

## Game architecture

8. **ECS with hecs** — archetype-based ECS without Bevy's scheduler; system
   ordering is designed by hand.
9. **Game loop / app structure** — `engine-app` + `winit` 0.30's event-loop
   model, fixed timestep vs. render frames.
10. **Custom physics** — `engine-physics` is hand-rolled (no rapier): collision,
    grounding, movers clamped to the playable radius.
11. **3D math with glam** — quaternions, transforms, bone-space vs. world-space
    reasoning (most animation bugs live here).

## Networking & server

12. **QUIC via quinn + rustls/rcgen** — connection lifecycle, streams vs.
    datagrams, self-signed cert generation for the dev single-player pack.
13. **Client/server state replication** — the `vordar-protocol` crate,
    `postcard` compact serialization, and the server-authoritative MMO model
    (even though dev runs as a single-player pack).
14. **tokio async** — runtime setup, bridging async net code with the
    synchronous game loop.
15. **SQLite persistence** — `rusqlite` on the server side (`vordar.db`).

## Content pipeline & art tooling

16. **Character asset pipeline** — the canonical Mixamo skeleton convention and
    its clip library, MPFB2/MHCLO parametric bodies and garments, FBX→glTF
    conversion, the `scripts/ai-pipeline` character chain.
17. **AA art direction constraints** — semi-realistic dark fantasy is locked;
    asset decisions are made autonomously within that.

## Project-specific meta

18. **Docs/diagram convention** — architecture is maintained as Mermaid
    (`.mmd` → SVG via `scripts/render-mmd.sh`).
19. **Verification style** — headless checks only; manual feel-checks happen in
    the running game. Diagnostics are written as automated probes (e.g. "no clip
    may pose joints below the floor").

## Top three by leverage

- **wgpu / skeletal animation** — where current work lives.
- **hecs ECS + the engine/game split** — where every feature lands.
- **QUIC replication model** — the architectural bet the MMO design rests on.
