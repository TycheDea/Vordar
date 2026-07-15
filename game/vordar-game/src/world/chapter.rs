// Chapter content model — what to spawn, when, and how fast. Pure data,
// loaded from RON. A chapter plugin inserts an ActiveChapter resource; the
// wave spawner systems drive everything from it.
//
// Chapter registration and installation (how chapters link into the binary)
// lives in chapter_registry.rs; this module re-exports it for compatibility.

pub use super::chapter_registry::{ChapterModule, ChapterRegistry};
use glam::Vec3;

#[derive(serde::Deserialize)]
pub struct ChapterDef {
    pub name:          String,
    pub spawning:      SpawnConfig,
    #[serde(default)]
    pub initial_spawns: Vec<InitialSpawn>,
    /// World-resident enemy populations: fixed places, fixed headcount,
    /// respawn timers — the world exists whether or not anyone is nearby.
    #[serde(default)]
    pub camps: Vec<CampDef>,
}

#[derive(serde::Deserialize)]
pub struct SpawnConfig {
    /// Wave timers freeze while this many enemies are alive.
    pub max_alive: usize,
    pub waves:     Vec<WaveDef>,
}

#[derive(serde::Deserialize)]
pub struct WaveDef {
    /// Chapter seconds at which this wave activates.
    #[serde(default)]
    pub start_time:      f32,
    /// Prefab id to spawn.
    pub prefab:          String,
    /// Seconds between spawns once active.
    pub interval:        f32,
    /// Ring radius around the player.
    pub spawn_radius:    f32,
    #[serde(default = "default_count")]
    pub count_per_spawn: usize,
}

fn default_count() -> usize { 1 }

#[derive(serde::Deserialize)]
pub struct InitialSpawn {
    pub prefab:    String,
    pub positions: Vec<Vec3>,
}

#[derive(serde::Deserialize)]
pub struct CampDef {
    pub prefab:          String,
    pub center:          Vec3,
    pub radius:          f32,
    pub count:           usize,
    /// Seconds after a member dies until its slot refills.
    pub respawn_seconds: f32,
}

/// Deterministic position of camp slot `i`: golden-angle scatter inside the
/// camp radius — even spread, no RNG, stable across runs and processes.
pub fn camp_slot_pos(camp: &CampDef, i: usize) -> Vec3 {
    let count = camp.count.max(1) as f32;
    let r = camp.radius * (((i as f32) + 0.5) / count).sqrt();
    let theta = i as f32 * 2.399_963; // golden angle in radians
    camp.center + Vec3::new(r * theta.cos(), 0.0, r * theta.sin())
}

/// Runtime state wrapping the loaded definition. Inserted as a resource by a
/// chapter plugin; consumed by ChapterSetupSystem and WaveSpawnerSystem.
pub struct ActiveChapter {
    pub def:         ChapterDef,
    pub elapsed:     f32,
    pub wave_timers: Vec<f32>,
    /// Rotating angle for ring placement — spreads consecutive spawns around the player.
    pub spawn_angle: f32,
    pub started:     bool,
}

/// Load a chapter RON file. Panics with a clear message on failure — a broken
/// chapter is a content bug the author must see immediately, not a fallback.
pub fn load_chapter(path: &str) -> ActiveChapter {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("chapter '{path}' unreadable: {e}"));
    let def: ChapterDef = ron::from_str(&text)
        .unwrap_or_else(|e| panic!("chapter '{path}' parse error: {e}"));
    log::info!("chapter loaded: '{}' ({} waves)", def.name, def.spawning.waves.len());
    let wave_timers = vec![0.0; def.spawning.waves.len()];
    ActiveChapter { def, elapsed: 0.0, wave_timers, spawn_angle: 0.0, started: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camp_slot_positions_deterministic_and_in_radius() {
        let camp = CampDef {
            prefab: "grunt".into(),
            center: Vec3::new(10.0, 0.0, -5.0),
            radius: 4.0,
            count: 7,
            respawn_seconds: 10.0,
        };
        for i in 0..camp.count {
            let a = camp_slot_pos(&camp, i);
            let b = camp_slot_pos(&camp, i);
            assert_eq!(a, b, "slot {i} must be deterministic");
            assert!(
                a.distance(camp.center) <= camp.radius + 1e-4,
                "slot {i} escaped the camp: {a}"
            );
            assert_eq!(a.y, 0.0);
        }
        // Slots don't stack.
        assert!(camp_slot_pos(&camp, 0).distance(camp_slot_pos(&camp, 1)) > 0.5);
    }

    #[test]
    fn camps_field_defaults_to_empty_on_old_chapters() {
        let old_style = r#"(
            name: "old",
            spawning: ( max_alive: 5, waves: [] ),
        )"#;
        let def: ChapterDef = ron::from_str(old_style).unwrap();
        assert!(def.camps.is_empty());
        assert!(def.initial_spawns.is_empty());
    }
}
