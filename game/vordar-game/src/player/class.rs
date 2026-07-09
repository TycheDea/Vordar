// Class & race system — a class is a named, RON-authored bundle of castable
// abilities plus its outfit (the gear shapes layered onto a race's base
// body); a race is a base body. Both libraries mirror PrefabLibrary's
// load-dir-by-filename-stem convention (engine-core/src/prefab.rs).
//
// Character visual = race base + class outfit, assembled client-side by
// BodyComposeSystem (ShapeGroup is presentation-only; the server parses
// RaceId/ClassId but never composes).

use crate::player::skills::AbilityDef;
use engine_core::components::SubShape;
use glam::Vec3;
use std::collections::HashMap;

/// Fallback class for entities with no `ClassId` (ad-hoc test entities that
/// never spawned from a real prefab).
pub const DEFAULT_CLASS: &str = "human";

/// Which class an entity plays as — authored on the entity's prefab
/// (`"Class": (id: "human")`).
#[derive(Clone, serde::Deserialize)]
pub struct ClassId {
    pub id: String,
}

/// Which race's base body an entity wears — authored on the entity's prefab
/// (`"Race": (id: "human")`).
#[derive(Clone, serde::Deserialize)]
pub struct RaceId {
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

/// A skinned glTF character for a race — the Phase-C alternative to the SDF
/// base body. Clip names are read from whatever the asset ships; the client
/// maps them into its `LocomotionClips`. `attack`/`death` may be empty when the
/// rig lacks them. Scale + ground offset are baked into the asset's armature
/// (honoured by the renderer's skeleton root transform), so no scale knob here.
#[derive(Clone, serde::Deserialize)]
pub struct RaceModel {
    /// glTF path, content convention "content/models/<race>.glb".
    pub asset: String,
    pub idle: String,
    pub walk: String,
    pub run:  String,
    #[serde(default)]
    pub attack: String,
    #[serde(default)]
    pub death: String,
    /// Hit-react clip (flinch on damage); empty when the rig lacks one.
    #[serde(default)]
    pub hit: String,
    /// Move faster than this → at least walk; faster than `run_speed` → run.
    pub walk_speed: f32,
    pub run_speed:  f32,
    /// Radians added to the facing yaw — glTF rigs disagree on forward axis.
    #[serde(default)]
    pub forward_offset: f32,
}

/// One race's presentation: an SDF base body (index 0 = torso, 1 = head, extra
/// silhouette after — stable pose anchors), and/or a skinned glTF model. A race
/// with a `model` renders as a rigged mesh; otherwise it composes from `body`.
#[derive(Clone, Default)]
pub struct RaceDef {
    pub body:  Vec<SubShape>,
    pub model: Option<RaceModel>,
}

/// On-disk shape of one `content/races/<id>.ron` file.
#[derive(serde::Deserialize)]
struct RaceDefFile {
    #[serde(default)]
    body:  Vec<SubShape>,
    #[serde(default)]
    model: Option<RaceModel>,
}

/// Loaded race definitions, keyed by race id (= RON filename stem).
#[derive(Clone, Default)]
pub struct RaceLibrary {
    races: HashMap<String, RaceDef>,
}

impl RaceLibrary {
    pub fn new() -> Self {
        Self { races: HashMap::new() }
    }

    /// Insert an SDF-only race (the common case + tests): a base body, no model.
    pub fn insert(&mut self, id: impl Into<String>, body: Vec<SubShape>) {
        self.insert_def(id, RaceDef { body, model: None });
    }

    pub fn insert_def(&mut self, id: impl Into<String>, def: RaceDef) {
        let id = id.into();
        if self.races.insert(id.clone(), def).is_some() {
            log::warn!("race '{id}' was overwritten");
        }
    }

    pub fn load_dir(&mut self, dir: &str) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                log::error!("race dir '{dir}' unreadable: {e}");
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
                Ok(text) => match ron::from_str::<RaceDefFile>(&text) {
                    Ok(file) => {
                        log::info!(
                            "race loaded: '{stem}' ({} base shapes, model: {})",
                            file.body.len(),
                            file.model.as_ref().map(|m| m.asset.as_str()).unwrap_or("none"),
                        );
                        self.insert_def(stem, RaceDef { body: file.body, model: file.model });
                    }
                    Err(e) => log::error!("race '{}' parse error: {e}", path.display()),
                },
                Err(e) => log::error!("race '{}' read error: {e}", path.display()),
            }
        }
    }

    pub fn base(&self, race_id: &str) -> Option<&[SubShape]> {
        self.races.get(race_id).map(|d| d.body.as_slice())
    }

    /// The race's skinned model, if it renders as a rigged mesh (Phase C).
    pub fn model(&self, race_id: &str) -> Option<&RaceModel> {
        self.races.get(race_id).and_then(|d| d.model.as_ref())
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

    #[test]
    fn race_model_round_trips_from_ron() {
        let ron = r#"(
            model: Some((
                asset: "content/models/human.glb",
                idle: "Idle", walk: "Walking_A", run: "Running_A",
                attack: "1H_Melee_Attack_Chop", death: "Death_A",
                walk_speed: 0.1, run_speed: 3.0,
            )),
        )"#;
        let file: super::RaceDefFile = ron::from_str(ron).unwrap();
        let m = file.model.expect("model present");
        assert_eq!(m.asset, "content/models/human.glb");
        assert_eq!(m.run, "Running_A");
        assert_eq!(m.forward_offset, 0.0, "defaults to 0");
        assert!(file.body.is_empty(), "a mesh race needs no SDF body");
    }

    /// The real Phase-C content: race models + class tints parse off disk.
    #[test]
    fn real_race_and_class_content_parses_if_present() {
        let races_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/races");
        if !std::path::Path::new(races_dir).exists() {
            return;
        }
        let mut races = RaceLibrary::new();
        races.load_dir(races_dir);
        let human = races.model("human").expect("human race has a skinned model");
        assert!(human.asset.contains("human.glb"));
        assert_eq!(human.idle, "idle");

        let mut classes = ClassLibrary::new();
        classes.load_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/classes"));
        assert!(
            classes.class("ravager").expect("ravager class").tint.is_some(),
            "ravager tints its mesh"
        );

        // Per-ability cast animations (Phase D): each authored ability names a
        // clip that exists in the preprocessed rigs.
        assert_eq!(
            classes.get("ravager", "rend").unwrap().anim.as_deref(),
            Some("attack_slash")
        );
        assert_eq!(
            classes.get("ravager", "onslaught").unwrap().anim.as_deref(),
            Some("leap")
        );
        assert_eq!(
            classes.get("human", "bolt").unwrap().anim.as_deref(),
            Some("attack_cast")
        );
        assert_eq!(human.hit, "hit", "races map a hit-react clip");
    }
}
