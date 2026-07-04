// Whole-sim-tick macro benchmark: the server's plugin composition minus
// networking (PhysicsPlugin + PrefabPlugin + CoreGamePlugin, per
// vordar-server/src/lib.rs), driven back-to-back through App::run_ticks.
// Context for the micro benches — ns/tick against the 16.67 ms budget.
//
// Population is stationary by construction: passive enemies (aggro 0) still
// pay the full O(E·P) AI scan but never move; players have no intents; nothing
// has ContactDamage and enemies carry no Health, so no combat, no deaths.

use criterion::{criterion_group, criterion_main, Criterion};
use engine_app::app::App;
use engine_app::prefab_plugin::PrefabPlugin;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::components::{CellOccupant, CollisionShape, Hitbox, Solid, Transform, Velocity};
use engine_core::traits::Resources;
use engine_core::World;
use engine_physics::PhysicsPlugin;
use glam::Vec3;
use std::time::Instant;
use vordar_benches::{positions, uniform_half, workspace_root, Layout, DT};
use vordar_game::player::Player;
use vordar_game::{AttackKind, CoreGamePlugin, Enemy};

/// One-shot world population, run inside the schedule (App::world is private).
struct Populate {
    enemies: usize,
    players: usize,
    done: bool,
}

impl System for Populate {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, _delta: f32) {
        if self.done {
            return;
        }
        self.done = true;
        let half = uniform_half(self.enemies + self.players);
        for pos in positions(self.enemies, Layout::Uniform { half_extent: half }, 7) {
            world.spawn((
                Transform::new(pos),
                Velocity { linear: Vec3::ZERO },
                Enemy { speed: 2.0, aggro_range: 0.0, attack: AttackKind::Melee, cooldown_left: 0.0 },
                Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
                CellOccupant { cells: Default::default() },
                Solid,
            ));
        }
        for pos in positions(self.players, Layout::Uniform { half_extent: half }, 13) {
            world.spawn((
                Transform::new(pos),
                Velocity { linear: Vec3::ZERO },
                Player { speed: 6.0 },
                Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
                CellOccupant { cells: Default::default() },
                Solid,
            ));
        }
    }
}

fn bench_full_tick(c: &mut Criterion) {
    workspace_root(); // prefab dirs load relative to the workspace root
    let mut group = c.benchmark_group("full_tick");
    group.sample_size(20);
    for (enemies, players) in [(200usize, 50usize), (1000, 200)] {
        let mut app = App::new();
        app.add_plugin(PhysicsPlugin)
            .add_plugin(PrefabPlugin)
            .add_plugin(CoreGamePlugin)
            .add_system(Populate { enemies, players, done: false }, Phase::PreUpdate, SystemOrder::First);

        group.bench_function(format!("e{enemies}_p{players}"), |b| {
            b.iter_custom(|iters| {
                let t = Instant::now();
                app.run_ticks(DT, iters);
                t.elapsed()
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_full_tick);
criterion_main!(benches);
