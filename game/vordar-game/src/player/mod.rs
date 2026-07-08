// The player — its component, its movement rule, and its skill book.
// Everything the player IS lives in this module; shared mechanics it uses
// (motion integration, projectiles, health/death) stay generic systems.

pub mod class;
pub mod skills;

use crate::events::MoveIntent;
use engine_app::events::EventBus;
use engine_app::scheduler::System;
use engine_core::components::Velocity;
use engine_core::traits::Resources;
use engine_core::World;
use glam::{Vec2, Vec3};
use hecs::Entity;
use std::collections::HashMap;

/// Marker + speed for the player entity. Registered for RON spawning.
#[derive(Clone, serde::Deserialize)]
pub struct Player {
    pub speed: f32,
}

/// THE player movement rule, as a pure function: a movement intent becomes
/// this velocity, integrated over the tick's dt. Shared by the live system
/// (server and sandbox) and the client's prediction replay — both sides must
/// compute the exact same step or prediction drifts (DESIGN.md §6 determinism).
pub fn movement_velocity(dir: Vec2, speed: f32) -> Vec3 {
    let dir = if dir.length_squared() > 0.0 { dir.normalize() } else { Vec2::ZERO };
    Vec3::new(dir.x, 0.0, dir.y) * speed
}

/// Converts MoveIntent events into player velocity.
///
/// Reads intents from the EventBus only; it knows nothing about keyboards,
/// cameras, or the network. The client's input plugin (locally) or the server's
/// network layer (per connection) is responsible for emitting MoveIntent each
/// Input tick. Players without an intent this tick stand still.
pub struct PlayerMovementSystem;

impl System for PlayerMovementSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let intents: HashMap<Entity, Vec2> = {
            let bus = resources.get::<EventBus>().expect("EventBus not in resources");
            bus.read::<MoveIntent>().map(|i| (i.entity, i.dir)).collect()
        };

        for (entity, velocity, player) in world.query::<(Entity, &mut Velocity, &Player)>().iter() {
            let dir = intents.get(&entity).copied().unwrap_or(Vec2::ZERO);
            velocity.linear = movement_velocity(dir, player.speed);
        }
    }
}
