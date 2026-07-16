// Per-instance GPU data and slot bookkeeping for the SDF-primitive pool:
// `SdfInstance` is the 96-byte instance struct sdf_pipeline.rs steps the
// vertex shader over; `InstancePool` hands out stable `InstanceSlot`s so
// despawn/reuse never shifts another entity's instance index.

use std::mem::size_of;

// ── Per-instance GPU data ─────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SdfInstance {
    pub model:        [[f32; 4]; 4],  // offset  0 — 64 bytes
    pub color:        [f32; 3],       // offset 64 — 12 bytes
    pub shape_type:   u32,            // offset 76 —  4 bytes
    pub shape_params: [f32; 4],       // offset 80 — 16 bytes
}                                     // total: 96 bytes

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
               in_use: Vec<bool>,
}

impl InstancePool {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            dirty: Vec::with_capacity(capacity),
            free:  Vec::with_capacity(capacity),
            in_use: Vec::with_capacity(capacity),
        }
    }

    /// Allocate a slot. Reuses a freed index if available, otherwise grows the Vec.
    pub(crate) fn alloc(&mut self) -> usize {
        if let Some(idx) = self.free.pop() {
            self.dirty[idx] = true;
            self.in_use[idx] = true;
            idx
        } else {
            let idx = self.slots.len();
            self.slots.push(SdfInstance::zeroed());
            self.dirty.push(true);
            self.in_use.push(true);
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
        self.in_use[idx] = false;
        self.free.push(idx);
    }

    /// Scan in_use to collect maximal contiguous runs of live slots, storing
    /// (first, count) per run. Clears out beforehand. Returns nothing for an
    /// empty pool; trailing freed slots are excluded.
    pub(crate) fn used_runs(&self, out: &mut Vec<(u32, u32)>) {
        out.clear();
        let mut i = 0;
        while i < self.in_use.len() {
            if self.in_use[i] {
                let first = i as u32;
                while i < self.in_use.len() && self.in_use[i] { i += 1; }
                let count = (i - first as usize) as u32;
                out.push((first, count));
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_used_runs_all_allocated() {
        let mut pool = InstancePool::new(10);
        let _slot0 = pool.alloc();
        let _slot1 = pool.alloc();
        let _slot2 = pool.alloc();
        let _slot3 = pool.alloc();
        let _slot4 = pool.alloc();

        let mut runs = Vec::new();
        pool.used_runs(&mut runs);

        assert_eq!(runs, vec![(0, 5)], "5 allocated slots should give one contiguous run");
    }

    #[test]
    fn test_used_runs_with_interior_frees() {
        let mut pool = InstancePool::new(10);
        pool.alloc(); // 0
        pool.alloc(); // 1
        pool.alloc(); // 2
        pool.alloc(); // 3
        pool.alloc(); // 4

        pool.free(1);
        pool.free(3);

        let mut runs = Vec::new();
        pool.used_runs(&mut runs);

        assert_eq!(
            runs,
            vec![(0, 1), (2, 1), (4, 1)],
            "freeing indices 1 and 3 should give three separate runs"
        );
    }

    #[test]
    fn test_used_runs_with_trailing_free() {
        let mut pool = InstancePool::new(10);
        pool.alloc(); // 0
        pool.alloc(); // 1
        pool.alloc(); // 2
        pool.alloc(); // 3
        pool.alloc(); // 4

        pool.free(1);
        pool.free(3);
        pool.free(4);

        let mut runs = Vec::new();
        pool.used_runs(&mut runs);

        assert_eq!(
            runs,
            vec![(0, 1), (2, 1)],
            "trailing free should be excluded from runs"
        );
    }

    #[test]
    fn test_used_runs_realloc_maintains_invariant() {
        let mut pool = InstancePool::new(10);
        pool.alloc(); // 0
        pool.alloc(); // 1
        pool.alloc(); // 2

        pool.free(1);

        let mut runs = Vec::new();
        pool.used_runs(&mut runs);
        let sum_before: usize = runs.iter().map(|(_, count)| *count as usize).sum();
        assert_eq!(sum_before, pool.used());

        let _realloc = pool.alloc(); // should reuse index 1

        runs.clear();
        pool.used_runs(&mut runs);
        let sum_after: usize = runs.iter().map(|(_, count)| *count as usize).sum();
        assert_eq!(sum_after, pool.used(), "invariant: sum(count) == pool.used()");
    }

    #[test]
    fn test_used_runs_empty() {
        let pool = InstancePool::new(10);
        let mut runs = Vec::new();
        pool.used_runs(&mut runs);

        assert_eq!(runs, vec![], "empty pool should give no runs");
    }
}
