// Chapter definitions — what to spawn, when, and how fast. Pure data, loaded
// from RON. A chapter plugin inserts an ActiveChapter resource; the wave
// spawner systems drive everything from it.
//
// Chapters are LINKED MODULES (one crate each): ChapterModule describes a
// chapter's name, its dependency chain, and how to install it; the binaries
// build a ChapterRegistry from the chapter crates they link and install by
// name — no hardcoded name → plugin matches anywhere.

use engine_app::app::App;
use glam::Vec3;

/// One chapter crate's self-description. `requires` is the dependency chain
/// ("chapter02 requires chapter01"): installing a chapter first installs the
/// CONTENT of everything it requires (prefabs/components must exist for
/// carried-over entities), then its own full plugin.
pub struct ChapterModule {
    pub name: &'static str,
    pub requires: &'static [&'static str],
    /// Full simulation plugin (server zones, sandbox).
    pub install: fn(&mut App),
    /// Registration-only content subset (networked display clients, deps).
    pub install_content: fn(&mut App),
}

pub struct ChapterRegistry {
    modules: Vec<ChapterModule>,
}

impl ChapterRegistry {
    pub fn new(modules: Vec<ChapterModule>) -> Self {
        Self { modules }
    }

    fn find(&self, name: &str) -> Result<&ChapterModule, String> {
        self.modules
            .iter()
            .find(|m| m.name == name)
            .ok_or_else(|| format!("unknown chapter '{name}' (not linked into this binary)"))
    }

    /// Names of `name`'s transitive dependencies, dependencies first.
    /// Depth-first, cycle-checked; chapter chains are tiny.
    fn deps_of(&self, name: &str) -> Result<Vec<&'static str>, String> {
        fn visit<'a>(
            reg: &'a ChapterRegistry,
            name: &str,
            ordered: &mut Vec<&'static str>,
            visiting: &mut Vec<&'a str>,
        ) -> Result<(), String> {
            if visiting.iter().any(|v| *v == name) {
                return Err(format!("chapter dependency cycle through '{name}'"));
            }
            let module = reg.find(name)?;
            visiting.push(module.name);
            for dep in module.requires {
                if !ordered.contains(dep) {
                    visit(reg, dep, ordered, visiting)?;
                    ordered.push(dep);
                }
            }
            visiting.pop();
            Ok(())
        }
        let mut ordered = Vec::new();
        let mut visiting = Vec::new();
        visit(self, name, &mut ordered, &mut visiting)?;
        Ok(ordered)
    }

    /// Install chapter `name` into a simulation App: content of its
    /// transitive dependencies first, then its own full plugin.
    pub fn install(&self, name: &str, app: &mut App) -> Result<(), String> {
        for dep in self.deps_of(name)? {
            (self.find(dep)?.install_content)(app);
        }
        (self.find(name)?.install)(app);
        Ok(())
    }

    /// Install every linked chapter's CONTENT (a display client must be able
    /// to show replicated entities from any zone it can be redirected to).
    pub fn install_all_content(&self, app: &mut App) {
        for module in &self.modules {
            (module.install_content)(app);
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ChapterDef {
    pub name:          String,
    pub spawning:      SpawnConfig,
    #[serde(default)]
    pub initial_spawns: Vec<InitialSpawn>,
    /// World-resident enemy populations (Phase 7.5): fixed places, fixed
    /// headcount, respawn timers — the world exists whether or not anyone
    /// is nearby. Replaces around-the-player waves as the population model.
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
