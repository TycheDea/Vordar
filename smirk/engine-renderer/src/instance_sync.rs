//! Snapshots each entity's previous Transform for render-frame interpolation,
//! and keeps the SDF InstancePool in sync with the world — dirty-slot writes,
//! slot attach/free.

use engine_core::traits::Resources;
use engine_core::World;
use engine_app::scheduler::System;
use engine_core::traits::DespawnQueue;
use engine_core::components::{PreviousTransform, RenderShape, RenderShapeType, ShapeGroup, Transform};
use glam::Mat4;
use crate::facade::{alloc_render_slot, alloc_shape_group_slots};
use crate::instance::{InstancePool, InstanceSlot, ShapeGroupSlots, SdfInstance};
use engine_app::scheduler::InterpolationAlpha;

/// Saves each entity's current position into PreviousTransform at the start of
/// every fixed step. Register in Phase::Update, SystemOrder::First so it runs
/// before any movement system mutates Transform.
pub(crate) struct SaveTransformSystem;

impl System for SaveTransformSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, _delta: f32) {
        for (transform, prev) in world.query::<(&Transform, &mut PreviousTransform)>().iter() {
            prev.position = transform.position;
        }
    }
}

pub struct RenderSyncSystem;

impl System for RenderSyncSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let alpha = resources.get::<InterpolationAlpha>().map(|a| a.0).unwrap_or(1.0);
        let pool = resources.get_mut::<InstancePool>()
            .expect("InstancePool not in resources");

        for (transform, prev, render_shape, slot) in
            world.query::<(&Transform, Option<&PreviousTransform>, &RenderShape, &InstanceSlot)>().iter()
        {
            // Lerp position if PreviousTransform is present; otherwise use current.
            let render_pos = match prev {
                Some(p) => p.position.lerp(transform.position, alpha),
                None    => transform.position,
            };
            let render_transform = Transform {
                position: render_pos,
                rotation: transform.rotation,
                scale:    transform.scale,
            };
            let (shape_type, shape_params) = shape_to_gpu(render_shape.shape);
            let new_inst = SdfInstance {
                model:       render_transform.to_model_matrix().to_cols_array_2d(),
                color:       render_shape.color.to_array(),
                shape_type,
                shape_params,
            };
            if bytemuck::bytes_of(&pool.slots[slot.0]) != bytemuck::bytes_of(&new_inst) {
                pool.slots[slot.0] = new_inst;
                pool.dirty[slot.0] = true;
            }
        }

        for (transform, prev, group, slots) in
            world.query::<(&Transform, Option<&PreviousTransform>, &ShapeGroup, &ShapeGroupSlots)>().iter()
        {
            let render_pos = match prev {
                Some(p) => p.position.lerp(transform.position, alpha),
                None    => transform.position,
            };
            let parent_model = Transform {
                position: render_pos,
                rotation: transform.rotation,
                scale:    transform.scale,
            }.to_model_matrix();

            for (sub, key) in group.shapes.iter().zip(slots.0.iter()) {
                let sub_model = parent_model
                    * Mat4::from_scale_rotation_translation(sub.scale, sub.rotation, sub.offset);
                let (shape_type, shape_params) = shape_to_gpu(sub.shape);
                let new_inst = SdfInstance {
                    model:       sub_model.to_cols_array_2d(),
                    color:       sub.color.to_array(),
                    shape_type,
                    shape_params,
                };
                if bytemuck::bytes_of(&pool.slots[*key]) != bytemuck::bytes_of(&new_inst) {
                    pool.slots[*key] = new_inst;
                    pool.dirty[*key] = true;
                }
            }
        }
    }
}

/// Frees render slots for entities queued for despawn. Registered by
/// RenderPlugin in Phase::DespawnFlush, First — must run before
/// DespawnFlushSystem.
pub(crate) struct RenderSlotDespawnSystem;

impl System for RenderSlotDespawnSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Collect entities without holding the DespawnQueue borrow into the loop.
        let entities: Vec<_> = resources
            .get::<DespawnQueue>()
            .map(|q| q.0.iter().map(|(e, _)| *e).collect())
            .unwrap_or_default();

        let pool = resources.get_mut::<InstancePool>()
            .expect("InstancePool not in resources");

        for entity in entities {
            if let Ok(slot) = world.get::<&InstanceSlot>(entity) {
                pool.free(slot.0);
            }
            if let Ok(slots) = world.get::<&ShapeGroupSlots>(entity) {
                for &key in &slots.0 { pool.free(key); }
            }
        }
    }
}

/// Allocates GPU instance slots for entities that have a RenderShape/ShapeGroup
/// but no slot yet — entities spawned from data (prefabs) need no renderer access
/// at spawn time. Runs in Phase::RenderSync, First, so freshly flushed entities
/// are visible the same frame. Steady-state cost is ~zero: the matched archetype
/// set is empty once every renderable entity holds its slot.
pub(crate) struct RenderSlotAttachSystem;

impl System for RenderSlotAttachSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Collect first — the query borrow must end before insert_one mutates the world.
        let singles: Vec<hecs::Entity> = world
            .query::<(hecs::Entity, &RenderShape)>().without::<&InstanceSlot>()
            .iter().map(|(e, _)| e).collect();
        let groups: Vec<(hecs::Entity, usize)> = world
            .query::<(hecs::Entity, &ShapeGroup)>().without::<&ShapeGroupSlots>()
            .iter().map(|(e, g)| (e, g.shapes.len())).collect();

        for entity in singles {
            let slot = alloc_render_slot(resources);
            let _ = world.insert_one(entity, slot);
        }
        for (entity, count) in groups {
            let slots = alloc_shape_group_slots(count, resources);
            let _ = world.insert_one(entity, slots);
        }
    }
}

fn shape_to_gpu(shape: RenderShapeType) -> (u32, [f32; 4]) {
    match shape {
        RenderShapeType::Cube                         => (0, [0.0; 4]),
        RenderShapeType::Sphere                       => (1, [0.0; 4]),
        RenderShapeType::Diamond                      => (2, [0.0; 4]),
        RenderShapeType::RoundedBox { corner_radius } => (3, [corner_radius, 0.0, 0.0, 0.0]),
        RenderShapeType::Cylinder                     => (4, [0.0; 4]),
        RenderShapeType::Capsule                      => (5, [0.0; 4]),
        RenderShapeType::Custom { shape_type, params } => (shape_type, params),
    }
}
