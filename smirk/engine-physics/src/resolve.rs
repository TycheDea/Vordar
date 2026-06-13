// CollisionResolveSystem — reads CollisionStarted events each frame.
//
// This is a stub: the actual damage/pushback logic lives in the game crate,
// which subscribes to CollisionStarted/CollisionEnded via EventBus.
// The engine registers this system to hold the Phase::CollisionResolve slot.

use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;

pub struct CollisionResolveSystem;

impl System for CollisionResolveSystem {
    fn run(&mut self, _world: &mut World, _resources: &mut Resources, _delta: f32) {
        // Game systems run after this in Phase::CollisionResolve.
    }
}
