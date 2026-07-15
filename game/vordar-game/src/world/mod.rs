// The world — its clock and timed events (below), plus how it is laid out
// and populated: zones/portals, chapters, camps, wave spawning (submodules).
//
// World clock + timed world events (DESIGN.md §4): one authoritative world
// time; events are deterministic shared definitions evaluated against it.
// Synchronization is trivial by construction: every process that knows the
// clock and loads the same defs agrees on what is active — the server
// spawns, clients tint, nobody exchanges event state.

pub mod camp;
pub mod chapter;
mod chapter_registry;
pub mod wave_spawner;
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
}

#[derive(serde::Deserialize)]
pub struct WorldSpawn {
    pub prefab: String,
    pub positions: Vec<Vec3>,
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

/// Fires each event's spawns once per world day, on entering its window.
/// Mid-window joiners need nothing special: spawned entities replicate via
/// AOI, and tint is a pure function of the clock.
pub struct WorldEventSystem {
    /// Per-event index of the last world day it fired on.
    fired: Vec<i64>,
}

impl WorldEventSystem {
    pub fn new() -> Self {
        Self { fired: Vec::new() }
    }
}

impl System for WorldEventSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let to_spawn: Vec<(String, Vec3)> = {
            let Some(now) = resources.get::<WorldTime>().map(|t| t.0) else { return };
            let Some(def) = resources.get::<WorldEventsDef>() else { return };
            if self.fired.len() != def.events.len() {
                self.fired = vec![i64::MIN; def.events.len()];
            }

            let world_seconds = now as f64 * 1e-6;
            let day = (world_seconds / def.day_seconds).floor() as i64;
            let day_time = world_seconds - day as f64 * def.day_seconds;

            let mut to_spawn = Vec::new();
            for (i, event) in def.events.iter().enumerate() {
                let in_window = day_time >= event.start_seconds_of_day
                    && day_time < event.start_seconds_of_day + event.duration_seconds;
                if !in_window || self.fired[i] == day {
                    continue;
                }
                self.fired[i] = day;
                log::info!("world event '{}' started (day {day})", event.name);
                for spawn in &event.spawns {
                    for &pos in &spawn.positions {
                        to_spawn.push((spawn.prefab.clone(), pos));
                    }
                }
            }
            to_spawn
        };

        for (prefab, pos) in to_spawn {
            queue_prefab_spawn(resources, prefab, pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> WorldEventsDef {
        WorldEventsDef {
            day_seconds: 100.0,
            events: vec![WorldEventDef {
                name: "blood_moon".into(),
                start_seconds_of_day: 30.0,
                duration_seconds: 20.0,
                ambient: Vec3::new(0.6, 0.05, 0.05),
                spawns: vec![],
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
