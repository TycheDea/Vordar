use engine_core::World;
use hecs::Entity;
use std::collections::HashMap;

/// Stable wire-id allocator for entities replicated to clients (Finding 1 of
/// docs/reviews/networking/plan-networking-rework-5-2026-07-13.md): players,
/// enemies, bolts, and hazards each get a unique, monotonically increasing u32
/// on their first snapshot. Ids are NOT reused when an entity despawns — the
/// wire-side assumption is that an id uniquely identifies a GENERATION of an
/// entity, so if a hecs slot is reused at a new generation (a new entity
/// spawned at the same slot), the new entity must get a new id. Tracking both
/// the entity and its generation via hecs::Entity's typestate (opaque slot +
/// generation) makes the invariant automatic: an old Entity handle from a
/// despawned entity fails to match a fresh spawn at that slot, so
/// generations mean a reused `Entity` slot compares unequal to the old one
/// stored here, so `sweep` can drop a despawned entity's entry without any
/// risk of a stale id later aliasing a new entity.
pub(super) struct ReplIds {
    pub(super) by_entity: HashMap<Entity, u32>,
    next: u32,
}

impl ReplIds {
    pub(super) fn new() -> Self {
        Self { by_entity: HashMap::new(), next: 1 }
    }

    /// The existing wire id for `entity`, or a freshly assigned one.
    pub(super) fn id_for(&mut self, entity: Entity) -> u32 {
        if let Some(&id) = self.by_entity.get(&entity) {
            return id;
        }
        let id = self.next;
        self.next += 1;
        self.by_entity.insert(entity, id);
        id
    }

    /// Drop entries for entities no longer alive — bolts and dead enemies
    /// despawn continuously, so without this the map would grow unboundedly
    /// over a zone's lifetime.
    pub(super) fn sweep(&mut self, world: &World) {
        self.by_entity.retain(|&entity, _| world.contains(entity));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finding 1 of docs/reviews/networking/plan-networking-rework-5-2026-07-13.md:
    /// `ReplIds` must hand back the SAME id on every subsequent lookup of an
    /// entity, and assign distinct, monotonically increasing ids to distinct
    /// entities — the wire-compactness contract the whole finding rests on.
    #[test]
    fn repl_ids_assign_stable_monotonic_ids() {
        let mut world = World::new();
        let e1 = world.spawn(());
        let e2 = world.spawn(());
        let mut ids = ReplIds::new();

        let id1_first = ids.id_for(e1);
        let id1_again = ids.id_for(e1);
        assert_eq!(id1_first, id1_again, "the same entity must always get the same wire id");

        let id2 = ids.id_for(e2);
        assert_ne!(id1_first, id2, "distinct entities must get distinct wire ids");
        assert!(id2 > id1_first, "ids are assigned monotonically as entities are first referenced");
    }

    /// Finding 1 of docs/reviews/networking/plan-networking-rework-5-2026-07-13.md:
    /// `sweep` must drop a despawned entity's mapping, and a fresh entity
    /// (even one that reuses the despawned entity's hecs slot at a new
    /// generation) must get a BRAND NEW id — never the stale one — so a
    /// lingering client reference can never alias a different live entity.
    #[test]
    fn repl_ids_sweep_drops_despawned_and_never_reuses_ids() {
        let mut world = World::new();
        let e1 = world.spawn(());
        let mut ids = ReplIds::new();
        let id1 = ids.id_for(e1);

        world.despawn(e1).unwrap();
        ids.sweep(&world);
        assert!(!ids.by_entity.contains_key(&e1), "a despawned entity's id mapping must be forgotten");

        let e2 = world.spawn(()); // may reuse e1's hecs slot at a new generation
        let id2 = ids.id_for(e2);
        assert_ne!(id1, id2, "a fresh entity must never be handed a stale wire id");
    }
}
