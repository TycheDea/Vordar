// Race system — a race is a base body that a character wears. Loaded from
// content/races/ by filename stem, mirroring PrefabLibrary's load-dir
// convention (engine-core/src/prefab.rs).

use engine_core::components::SubShape;
use std::collections::HashMap;

/// Which race's base body an entity wears — authored on the entity's prefab
/// (`"Race": (id: "human")`).
#[derive(Clone, serde::Deserialize)]
pub struct RaceId {
    pub id: String,
}

/// A skinned glTF character for a race — an alternative to the SDF base
/// body. Clip names are read from whatever the asset ships; the client
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
/// Mirrors `PrefabLibrary::load_dir` exactly, including its "one bad file
/// must not take the whole library down" error handling.
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

    /// Load every *.ron file in `dir` as a race definition; the race id is
    /// the file stem.
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

    /// The race's skinned model, if it renders as a rigged mesh.
    pub fn model(&self, race_id: &str) -> Option<&RaceModel> {
        self.races.get(race_id).and_then(|d| d.model.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let file: RaceDefFile = ron::from_str(ron).unwrap();
        let m = file.model.expect("model present");
        assert_eq!(m.asset, "content/models/human.glb");
        assert_eq!(m.run, "Running_A");
        assert_eq!(m.forward_offset, 0.0, "defaults to 0");
        assert!(file.body.is_empty(), "a mesh race needs no SDF body");
    }

    /// The real content: race models parse off disk.
    #[test]
    fn real_race_content_parses_if_present() {
        let races_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/races");
        if !std::path::Path::new(races_dir).exists() {
            return;
        }
        let mut races = RaceLibrary::new();
        races.load_dir(races_dir);
        let human = races.model("human").expect("human race has a skinned model");
        assert!(human.asset.contains("human.glb"));
        assert_eq!(human.idle, "idle");
    }
}
