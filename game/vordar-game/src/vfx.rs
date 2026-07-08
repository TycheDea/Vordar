// Cosmetic VFX data — shared so prefabs and effect defs parse on the server
// too (it never reads them). The client's vfx module turns them into
// particles.
//
// Three beats per ability (VQ-E1):
//   cast   — content/vfx/<ability-id>.ron, at the caster's hand socket
//   travel — VfxTrail on the projectile prefab (server-spawned bolts too)
//   impact — VfxTrail.impact, burst where the projectile dies
// Scheduled abilities telegraph instead of travelling; their impact fires at
// telegraph resolve (client code, threat-colored per VQ-A4).

use glam::Vec3;
use serde::Deserialize;
use std::collections::HashMap;

/// How a particle composites (VQ-E3): additive for energy (glows, sparks),
/// premultiplied alpha for occluding media (smoke, dust).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub enum ParticleBlend {
    #[default]
    Additive,
    Alpha,
}

/// A radial burst (cast or impact beat).
#[derive(Clone, Debug, Deserialize)]
pub struct BurstDef {
    pub count: usize,
    pub speed: f32,
    pub size:  f32,
    /// Components above 1.0 are HDR emissive (bloom, VQ-C3).
    pub color: Vec3,
    /// Atlas cell (4×4 grid; 0 = soft glow, 1 = core glow, 2 = streak, 3 = smoke).
    #[serde(default)]
    pub cell: u32,
    #[serde(default)]
    pub blend: ParticleBlend,
    #[serde(default = "default_burst_ttl")]
    pub ttl: (f32, f32),
    #[serde(default = "default_gravity")]
    pub gravity: f32,
    #[serde(default = "default_drag")]
    pub drag: f32,
    /// Velocity-stretch factor: 0 = round billboard, >0 elongates along motion.
    #[serde(default)]
    pub stretch: f32,
}

fn default_burst_ttl() -> (f32, f32) {
    (0.30, 0.55)
}
fn default_gravity() -> f32 {
    -7.0
}
fn default_drag() -> f32 {
    2.5
}

/// One ability's effect beats. Missing beats fall back to the legacy tinted
/// spark burst so unauthored abilities still read.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct VfxDef {
    #[serde(default)]
    pub cast: Option<BurstDef>,
}

/// Loaded effect defs keyed by ability id (= RON filename stem), mirroring
/// ClassLibrary's load-dir convention.
#[derive(Default)]
pub struct VfxLibrary {
    effects: HashMap<String, VfxDef>,
}

impl VfxLibrary {
    pub fn new() -> Self {
        Self { effects: HashMap::new() }
    }

    pub fn insert(&mut self, id: impl Into<String>, def: VfxDef) {
        self.effects.insert(id.into(), def);
    }

    pub fn get(&self, ability_id: &str) -> Option<&VfxDef> {
        self.effects.get(ability_id)
    }

    pub fn load_dir(&mut self, dir: &str) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                log::error!("vfx dir '{dir}' unreadable: {e}");
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("ron") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            match std::fs::read_to_string(&path) {
                Ok(text) => match ron::from_str::<VfxDef>(&text) {
                    Ok(def) => {
                        self.insert(stem, def);
                    }
                    Err(e) => log::error!("vfx '{}' parse error: {e}", path.display()),
                },
                Err(e) => log::error!("vfx '{}' read error: {e}", path.display()),
            }
        }
        log::info!("vfx defs loaded: {}", self.effects.len());
    }
}

/// Emit a particle trail from this entity while it moves (projectiles) —
/// the travel beat. `rate` is particles per second. `impact` bursts where
/// the entity despawns (the impact beat).
#[derive(Clone, Debug, Deserialize)]
pub struct VfxTrail {
    pub color: Vec3,
    pub rate:  f32,
    #[serde(default)]
    pub cell: u32,
    #[serde(default)]
    pub blend: ParticleBlend,
    /// Velocity-stretch for trail motes (projectile streaks).
    #[serde(default)]
    pub stretch: f32,
    #[serde(default)]
    pub impact: Option<BurstDef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfx_def_parses_with_defaults() {
        let def: VfxDef = ron::from_str(
            r#"(cast: Some((count: 12, speed: 4.0, size: 0.12, color: (0.7, 1.9, 2.0))))"#,
        )
        .unwrap();
        let cast = def.cast.unwrap();
        assert_eq!(cast.cell, 0);
        assert_eq!(cast.blend, ParticleBlend::Additive);
        assert_eq!(cast.ttl, (0.30, 0.55));
    }

    #[test]
    fn trail_with_impact_parses() {
        let t: VfxTrail = ron::from_str(
            r#"(color: (0.7, 1.9, 2.0), rate: 60.0, cell: 2, stretch: 1.5,
                impact: Some((count: 10, speed: 3.0, size: 0.1, color: (0.7, 1.9, 2.0))))"#,
        )
        .unwrap();
        assert_eq!(t.cell, 2);
        assert!(t.impact.is_some());
    }

    #[test]
    fn legacy_trail_shape_still_parses() {
        // Pre-Phase-7 prefabs author only color + rate.
        let t: VfxTrail = ron::from_str(r#"(color: (1.0, 0.5, 0.1), rate: 45.0)"#).unwrap();
        assert_eq!(t.stretch, 0.0);
        assert!(t.impact.is_none());
    }
}
