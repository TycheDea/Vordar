// Class system — a class is a named, RON-authored bundle of castable
// abilities plus its outfit (the gear shapes layered onto a race's base body).
// Loaded from content/classes/ by filename stem, mirroring PrefabLibrary's
// load-dir-by-filename-stem convention (engine-core/src/prefab.rs).
//
// Character visual = race base + class outfit, assembled client-side by
// BodyComposeSystem (ShapeGroup is presentation-only; the server parses
// RaceId/ClassId but never composes).

use crate::player::skills::AbilityDef;
use engine_core::components::SubShape;
use glam::Vec3;
use std::collections::HashMap;

// Re-export from the race module.
pub use super::race::{RaceId, RaceLibrary, RaceModel};

/// Fallback class for entities with no `ClassId` (ad-hoc test entities that
/// never spawned from a real prefab).
pub const DEFAULT_CLASS: &str = "human";

/// Which class an entity plays as — authored on the entity's prefab
/// (`"Class": (id: "human")`).
#[derive(Clone, serde::Deserialize)]
pub struct ClassId {
    pub id: String,
}

/// Procedural-pose tuning, authored per class (the outfit defines what
/// swings). Consumed by the client's pose system; zero everything = static.
#[derive(Clone, Default, serde::Deserialize)]
pub struct PoseParams {
    /// Idle-breath bob height on the torso (base shape 0).
    #[serde(default)]
    pub bob_amplitude: f32,
    /// Bob cycles per second × TAU (radians/sec of the sine phase).
    #[serde(default)]
    pub bob_speed: f32,
    /// Outfit-relative index of the shape swung during a cast.
    #[serde(default)]
    pub swing_index: Option<usize>,
    /// Peak offset delta of the cast swing, in entity-local space.
    #[serde(default)]
    pub swing_arc: Vec3,
}

#[derive(Clone)]
pub struct ClassDef {
    pub id: String,
    pub abilities: Vec<AbilityDef>,
    /// Gear shapes appended after the race's base body (SDF races only — a
    /// glTF race ships pre-dressed, so its class shows through `tint` instead).
    pub outfit: Vec<SubShape>,
    pub pose: PoseParams,
    /// Colour multiplier for a skinned-mesh race worn by this class. `None` =
    /// unchanged (white). Ignored on SDF races (their outfit does the dressing).
    pub tint: Option<Vec3>,
}

/// On-disk shape of one `content/classes/<id>.ron` file.
#[derive(serde::Deserialize)]
struct ClassDefFile {
    abilities: Vec<AbilityDef>,
    #[serde(default)]
    outfit: Vec<SubShape>,
    #[serde(default)]
    pose: PoseParams,
    #[serde(default)]
    tint: Option<Vec3>,
}

/// Loaded class definitions, keyed by class id (= RON filename stem).
/// Mirrors `PrefabLibrary::load_dir` exactly, including its "one bad file
/// must not take the whole library down" error handling.
#[derive(Clone, Default)]
pub struct ClassLibrary {
    classes: HashMap<String, ClassDef>,
}

impl ClassLibrary {
    pub fn new() -> Self {
        Self { classes: HashMap::new() }
    }

    pub fn insert(&mut self, def: ClassDef) {
        let id = def.id.clone();
        if self.classes.insert(id.clone(), def).is_some() {
            log::warn!("class '{id}' was overwritten");
        }
    }

    /// Load every *.ron file in `dir` as a class definition; the class id is
    /// the file stem.
    pub fn load_dir(&mut self, dir: &str) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                log::error!("class dir '{dir}' unreadable: {e}");
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
                Ok(text) => match ron::from_str::<ClassDefFile>(&text) {
                    Ok(file) => {
                        log::info!("class loaded: '{stem}' ({} abilities)", file.abilities.len());
                        self.insert(ClassDef {
                            id: stem.to_owned(),
                            abilities: file.abilities,
                            outfit: file.outfit,
                            pose: file.pose,
                            tint: file.tint,
                        });
                    }
                    Err(e) => log::error!("class '{}' parse error: {e}", path.display()),
                },
                Err(e) => log::error!("class '{}' read error: {e}", path.display()),
            }
        }
    }

    pub fn get(&self, class_id: &str, ability_id: &str) -> Option<&AbilityDef> {
        self.classes.get(class_id)?.abilities.iter().find(|a| a.id == ability_id)
    }

    pub fn abilities_of(&self, class_id: &str) -> &[AbilityDef] {
        self.classes.get(class_id).map(|c| c.abilities.as_slice()).unwrap_or(&[])
    }

    pub fn class(&self, class_id: &str) -> Option<&ClassDef> {
        self.classes.get(class_id)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::skills::AbilityEffect;

    fn sample() -> ClassDef {
        ClassDef {
            id: "human".into(),
            outfit: Vec::new(),
            pose: PoseParams::default(),
            tint: None,
            abilities: vec![
                AbilityDef {
                    id: "blast".into(),
                    name: "Blast".into(),
                    cooldown_micros: 3_000_000,
                    anim: None,
                    anim_secs: None,
                    effect: AbilityEffect::Scheduled {
                        telegraph_prefab: "telegraph".into(),
                        radius: 4.0,
                        damage: 25,
                        damage_type: Default::default(),
                        cast_micros: 2_000_000,
                        max_range: 15.0,
                    },
                },
                AbilityDef {
                    id: "bolt".into(),
                    name: "Bolt".into(),
                    cooldown_micros: 600_000,
                    anim: None,
                    anim_secs: None,
                    effect: AbilityEffect::Projectile {
                        prefab: "bolt".into(),
                        speed: 18.0,
                        damage: 12,
                        damage_type: Default::default(),
                        ttl_secs: 1.5,
                        spawn_offset: 0.9,
                    },
                },
            ],
        }
    }

    #[test]
    fn get_finds_ability_by_class_and_id() {
        let mut lib = ClassLibrary::new();
        lib.insert(sample());
        let blast = lib.get("human", "blast").unwrap();
        assert_eq!(blast.cooldown_micros, 3_000_000);
        match &blast.effect {
            AbilityEffect::Scheduled { radius, damage, .. } => {
                assert_eq!(*radius, 4.0);
                assert_eq!(*damage, 25);
            }
            _ => panic!("blast must stay a scheduled mechanic"),
        }
    }

    #[test]
    fn unknown_class_or_ability_is_none() {
        let mut lib = ClassLibrary::new();
        lib.insert(sample());
        assert!(lib.get("ravager", "blast").is_none());
        assert!(lib.get("human", "nonexistent").is_none());
    }

    #[test]
    fn abilities_of_lists_in_authored_order() {
        let mut lib = ClassLibrary::new();
        lib.insert(sample());
        let ids: Vec<&str> = lib.abilities_of("human").iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["blast", "bolt"]);
    }

    /// The real content: class tints parse off disk.
    #[test]
    fn real_class_content_parses_if_present() {
        let classes_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/classes");
        if !std::path::Path::new(classes_dir).exists() {
            return;
        }
        let mut classes = ClassLibrary::new();
        classes.load_dir(classes_dir);
        assert!(
            classes.class("ravager").expect("ravager class").tint.is_some(),
            "ravager tints its mesh"
        );

        // Per-ability cast animations: each authored ability names a clip
        // that exists in the preprocessed rigs.
        assert_eq!(
            classes.get("ravager", "rend").unwrap().anim.as_deref(),
            Some("attack_slash")
        );
        assert_eq!(
            classes.get("ravager", "onslaught").unwrap().anim.as_deref(),
            Some("attack_slash")
        );
        assert_eq!(
            classes.get("human", "bolt").unwrap().anim.as_deref(),
            Some("attack_cast")
        );
    }
}
