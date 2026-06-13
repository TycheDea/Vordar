// EventBus — typed single-frame events
//
// Events live for exactly one frame. Cleared at Phase::Input.
// Any system can emit; any system in a later phase can read.
//
// Usage:
//   // emit
//   events.emit(EnemyDied { entity, position });
//
//   // read (in a later phase)
//   for ev in events.read::<EnemyDied>() { ... }
//
// Built-in engine events: EntitySpawned, EntityDespawned,
//   CollisionStarted, CollisionEnded, HealthDepleted
//
// Implementation note: each event type gets one typed Vec<E>.
// emit() is a Vec push — zero heap allocation per event after the first frame.
// read() downcasts once per call, not once per event.
// clear() preserves Vec capacity — zero allocation on subsequent frames.

use std::any::{Any, TypeId};
use std::collections::HashMap;

trait AnyQueue: Send + Sync {
    fn clear(&mut self);
    fn as_any(&self)     -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct TypedQueue<E> {
    events: Vec<E>,
}

impl<E: Send + Sync + 'static> AnyQueue for TypedQueue<E> {
    fn clear(&mut self)          { self.events.clear(); }
    fn as_any(&self)             -> &dyn Any     { self }
    fn as_any_mut(&mut self)     -> &mut dyn Any { self }
}

pub struct EventBus {
    queues: HashMap<TypeId, Box<dyn AnyQueue>>,
}

impl EventBus {
    pub fn new() -> Self { Self { queues: HashMap::new() } }

    pub fn emit<E: Send + Sync + 'static>(&mut self, event: E) {
        self.queues
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(TypedQueue::<E> { events: Vec::new() }))
            .as_any_mut()
            .downcast_mut::<TypedQueue<E>>()
            .expect("EventBus type mismatch — TypeId collision")
            .events
            .push(event);
    }

    pub fn read<E: Send + Sync + 'static>(&self) -> impl Iterator<Item = &E> {
        self.queues
            .get(&TypeId::of::<E>())
            .and_then(|q| q.as_any().downcast_ref::<TypedQueue<E>>())
            .map(|q| q.events.iter())
            .into_iter()
            .flatten()
    }

    /// Called at the start of Phase::Input each frame. Clears events but keeps
    /// Vec capacity — no allocation on subsequent frames.
    pub fn clear(&mut self) {
        for queue in self.queues.values_mut() {
            queue.clear();
        }
    }
}

// ── Built-in engine events ────────────────────────────────────────────────────

use hecs::Entity;

pub struct EntitySpawned   { pub entity: Entity }
pub struct EntityDespawned { pub entity: Entity }
pub struct HealthDepleted  { pub entity: Entity }
pub struct CollisionStarted { pub a: Entity, pub b: Entity }
pub struct CollisionEnded   { pub a: Entity, pub b: Entity }

#[cfg(test)]
mod tests {
    use super::*;

    struct Damage(u32);
    struct Heal(u32);

    #[test]
    fn emit_then_read_returns_event() {
        let mut bus = EventBus::new();
        bus.emit(Damage(42));
        let events: Vec<_> = bus.read::<Damage>().collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, 42);
    }

    #[test]
    fn multiple_emits_all_readable() {
        let mut bus = EventBus::new();
        bus.emit(Damage(1));
        bus.emit(Damage(2));
        bus.emit(Damage(3));
        let values: Vec<u32> = bus.read::<Damage>().map(|e| e.0).collect();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn clear_removes_all_events() {
        let mut bus = EventBus::new();
        bus.emit(Damage(1));
        bus.clear();
        assert_eq!(bus.read::<Damage>().count(), 0);
    }

    #[test]
    fn different_types_do_not_interfere() {
        let mut bus = EventBus::new();
        bus.emit(Damage(10));
        bus.emit(Heal(20));
        assert_eq!(bus.read::<Damage>().count(), 1);
        assert_eq!(bus.read::<Heal>().count(), 1);
        assert_eq!(bus.read::<Damage>().next().unwrap().0, 10);
        assert_eq!(bus.read::<Heal>().next().unwrap().0, 20);
    }

    #[test]
    fn read_empty_bus_returns_nothing() {
        let bus = EventBus::new();
        assert_eq!(bus.read::<Damage>().count(), 0);
    }

    #[test]
    fn clear_preserves_capacity() {
        let mut bus = EventBus::new();
        for i in 0..100u32 { bus.emit(Damage(i)); }
        bus.clear();
        // After clear, reading returns nothing but the Vec retains its capacity.
        assert_eq!(bus.read::<Damage>().count(), 0);
        // Second round of emits must not re-allocate (capacity already there).
        for i in 0..100u32 { bus.emit(Damage(i)); }
        assert_eq!(bus.read::<Damage>().count(), 100);
    }
}
