// Motion — shared movement mechanics: velocity integration and the solid-
// overlap separation response. Entity-agnostic by design.

pub mod movement;
pub mod separation;

use engine_core::components::CollisionShape;
use glam::Vec3;

pub use movement::{step, MovementSystem, PlayRadius};
pub use separation::{anchored_push, SeparationSystem};

/// The prediction half of the shared movement + static-collision rule:
/// integrate, then resolve against anchored statics, so the client's
/// reconciliation replay lands on the exact position the live
/// Movement -> Collision -> CollisionResolve pipeline would (DESIGN.md §6
/// determinism, same contract as `player::movement_velocity`).
pub fn predict_step(
    pos: Vec3,
    velocity: Vec3,
    dt: f32,
    bound: f32,
    shape: &CollisionShape,
    statics: &[(Vec3, CollisionShape)],
) -> Vec3 {
    let integrated = step(pos, velocity, dt, bound);
    integrated + anchored_push(integrated, shape, statics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_app::events::EventBus;
    use engine_app::scheduler::System;
    use engine_core::components::{Anchored, CellOccupant, Hitbox, Solid, Transform, Velocity};
    use engine_core::spatial::SpatialGrid;
    use engine_core::traits::Resources;
    use engine_core::World;
    use engine_physics::broadphase::{BroadphaseSystem, CandidatePairs};
    use engine_physics::cell_update::CellUpdateSystem;
    use engine_physics::narrowphase::{ActivePairs, NarrowphaseSystem};
    use hecs::Entity;

    const DT: f32 = 1.0 / 60.0;
    const TICKS: usize = 60;

    fn walker_shape() -> CollisionShape {
        CollisionShape::Aabb { half_extents: Vec3::splat(0.5) } // the player prefab's shape (content/prefabs/human.ron)
    }

    fn wall_shape() -> CollisionShape {
        CollisionShape::Aabb { half_extents: Vec3::new(1.6, 0.9, 1.3) } // the cottage's shape (content/chapters/chapter02/prefabs/cottage.ron)
    }

    fn spawn_walker(world: &mut World) -> Entity {
        world.spawn((
            Transform::new(Vec3::ZERO),
            Velocity { linear: Vec3::new(6.0, 0.0, 0.0) },
            Hitbox { shape: walker_shape() },
            CellOccupant { cells: Default::default() },
            Solid,
        ))
    }

    fn spawn_anchored_wall(world: &mut World, pos: Vec3) -> Entity {
        let wall = world.spawn((
            Transform::new(pos),
            Hitbox { shape: wall_shape() },
            CellOccupant { cells: Default::default() },
            Solid,
        ));
        world.insert_one(wall, Anchored).unwrap();
        wall
    }

    fn base_resources() -> Resources {
        let mut resources = Resources::new();
        resources.insert(SpatialGrid::new(10.0));
        resources.insert(CandidatePairs::new());
        resources.insert(ActivePairs::new());
        resources.insert(EventBus::new());
        resources.insert(PlayRadius::default());
        resources
    }

    /// Runs the real Movement -> Collision -> CollisionResolve pipeline, in
    /// registration order (plugin.rs / engine-physics lib.rs), for `TICKS`
    /// steps, folding `predict_step` from the same start alongside it; each
    /// tick, `check` compares the live walker position against the fold.
    fn drive_and_compare(
        world: &mut World,
        resources: &mut Resources,
        walker: Entity,
        statics: &[(Vec3, CollisionShape)],
        check: impl Fn(usize, Vec3, Vec3),
    ) {
        let mut cell_update = CellUpdateSystem::new();
        let mut broadphase = BroadphaseSystem::new();
        let mut narrowphase = NarrowphaseSystem::new();
        let velocity = Vec3::new(6.0, 0.0, 0.0);
        let mut predicted = Vec3::ZERO;

        for tick in 0..TICKS {
            MovementSystem.run(&mut *world, &mut *resources, DT);
            cell_update.run(&mut *world, &mut *resources, DT);
            broadphase.run(&mut *world, &mut *resources, DT);
            narrowphase.run(&mut *world, &mut *resources, DT);
            SeparationSystem.run(&mut *world, &mut *resources, DT);

            predicted = predict_step(predicted, velocity, DT, PlayRadius::default().0, &walker_shape(), statics);

            let live = world.get::<&Transform>(walker).unwrap().position;
            check(tick, live, predicted);
        }

        // The scenario is real, not vacuous: the walker actually reaches the
        // wall (contact surface near x = 3.0 - 1.6 - 0.5) and stops there,
        // rather than tunneling through it.
        let final_pos = world.get::<&Transform>(walker).unwrap().position;
        assert!(final_pos.x > 0.5, "walker should have moved toward the wall, got {final_pos:?}");
        assert!(final_pos.x < 1.4, "walker should be pressed against the wall, not through it, got {final_pos:?}");
    }

    // Single contact has one float path: the live pipeline's accumulated
    // correction and predict_step's anchored_push apply the same mtv() call
    // to the same positions, so the two must match bit for bit.
    #[test]
    fn single_anchored_contact_matches_predict_step_exactly() {
        let mut world = World::new();
        let mut resources = base_resources();
        let walker = spawn_walker(&mut world);
        let wall_pos = Vec3::new(3.0, 0.0, 0.0);
        spawn_anchored_wall(&mut world, wall_pos);

        drive_and_compare(&mut world, &mut resources, walker, &[(wall_pos, wall_shape())], |tick, live, predicted| {
            assert_eq!(live, predicted, "tick {tick}: live and predicted positions diverged");
        });
    }

    // Two anchored walls overlapping each other (and both contacting the
    // walker) put two pairs in ActivePairs at once. The live pipeline sums
    // their corrections in ActivePairs' HashSet iteration order (run-varying,
    // per narrowphase.rs's own sort-for-determinism comment on emitted
    // events — the correction sum itself isn't sorted); predict_step's fold
    // always sums `statics` in slice order. f32 addition isn't associative,
    // so the two orders can differ in the last bit or two.
    #[test]
    fn two_overlapping_anchored_walls_match_predict_step_within_tolerance() {
        let mut world = World::new();
        let mut resources = base_resources();
        let walker = spawn_walker(&mut world);
        let wall_a_pos = Vec3::new(3.0, 0.0, 0.5);
        let wall_b_pos = Vec3::new(3.0, 0.0, -0.5);
        spawn_anchored_wall(&mut world, wall_a_pos);
        spawn_anchored_wall(&mut world, wall_b_pos);

        drive_and_compare(
            &mut world,
            &mut resources,
            walker,
            &[(wall_a_pos, wall_shape()), (wall_b_pos, wall_shape())],
            |tick, live, predicted| {
                assert!(
                    live.abs_diff_eq(predicted, 1e-5),
                    "tick {tick}: live {live:?} vs predicted {predicted:?}"
                );
            },
        );
    }
}
