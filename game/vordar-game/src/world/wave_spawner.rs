// Chapter-driven spawning — replaces the old hard-coded SetupSystem/EnemySpawnerSystem.
//
// Spawn requests go directly through SpawnQueue (via queue_prefab_spawn), NOT
// an EventBus hop: events are cleared only in Phase::Input, so a same-phase
// event reader would re-read step-1 events during fixed-rate catch-up steps
// and double-spawn.

use super::chapter::ActiveChapter;
use crate::enemies::Enemy;
use crate::player::Player;
use engine_app::scheduler::System;
use engine_core::components::Transform;
use engine_core::prefab::queue_prefab_spawn;
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;

/// Spawns the chapter's initial entities on the first run. Players are NOT
/// chapter content — whoever has authority spawns them (the sandbox locally,
/// the server per connection).
pub struct ChapterSetupSystem {
    warned: bool,
}

impl ChapterSetupSystem {
    pub fn new() -> Self { Self { warned: false } }
}

impl System for ChapterSetupSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let initial = {
            let Some(chapter) = resources.get_mut::<ActiveChapter>() else {
                if !self.warned {
                    log::warn!("no ActiveChapter resource — nothing will spawn (add a chapter plugin)");
                    self.warned = true;
                }
                return;
            };
            if chapter.started { return; }
            chapter.started = true;

            // Clone the spawn list out so the resources borrow ends before queueing.
            let mut initial: Vec<(String, Vec3)> = Vec::new();
            for spawn in &chapter.def.initial_spawns {
                for &pos in &spawn.positions {
                    initial.push((spawn.prefab.clone(), pos));
                }
            }
            initial
        };

        for (prefab, pos) in initial {
            queue_prefab_spawn(resources, prefab, pos);
        }
    }
}

/// Ticks chapter time and wave timers; spawns wave prefabs on a rotating ring
/// around the player. Wave timers freeze while max_alive enemies exist.
pub struct WaveSpawnerSystem;

impl System for WaveSpawnerSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let enemy_count = world.query::<(&Enemy,)>().iter().count();
        let player_pos: Vec3 = world
            .query::<(&Transform, &Player)>()
            .iter()
            .next()
            .map(|(t, _)| t.position)
            .unwrap_or(Vec3::ZERO);

        let spawns = {
            let Some(chapter) = resources.get_mut::<ActiveChapter>() else { return; };
            if !chapter.started { return; }
            chapter.elapsed += delta;
            if enemy_count >= chapter.def.spawning.max_alive { return; }

            let mut spawns: Vec<(String, Vec3)> = Vec::new();
            for (i, wave) in chapter.def.spawning.waves.iter().enumerate() {
                if chapter.elapsed < wave.start_time { continue; }
                chapter.wave_timers[i] += delta;
                if chapter.wave_timers[i] < wave.interval { continue; }
                chapter.wave_timers[i] = 0.0;

                for _ in 0..wave.count_per_spawn {
                    let angle = chapter.spawn_angle;
                    chapter.spawn_angle += std::f32::consts::TAU / 8.0;
                    let pos = player_pos
                        + Vec3::new(angle.cos() * wave.spawn_radius, 0.0, angle.sin() * wave.spawn_radius);
                    spawns.push((wave.prefab.clone(), pos));
                }
            }
            spawns
        };

        for (prefab, pos) in spawns {
            queue_prefab_spawn(resources, prefab, pos);
        }
    }
}
