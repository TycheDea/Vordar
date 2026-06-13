# Vordar

A zone-based online action-RPG in the spirit of Ragnarok Online, with FF14-style
**scheduled-snapshot combat**: telegraphed ground mechanics resolve at a fixed,
clock-synchronized server moment, so "you see hack-and-slash but it resolves as
timed, fair actions." Built on a custom Rust engine (`smirk`).

During development it ships as a single-player **server + client pack**, but the
architecture stays MMO-shaped throughout: headless authoritative zone servers,
QUIC networking, client-side prediction + reconciliation, and interest management.

## Workspace layout

| Path | What it is |
|------|------------|
| `smirk/` | The engine, game-agnostic: `engine-core` (ECS, spatial grid, prefabs), `engine-app` (App + scheduler + EventBus), `engine-renderer` (wgpu/winit/egui), `engine-physics`, `engine-audio`, `engine-net` (QUIC transport) |
| `game/vordar-protocol` | Wire messages (postcard), versioned |
| `game/vordar-game` | Shared deterministic simulation, organized by ownership: `player/`, `enemies/`, `combat/`, `motion/`, `world/` — no render/window/input deps |
| `game/chapter-01` | A content module. Chapters are *linked modules* with a dependency chain |
| `server/vordar-server` | Headless authoritative server — one zone instance per thread |
| `client/vordar-client` | Presentation: input→intents, camera, snapshot replication, UI. Bins: `sandbox` (offline) and `vordar` (networked) |
| `content/` | RON data (prefabs, chapters, zones, world events) + textures |

## Build & run

Prerequisites: a stable Rust toolchain, and a GPU for the client. Run all commands
from the workspace root — `content/` is resolved relative to the working directory.

```sh
# Offline sandbox: one process, full sim, no networking (fastest iteration)
cargo run -p vordar-client --bin sandbox

# Networked: start the server, then connect a client (another terminal)
cargo run -p vordar-server
cargo run -p vordar-client --bin vordar [server_ip:port]   # default 127.0.0.1:5151

# Tests
cargo test --workspace
```

## Controls

| Input | Action |
|-------|--------|
| `WASD` | Move |
| Arrow keys | Orbit / pitch the camera |
| Mouse wheel | Zoom |
| Left click | Bolt (primary attack) |
| `Q` | Blast (AoE — networked play only) |
| `C` | Cycle camera projection (perspective / isometric / top-down) |
| `F3` | Dev stats overlay |
| `Esc` | Pause menu |

## Environment knobs

| Variable | Effect |
|----------|--------|
| `VORDAR_USER=name` | Character to play (default `player`) |
| `VORDAR_DB=path` | Server character database (default `vordar.db` in cwd) |
| `VORDAR_LATENCY_MS=150` | Artificial round-trip latency (client) |
| `VORDAR_PREDICT=0` | Disable client prediction (Phase-1 server-driven feel) |

## Design & roadmap

The design rationale (`DESIGN.md`) and the build roadmap (`tasks/`) live under
`.claude/` and are kept local (gitignored), not part of the committed tree.