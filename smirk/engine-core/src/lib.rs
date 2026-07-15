// engine-core — foundation layer, no rendering, physics, or audio
//
// Owns:
//   - Base traits: Spawnable, EntityLifecycle, Collidable, Renderable
//   - Core components: Transform, Velocity, Health, Hitbox, CellOccupant, RenderShape
//   - Spatial hash grid (proximity queries — the engine's core primitive)
//   - Math re-exports (glam)
//
// Rule: nothing in this crate may depend on engine-renderer, engine-physics,
//       engine-audio, or engine-app.

pub use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
pub use hecs::World;

pub mod components;   // Transform, Velocity, Health, Hitbox, RenderShape, CellOccupant, ...
pub mod prefab;       // ComponentRegistry + PrefabLibrary — data-driven entity definitions
pub mod spatial;      // SpatialGrid — "give me all entities within radius R"
pub mod traits;       // Spawnable, EntityLifecycle, Collidable, Renderable
