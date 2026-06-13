// MovementSystem — integrates Velocity into Transform each fixed tick.

use engine_app::scheduler::System;
use engine_core::components::{Transform, Velocity};
use engine_core::traits::Resources;
use engine_core::World;

pub struct MovementSystem;

impl System for MovementSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, delta: f32) {
        for (transform, velocity) in world.query::<(&mut Transform, &Velocity)>().iter() {
            transform.position += velocity.linear * delta;
        }
    }
}
