// The world — its clock and timed events (below), plus how it is laid out
// and populated: zones/portals, chapters, camps (submodules).
//
// World clock + timed world events (DESIGN.md §4): one authoritative world
// time; events are deterministic shared definitions evaluated against it. An
// event fires one-shot `spawns` once on window entry and can also drive
// recurring capped `waves` for the whole window (pulse timing is pure clock
// math). Synchronization is trivial by construction: every process that knows
// the clock and loads the same defs agrees on what is active — the server
// spawns, clients tint, nobody exchanges event state.

pub mod camp;
pub mod chapter;
mod chapter_registry;
pub mod setup;
pub mod zones;

use engine_app::scheduler::System;
use engine_core::prefab::queue_prefab_spawn;
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;

/// Current world time in microseconds — published every tick by whoever owns
/// the authoritative clock (online: the server's net plugin). World systems
/// no-op when absent (offline sandbox).
pub struct WorldTime(pub u64);

#[derive(serde::Deserialize)]
pub struct WorldEventsDef {
    /// Length of one world day in seconds — the day/night cycle period.
    pub day_seconds: f64,
    pub events: Vec<WorldEventDef>,
}

#[derive(serde::Deserialize)]
pub struct WorldEventDef {
    pub name: String,
    /// Seconds into each world day at which the event starts (recurs daily).
    pub start_seconds_of_day: f64,
    pub duration_seconds: f64,
    /// Light tint while active — applied render-side, never sent on the wire.
    pub ambient: Vec3,
    #[serde(default)]
    pub spawns: Vec<WorldSpawn>,
    /// Recurring pressure spawns fired every `interval_seconds` for the window.
    #[serde(default)]
    pub waves: Vec<EventWaveDef>,
}

#[derive(serde::Deserialize)]
pub struct WorldSpawn {
    pub prefab: String,
    pub positions: Vec<Vec3>,
}

#[derive(serde::Deserialize)]
pub struct EventWaveDef {
    pub prefab: String,
    pub positions: Vec<Vec3>,
    pub interval_seconds: f64,
    /// Zone-global budget for this wave: no pulse spawns past this many live
    /// `EventSpawned` entities tagged to it.
    pub max_alive: usize,
}

/// Marks an entity spawned by a world event's wave. Alive-counting and
/// window-end cleanup query this component rather than caching Entity ids, so
/// a slot reads as free the instant its entity is gone, regardless of despawn
/// order elsewhere in the frame.
pub struct EventSpawned {
    pub event: u16,
    pub wave: u16,
}

/// Load world event definitions. Panics on failure — broken content is an
/// authoring bug the author must see immediately (same policy as chapters).
pub fn load_world_events(path: &str) -> WorldEventsDef {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("world events '{path}' unreadable: {e}"));
    let def: WorldEventsDef = ron::from_str(&text)
        .unwrap_or_else(|e| panic!("world events '{path}' parse error: {e}"));
    log::info!("world events loaded: {} events, {} s day", def.events.len(), def.day_seconds);
    def
}

/// Index of the event active at `world_seconds`, if any. Pure function —
/// server spawning and client tinting agree by construction.
pub fn active_event(def: &WorldEventsDef, world_seconds: f64) -> Option<usize> {
    let day_time = world_seconds.rem_euclid(def.day_seconds);
    def.events.iter().position(|e| {
        day_time >= e.start_seconds_of_day
            && day_time < e.start_seconds_of_day + e.duration_seconds
    })
}

/// Sun direction, light color, and ambient strength as a pure function of the
/// day fraction (0 = midnight, 0.5 = noon). Render-side consumers feed this
/// straight into the light uniform; deterministic across clients.
pub fn day_night_light(day_fraction: f32) -> (Vec3, Vec3, f32) {
    let angle = day_fraction * std::f32::consts::TAU;
    // -1 at midnight, +1 at noon.
    let elevation = -angle.cos();
    let daylight = elevation.clamp(0.0, 1.0);

    // The sun sweeps east→west; clamp above the horizon so night keeps a dim
    // moon-ish key light instead of lighting from below.
    let dir = Vec3::new(angle.sin(), elevation.max(0.15), 0.4).normalize();
    let night = Vec3::new(0.25, 0.3, 0.55);
    let day = Vec3::new(1.0, 0.95, 0.85);
    let color = night.lerp(day, daylight);
    // Ambient scales the IBL environment (1.0 = as authored): full by day,
    // dimmed — never dead — at night.
    let ambient = 0.25 + 0.75 * daylight;
    (dir, color, ambient)
}

/// Fires each event's one-shot `spawns` once per world day on window entry,
/// and drives its `waves` — recurring capped pulses — for the whole window,
/// reaping the wave spawns when the window closes. Mid-window joiners need
/// nothing special: spawned entities replicate via AOI, and tint is a pure
/// function of the clock.
pub struct WorldEventSystem {
    /// Per-event index of the last world day it fired its one-shot spawns on.
    fired: Vec<i64>,
    /// Per event, per wave: (world day, pulses accounted for that day). The
    /// count resets when the day advances; forfeited over-cap pulses are not
    /// retried (no burst catch-up).
    pulses: Vec<Vec<(i64, u64)>>,
}

impl WorldEventSystem {
    pub fn new() -> Self {
        Self { fired: Vec::new(), pulses: Vec::new() }
    }
}

impl System for WorldEventSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        use engine_core::traits::{DespawnQueue, SpawnQueue};
        use engine_core::prefab::spawn_prefab;
        use hecs::Entity;

        // One-shot spawns (prefab, position), wave spawns tagged with
        // (event, wave), and event-wave entities to reap at window close.
        let (one_shot, waves, to_despawn): (Vec<(String, Vec3)>, Vec<(String, Vec3, u16, u16)>, Vec<Entity>) = {
            let Some(now) = resources.get::<WorldTime>().map(|t| t.0) else { return };
            let Some(def) = resources.get::<WorldEventsDef>() else { return };
            if self.fired.len() != def.events.len() {
                self.fired = vec![i64::MIN; def.events.len()];
                self.pulses = def
                    .events
                    .iter()
                    .map(|e| vec![(i64::MIN, 0u64); e.waves.len()])
                    .collect();
            }

            let world_seconds = now as f64 * 1e-6;
            let day = (world_seconds / def.day_seconds).floor() as i64;
            let day_time = world_seconds - day as f64 * def.day_seconds;

            // Live wave spawns, counted per (event, wave) and grouped per event
            // — the running budget and the window-end reap list, both read from
            // the world so a just-killed entity frees its slot immediately.
            let mut alive: std::collections::HashMap<(u16, u16), usize> = std::collections::HashMap::new();
            let mut by_event: std::collections::HashMap<u16, Vec<Entity>> = std::collections::HashMap::new();
            for (ent, m) in world.query::<(Entity, &EventSpawned)>().iter() {
                *alive.entry((m.event, m.wave)).or_default() += 1;
                by_event.entry(m.event).or_default().push(ent);
            }

            let mut one_shot = Vec::new();
            let mut waves = Vec::new();
            let mut to_despawn = Vec::new();
            for (i, event) in def.events.iter().enumerate() {
                let ev = i as u16;
                if self.pulses[i].len() != event.waves.len() {
                    self.pulses[i] = vec![(i64::MIN, 0u64); event.waves.len()];
                }
                let in_window = day_time >= event.start_seconds_of_day
                    && day_time < event.start_seconds_of_day + event.duration_seconds;

                if !in_window {
                    // Window closed: reap this event's wave spawns.
                    if let Some(ents) = by_event.get(&ev) {
                        to_despawn.extend(ents.iter().copied());
                    }
                    continue;
                }

                if self.fired[i] != day {
                    self.fired[i] = day;
                    log::info!("world event '{}' started (day {day})", event.name);
                    for spawn in &event.spawns {
                        for &pos in &spawn.positions {
                            one_shot.push((spawn.prefab.clone(), pos));
                        }
                    }
                }

                for (wi, wave) in event.waves.iter().enumerate() {
                    let w = wi as u16;
                    let (stored_day, stored_count) = self.pulses[i][wi];
                    let fired_count = if stored_day == day { stored_count } else { 0 };
                    // Pulse k fires at start + k·interval (k ≥ 1); window entry
                    // is covered by the one-shot spawns above.
                    let due = ((day_time - event.start_seconds_of_day) / wave.interval_seconds).floor() as u64;
                    for _ in fired_count..due {
                        for &pos in &wave.positions {
                            let count = alive.entry((ev, w)).or_default();
                            if *count < wave.max_alive {
                                waves.push((wave.prefab.clone(), pos, ev, w));
                                *count += 1;
                            }
                        }
                    }
                    // Forfeit over-cap pulses unconditionally — no catch-up.
                    self.pulses[i][wi] = (day, due);
                }
            }
            (one_shot, waves, to_despawn)
        };

        for (prefab, pos) in one_shot {
            queue_prefab_spawn(resources, prefab, pos);
        }
        for (prefab, pos, event, wave) in waves {
            resources.get_mut::<SpawnQueue>().unwrap().push(move |ctx| {
                match spawn_prefab(&prefab, pos, ctx) {
                    Ok(entity) => { let _ = ctx.world.insert_one(entity, EventSpawned { event, wave }); }
                    Err(e) => log::error!("wave spawn '{prefab}' failed: {e}"),
                }
            });
        }
        for entity in to_despawn {
            resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::prefab::{register_core_components, ComponentRegistry, PrefabDef, PrefabLibrary};
    use engine_core::traits::{DespawnQueue, SpawnContext, SpawnQueue};

    /// Resources mirroring camp.rs's fixture: a Transform-only `"dummy"` prefab
    /// plus the spawn/despawn queues the system drains through.
    fn wave_resources(def: WorldEventsDef) -> Resources {
        let mut registry = ComponentRegistry::new();
        register_core_components(&mut registry);
        let mut library = PrefabLibrary::new();
        library.insert(
            "dummy",
            ron::from_str::<PrefabDef>(r#"(components: { "Transform": () })"#).unwrap(),
        );
        let mut resources = Resources::new();
        resources.insert(registry);
        resources.insert(library);
        resources.insert(def);
        resources.insert(SpawnQueue::new());
        resources.insert(DespawnQueue::new());
        resources
    }

    /// Sets the world clock, runs the system, then drains both queues the way
    /// engine-app's flush systems do — a tick isn't complete until they flush.
    fn tick(system: &mut WorldEventSystem, world: &mut World, resources: &mut Resources, world_secs: f64) {
        resources.insert(WorldTime((world_secs * 1e6) as u64));
        system.run(world, resources, 0.0);
        let fns: Vec<_> = resources.get_mut::<SpawnQueue>().unwrap().0.drain(..).collect();
        for f in fns {
            f(&mut SpawnContext { world, resources });
        }
        let pairs: Vec<_> = resources.get_mut::<DespawnQueue>().unwrap().0.drain(..).collect();
        for (entity, _) in pairs {
            world.despawn(entity).ok();
        }
    }

    fn wave_event(waves: Vec<EventWaveDef>) -> WorldEventsDef {
        WorldEventsDef {
            day_seconds: 100.0,
            events: vec![WorldEventDef {
                name: "blood_moon".into(),
                start_seconds_of_day: 30.0,
                duration_seconds: 20.0,
                ambient: Vec3::new(0.6, 0.05, 0.05),
                spawns: vec![],
                waves,
            }],
        }
    }

    fn tagged(world: &World) -> usize {
        world.query::<&EventSpawned>().iter().count()
    }

    #[test]
    fn event_wave_pulses_spawn_on_interval() {
        let def = wave_event(vec![EventWaveDef {
            prefab: "dummy".into(),
            positions: vec![Vec3::ZERO, Vec3::X],
            interval_seconds: 5.0,
            max_alive: 10,
        }]);
        let mut world = World::new();
        let mut resources = wave_resources(def);
        let mut system = WorldEventSystem::new();

        // Before the window: nothing.
        tick(&mut system, &mut world, &mut resources, 29.0);
        assert_eq!(tagged(&world), 0, "no waves before the window opens");

        // 36 s: one pulse due (start+1·5 = 35), two positions.
        tick(&mut system, &mut world, &mut resources, 36.0);
        assert_eq!(tagged(&world), 2, "one pulse at 36 s spawns 2 entities");

        // 46 s: pulses 1..3 due, so three pulses total = 6 entities.
        tick(&mut system, &mut world, &mut resources, 46.0);
        assert_eq!(tagged(&world), 6, "three pulses total by 46 s = 6 entities");
    }

    #[test]
    fn event_wave_respects_max_alive_cap() {
        let def = wave_event(vec![EventWaveDef {
            prefab: "dummy".into(),
            positions: vec![Vec3::ZERO, Vec3::X],
            interval_seconds: 3.0,
            max_alive: 3,
        }]);
        let mut world = World::new();
        let mut resources = wave_resources(def);
        let mut system = WorldEventSystem::new();

        // 39 s: pulses 1..3 due; 2 positions × 3 pulses = 6 requested, capped 3.
        tick(&mut system, &mut world, &mut resources, 39.0);
        assert_eq!(tagged(&world), 3, "cap holds the wave at 3 alive");

        // Free one slot, then advance one more pulse: the freed capacity is
        // reused and the forfeited over-cap pulses never burst.
        let victim = world.query::<(hecs::Entity, &EventSpawned)>().iter().next().unwrap().0;
        world.despawn(victim).unwrap();
        assert_eq!(tagged(&world), 2, "one killed");

        tick(&mut system, &mut world, &mut resources, 42.0);
        assert_eq!(tagged(&world), 3, "freed slot refilled by exactly one pulse, no burst");
    }

    #[test]
    fn window_end_despawns_event_spawns() {
        let def = wave_event(vec![EventWaveDef {
            prefab: "dummy".into(),
            positions: vec![Vec3::ZERO, Vec3::X],
            interval_seconds: 5.0,
            max_alive: 10,
        }]);
        let mut world = World::new();
        let mut resources = wave_resources(def);
        let mut system = WorldEventSystem::new();

        tick(&mut system, &mut world, &mut resources, 46.0);
        assert!(tagged(&world) > 0, "waves populated in-window");

        // Past the window end (30+20 = 50): every EventSpawned entity is reaped.
        tick(&mut system, &mut world, &mut resources, 55.0);
        assert_eq!(tagged(&world), 0, "window close despawns the event's wave spawns");
    }

    #[test]
    fn events_ron_parses_with_and_without_waves() {
        let without = r#"(day_seconds: 120.0, events: [(
            name: "e", start_seconds_of_day: 30.0, duration_seconds: 20.0,
            ambient: (0.7, 0.08, 0.08),
            spawns: [(prefab: "grunt", positions: [(1.0, 0.0, 0.0)])],
        )])"#;
        let d = ron::from_str::<WorldEventsDef>(without).unwrap();
        assert!(d.events[0].waves.is_empty(), "waves defaults empty when absent");

        let with = r#"(day_seconds: 120.0, events: [(
            name: "e", start_seconds_of_day: 30.0, duration_seconds: 20.0,
            ambient: (0.7, 0.08, 0.08),
            waves: [(prefab: "grunt", positions: [(16.0, 0.0, 16.0)],
                interval_seconds: 5.0, max_alive: 8)],
        )])"#;
        let d = ron::from_str::<WorldEventsDef>(with).unwrap();
        assert_eq!(d.events[0].waves.len(), 1);
        assert_eq!(d.events[0].waves[0].max_alive, 8);
    }

    fn defs() -> WorldEventsDef {
        WorldEventsDef {
            day_seconds: 100.0,
            events: vec![WorldEventDef {
                name: "blood_moon".into(),
                start_seconds_of_day: 30.0,
                duration_seconds: 20.0,
                ambient: Vec3::new(0.6, 0.05, 0.05),
                spawns: vec![],
                waves: vec![],
            }],
        }
    }

    #[test]
    fn active_event_window_and_daily_recurrence() {
        let d = defs();
        assert_eq!(active_event(&d, 29.9), None);
        assert_eq!(active_event(&d, 30.0), Some(0));
        assert_eq!(active_event(&d, 49.9), Some(0));
        assert_eq!(active_event(&d, 50.0), None);
        // Same window, next day.
        assert_eq!(active_event(&d, 135.0), Some(0));
    }

    #[test]
    fn noon_is_brighter_than_midnight() {
        let (_, _, midnight) = day_night_light(0.0);
        let (_, _, noon) = day_night_light(0.5);
        assert!(noon > midnight);
        // The sun never lights from below the horizon.
        let (dir, _, _) = day_night_light(0.0);
        assert!(dir.y > 0.0);
    }
}
