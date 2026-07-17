// Resource type-map and deferred spawn/despawn plumbing shared across the engine

use hecs::Entity;
use std::any::{Any, TypeId};
use std::collections::HashMap;

// ── Resources (type-map) ──────────────────────────────────────────────────────
//
// A type-safe heterogeneous map. The engine inserts its own resources
// (InstancePool, EventBus, SpawnQueue, ...). Games add their own.
// Access via ctx.resources.get_mut::<InstancePool>().
//
// Analogous to Kotlin's Map<KClass<*>, Any> but with compile-time type safety.

pub struct Resources {
    data: HashMap<TypeId, Box<dyn Any>>,
}

impl Default for Resources {
    fn default() -> Self {
        Self::new()
    }
}

impl Resources {
    pub fn new() -> Self { Self { data: HashMap::new() } }

    pub fn insert<T: Any>(&mut self, value: T) {
        self.data.insert(TypeId::of::<T>(), Box::new(value));
    }

    pub fn get<T: Any>(&self) -> Option<&T> {
        self.data.get(&TypeId::of::<T>())?.downcast_ref()
    }

    pub fn get_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.data.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }

    pub fn contains<T: Any>(&self) -> bool {
        self.data.contains_key(&TypeId::of::<T>())
    }
}

// ── SpawnContext ──────────────────────────────────────────────────────────────
//
// Passed to spawn_prefab and the SpawnQueue/DespawnQueue closures.
// Access engine resources via self.resources.get_mut::<T>().
//
// Standard resources inserted by the engine:
//   InstancePool   (engine-renderer) — alloc/free render slots
//   SpawnQueue     (engine-app)      — queue further spawns
//   DespawnQueue   (engine-app)      — queue despawns
//   EventBus       (engine-app)      — emit events

pub struct SpawnContext<'a> {
    pub world:     &'a mut hecs::World,
    pub resources: &'a mut Resources,
}

// ── SpawnQueue / DespawnQueue ─────────────────────────────────────────────────
//
// Systems never mutate the world mid-frame. Push requests here instead.
// Drained by engine-app during Phase::SpawnFlush and Phase::DespawnFlush.

/// A deferred callback run with world access once a spawn/despawn is flushed.
pub type SpawnContextHook = Box<dyn FnOnce(&mut SpawnContext) + Send>;

pub struct SpawnQueue(pub Vec<SpawnContextHook>);

impl Default for SpawnQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl SpawnQueue {
    pub fn new() -> Self { Self(Vec::new()) }
    pub fn push(&mut self, f: impl FnOnce(&mut SpawnContext) + Send + 'static) {
        self.0.push(Box::new(f));
    }
}

pub struct DespawnQueue(pub Vec<(Entity, Option<SpawnContextHook>)>);

impl Default for DespawnQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl DespawnQueue {
    pub fn new() -> Self { Self(Vec::new()) }
    /// Queue an entity for removal.
    /// Pass Some(hook) to run cleanup (free render slot, emit event, etc.) before despawn.
    /// The hook is defined by the caller — engine-app stays unaware of what's inside.
    pub fn push(&mut self, entity: Entity, hook: Option<SpawnContextHook>) {
        self.0.push((entity, hook));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Resources ────────────────────────────────────────────────────────────

    #[test]
    fn resources_insert_and_get() {
        let mut r = Resources::new();
        r.insert(42u32);
        assert_eq!(r.get::<u32>(), Some(&42));
    }

    #[test]
    fn resources_get_mut_allows_mutation() {
        let mut r = Resources::new();
        r.insert(0u32);
        *r.get_mut::<u32>().unwrap() = 99;
        assert_eq!(r.get::<u32>(), Some(&99));
    }

    #[test]
    fn resources_contains_returns_false_before_insert() {
        let r = Resources::new();
        assert!(!r.contains::<u32>());
    }

    #[test]
    fn resources_contains_returns_true_after_insert() {
        let mut r = Resources::new();
        r.insert(1u32);
        assert!(r.contains::<u32>());
    }

    #[test]
    fn resources_different_types_dont_collide() {
        let mut r = Resources::new();
        r.insert(1u32);
        r.insert(2u64);
        assert_eq!(r.get::<u32>(), Some(&1));
        assert_eq!(r.get::<u64>(), Some(&2));
    }

    #[test]
    fn resources_insert_overwrites() {
        let mut r = Resources::new();
        r.insert(1u32);
        r.insert(2u32);
        assert_eq!(r.get::<u32>(), Some(&2));
    }

    // ── SpawnQueue ────────────────────────────────────────────────────────────

    #[test]
    fn spawn_queue_push_increments_len() {
        let mut q = SpawnQueue::new();
        assert_eq!(q.0.len(), 0);
        q.push(|_ctx| {});
        assert_eq!(q.0.len(), 1);
        q.push(|_ctx| {});
        assert_eq!(q.0.len(), 2);
    }

    // ── DespawnQueue ──────────────────────────────────────────────────────────

    #[test]
    fn despawn_queue_push_no_hook() {
        let mut world = hecs::World::new();
        let e = world.spawn(());
        let mut q = DespawnQueue::new();
        q.push(e, None);
        assert_eq!(q.0.len(), 1);
        assert_eq!(q.0[0].0, e);
        assert!(q.0[0].1.is_none());
    }

    #[test]
    fn despawn_queue_push_with_hook() {
        let mut world = hecs::World::new();
        let e = world.spawn(());
        let mut q = DespawnQueue::new();
        q.push(e, Some(Box::new(|_ctx| {})));
        assert_eq!(q.0.len(), 1);
        assert!(q.0[0].1.is_some());
    }

}
