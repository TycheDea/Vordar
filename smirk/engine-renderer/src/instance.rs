use std::mem::size_of;

// ── Per-instance GPU data ─────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SdfInstance {
    pub(crate) model:        [[f32; 4]; 4],  // offset  0 — 64 bytes
    pub(crate) color:        [f32; 3],       // offset 64 — 12 bytes
    pub(crate) shape_type:   u32,            // offset 76 —  4 bytes
    pub(crate) shape_params: [f32; 4],       // offset 80 — 16 bytes
}                                            // total: 96 bytes

impl SdfInstance {
    pub(crate) fn zeroed() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

pub(crate) const INSTANCE_SIZE: usize = size_of::<SdfInstance>(); // 96

// ── Slot handles (opaque to callers outside engine-renderer) ──────────────────

/// Stable index into `InstancePool.slots` for a single-shape entity.
#[derive(Copy, Clone)]
pub struct InstanceSlot(pub usize);

/// One stable index per sub-shape in a `ShapeGroup` entity.
/// NOT Copy — the entity owns it and frees all indices on despawn.
pub struct ShapeGroupSlots(pub Vec<usize>);

// ── Instance pool ─────────────────────────────────────────────────────────────

pub(crate) struct InstancePool {
    pub(crate) slots: Vec<SdfInstance>,
    pub(crate) dirty: Vec<bool>,
               free:  Vec<usize>,
}

impl InstancePool {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            dirty: Vec::with_capacity(capacity),
            free:  Vec::with_capacity(capacity),
        }
    }

    /// Allocate a slot. Reuses a freed index if available, otherwise grows the Vec.
    pub(crate) fn alloc(&mut self) -> usize {
        if let Some(idx) = self.free.pop() {
            self.dirty[idx] = true;
            idx
        } else {
            let idx = self.slots.len();
            self.slots.push(SdfInstance::zeroed());
            self.dirty.push(true);
            idx
        }
    }

    /// Number of slots currently in use (allocated minus freed).
    pub(crate) fn used(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    /// Free a slot. Zeroes the slot data (uploaded once so the instance disappears
    /// from the GPU), then returns the index to the free list.
    pub(crate) fn free(&mut self, idx: usize) {
        self.slots[idx] = SdfInstance::zeroed();
        self.dirty[idx] = true;
        self.free.push(idx);
    }
}
