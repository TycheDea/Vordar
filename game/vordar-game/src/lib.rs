// vordar-game — the shared game simulation, organized by what OWNS the code:
// each entity type's module holds its components, behavior, and tuning hooks;
// shared mechanics stay generic systems. Compiled by BOTH the client (for
// prediction) and the server (as authority), so nothing here may touch the
// renderer, the window, or input devices — gameplay reacts to intent events
// only. Content (stats, camps, prefabs) belongs to chapter crates and asset
// files, not here.
//
//   player/   — the player: component, movement rule, skill book
//   enemies/  — Enemy component, engagement model, per-archetype behaviors
//   combat/   — projectiles, contact damage, death, scheduled mechanics
//   motion/   — velocity integration, solid separation
//   world/    — world clock/events, zones, chapters (linked modules), camps
//   events.rs — intent events (the only way input reaches the simulation)

pub mod combat;
pub mod enemies;
pub mod events;
pub mod motion;
pub mod player;
pub mod plugin;
pub mod world;

pub use plugin::{CoreGamePlugin, GameComponentsPlugin};

// The heavily-used names, at the crate root.
pub use combat::{ContactDamage, Mechanic, OnDeath, Projectile};
pub use enemies::{AttackKind, Enemy, Provoked};
pub use player::Player;

// Short content-facing module paths (`vordar_game::skills::skill`, ...).
pub use player::skills;
pub use world::{chapter, zones};
