// engine-app — game loop, system scheduler, app builder
//
// Owns:
//   - App builder: register systems, configure phases, load config
//   - Scheduler: topological sort at startup, fixed execution order at runtime
//   - EventBus: typed single-frame events between systems
//   - SpawnFlush / DespawnFlush: drain SpawnQueue and DespawnQueue each frame
//   - Resources: type-map populated at startup, passed to every system
//
// Dependency direction:
//   engine-app → engine-core (traits, components, world)
//   engine-app is NOT allowed to depend on engine-renderer or engine-physics directly.
//   Those crates register themselves with the app via their own init functions.
//
// Frame execution order:
//   Input → PreUpdate → Update → SpawnFlush → Collision → CollisionResolve
//   → DespawnFlush → PostUpdate → RenderSync → Render
//
// Thread affinity: a built App cannot move between threads — systems are not
// Send, so App::run/run_headless/run_ticks must be called on the thread that
// built it (each server zone gets its own thread for exactly this reason).

pub mod app;             // App struct + builder API
pub mod plugin;          // Plugin trait — the extension point for subsystems, game modules, chapters
pub mod prefab_plugin;   // PrefabPlugin — ComponentRegistry + PrefabLibrary resources
pub mod logger;          // minimal stderr backend for the `log` facade
pub mod dev_stats;       // DevStats resource — fps/frame-time + custom debug counters (F3 overlay)
pub mod config;          // WindowConfig — loaded from RON via App::configure()
#[cfg(feature = "winit")]
pub mod input;           // KeyboardState resource
#[cfg(feature = "winit")]
pub mod winit_processor; // WinitEventProcessor — type-erased bridge for subsystem event forwarding
#[cfg(feature = "winit")]
mod app_loop;       // ApplicationHandler impl (winit event loop)
pub mod scheduler;  // Phase enum, SystemOrder, DAG builder, topological sort
pub mod events;     // EventBus — emit and read typed events
pub mod time;       // Time resource — frame_dt for render systems
pub(crate) mod flush;   // SpawnFlushSystem, DespawnFlushSystem
