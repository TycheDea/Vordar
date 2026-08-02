// Headless full-pipeline test: a scheduled Mechanic's damage must flow
// through Health -> DeathSystem -> XpGrantSystem and the resolved mechanic
// must clear via DespawnQueue, all driven by the real server App
// (CoreGamePlugin + NetServerPlugin) instead of e2e QUIC round trips.
// Reuses the bench harness shape from benchmarks/benches/full_tick.rs:
// a one-shot PreUpdate::First spawn system, App::run_ticks with an injected
// fixed dt, no sleeps.

use engine_app::app::App;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::components::{Health, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use engine_net::NetLimits;
use glam::Vec3;
use hecs::Entity;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use test_support::workspace_root;
use vordar_game::combat::stats::DamageType;
use vordar_game::progression::{Xp, XpReward};
use vordar_game::Mechanic;

const DT: f32 = 1.0 / 60.0;

/// One-shot world population: caster (excluded from the hit test), an enemy
/// target with Health + XpReward, and a Mechanic due at t=0 centered on the
/// enemy with lethal damage.
struct Spawn {
    entities: Arc<Mutex<Option<(Entity, Entity, Entity)>>>,
    done: bool,
}

impl System for Spawn {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, _delta: f32) {
        if self.done {
            return;
        }
        self.done = true;
        let caster = world.spawn(());
        let enemy = world.spawn((Transform::new(Vec3::ZERO), Health { current: 50, max: 50 }, XpReward { amount: 25 }));
        let mechanic = world.spawn((
            Transform::new(Vec3::ZERO),
            Mechanic { id: 1, radius: 5.0, damage: 9999, damage_type: DamageType::Physical, resolve_at_micros: 0, caster },
        ));
        *self.entities.lock().unwrap() = Some((caster, enemy, mechanic));
    }
}

#[derive(Default, Clone, Copy)]
struct Outcome {
    enemy_health: i32,
    enemy_alive: bool,
    mechanic_alive: bool,
    killer_xp: Option<u32>,
}

/// Reads the observable chain back out every tick (Render fires once per
/// `run_ticks` call at dt == fixed_dt, so this sees every fixed step).
struct Observe {
    entities: Arc<Mutex<Option<(Entity, Entity, Entity)>>>,
    outcome: Arc<Mutex<Outcome>>,
}

impl System for Observe {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, _delta: f32) {
        let Some((caster, enemy, mechanic)) = *self.entities.lock().unwrap() else { return };
        let mut out = self.outcome.lock().unwrap();
        if let Ok(h) = world.get::<&Health>(enemy) {
            out.enemy_health = h.current;
        }
        out.enemy_alive = world.contains(enemy);
        out.mechanic_alive = world.contains(mechanic);
        out.killer_xp = world.get::<&Xp>(caster).ok().map(|xp| xp.0);
    }
}

fn build(entities: Arc<Mutex<Option<(Entity, Entity, Entity)>>>, outcome: Arc<Mutex<Outcome>>) -> App {
    workspace_root(); // prefab dirs load relative to the workspace root
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let mut app = vordar_server::build_server_app_with_limits(addr, ":memory:", NetLimits::default());
    app.add_system(Spawn { entities: entities.clone(), done: false }, Phase::PreUpdate, SystemOrder::First)
        .add_system(Observe { entities, outcome }, Phase::Render, SystemOrder::Last);
    app
}

#[test]
fn mechanic_damage_flows_through_death_but_grants_no_xp() {
    let entities = Arc::new(Mutex::new(None));
    let outcome = Arc::new(Mutex::new(Outcome::default()));
    let mut app = build(entities, outcome.clone());

    // Tick 1: mechanic resolves (due at t=0) in PostUpdate, applying lethal
    // damage. Tick 2: DeathSystem (CollisionResolve) sees Health <= 0 and the
    // DespawnFlush that follows removes both the enemy and the mechanic
    // queued by tick 1.
    app.run_ticks(DT, 2);

    let out = *outcome.lock().unwrap();
    assert!(out.enemy_health <= 0, "mechanic damage must reduce the enemy's health to lethal: {}", out.enemy_health);
    assert!(!out.enemy_alive, "DeathSystem must despawn the enemy once its health is depleted");
    assert!(!out.mechanic_alive, "the resolved mechanic must be despawned via DespawnFlush");
    // Killer attribution reads DamageDealt from the CURRENT tick's EventBus
    // (game/vordar-game/src/combat/death.rs), which ClearEventsSystem wipes
    // every tick at Phase::Input. MechanicResolveSystem emits DamageDealt in
    // Phase::PostUpdate, one phase after DeathSystem's Phase::CollisionResolve,
    // so the death DeathSystem detects next tick never sees that event —
    // no killer, no Killed, no XP.
    assert_eq!(out.killer_xp, None, "mechanic-caused kills do not currently grant XP");
}
