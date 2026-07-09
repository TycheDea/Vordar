// Body composition — turns (RaceId, ClassId) into the entity's visual. Two
// outputs, chosen by whether the race defines a skinned model:
//   • mesh race  → RenderMesh (glTF) + LocomotionClips + AnimController; the
//     Phase-B runtime animates it. Class shows through the mesh tint. No
//     ShapeGroup/PoseRig — the skinned pipeline draws it.
//   • SDF race   → ShapeGroup (race base body + class outfit) + a PoseRig from
//     the class's pose params (the pre-Phase-C path, unchanged). Enemies/NPCs
//     never reach here (they author ShapeGroup directly and carry no Race).
// Client-only: the server parses the ids but never composes. Composed once per
// entity (BodyComposed marker); prefabs author no visual of their own.

use crate::locomotion::{AnimController, LocomotionClips};
use crate::pose::PoseRig;
use engine_app::scheduler::System;
use engine_core::components::{RenderMesh, ShapeGroup};
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;
use hecs::Entity;
use vordar_game::class::{ClassId, ClassLibrary, RaceId, RaceLibrary};

/// How long the attack one-shot latches for a mesh character (the KayKit chop
/// is well under this); locomotion resumes after.
const MESH_ATTACK_SECS: f32 = 0.6;

/// Marker: this entity's ShapeGroup has been assembled.
pub struct BodyComposed;

pub struct BodyComposeSystem;

impl System for BodyComposeSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let candidates: Vec<(Entity, String, Option<String>)> = world
            .query::<(Entity, &RaceId, Option<&ClassId>)>()
            .iter()
            .filter(|&(entity, ..)| world.get::<&BodyComposed>(entity).is_err())
            .map(|(entity, race, class)| (entity, race.id.clone(), class.map(|c| c.id.clone())))
            .collect();
        if candidates.is_empty() {
            return;
        }

        let races = resources.get::<RaceLibrary>().expect("RaceLibrary not in resources");
        let classes = resources.get::<ClassLibrary>().expect("ClassLibrary not in resources");
        for (entity, race_id, class_id) in candidates {
            let class = class_id.as_deref().and_then(|id| classes.class(id));

            // Mesh race: attach the skinned-character bundle and skip the SDF
            // path entirely (no ShapeGroup → no double-draw). Scale + ground
            // offset are baked into the asset's armature, so Transform is left
            // alone.
            if let Some(model) = races.model(&race_id) {
                let tint = class.and_then(|c| c.tint).unwrap_or(Vec3::ONE);
                let clips = LocomotionClips {
                    idle:   model.idle.clone(),
                    walk:   model.walk.clone(),
                    run:    model.run.clone(),
                    attack: model.attack.clone(),
                    death:  model.death.clone(),
                    hit:    model.hit.clone(),
                    walk_speed:  model.walk_speed,
                    run_speed:   model.run_speed,
                    attack_secs: MESH_ATTACK_SECS,
                    forward_offset: model.forward_offset,
                };
                // TEMP (anim feel-check): confirm the live player takes the mesh
                // branch. Remove with the MeshRenderSyncSystem pose log.
                log::info!("body compose: race '{race_id}' -> skinned mesh {}", model.asset);
                let _ = world.insert(entity, (
                    RenderMesh { asset: model.asset.clone(), tint },
                    clips,
                    AnimController::default(),
                    BodyComposed,
                ));
                continue;
            }

            let Some(base) = races.base(&race_id) else {
                log::error!("body compose: unknown race '{race_id}'");
                let _ = world.insert_one(entity, BodyComposed);
                continue;
            };
            let base_len = base.len();
            let mut shapes = base.to_vec();
            // No ClassId = no gear (e.g. town NPCs wear only their base body).
            if let Some(def) = class {
                shapes.extend(def.outfit.iter().cloned());
            }

            // Pose rig: torso = base shape 0 (race convention), swing shape =
            // outfit-relative index shifted past the base.
            let pose = class.map(|c| c.pose.clone()).unwrap_or_default();
            let swing_index = pose.swing_index.map(|i| base_len + i).filter(|&i| i < shapes.len());
            let rig = PoseRig {
                bob_amplitude: pose.bob_amplitude,
                bob_speed: pose.bob_speed,
                swing_index,
                swing_arc: pose.swing_arc,
                torso_rest_y: shapes.first().map(|s| s.offset.y).unwrap_or(0.0),
                swing_rest: swing_index.map(|i| shapes[i].offset).unwrap_or_default(),
                swing_rest_rotation: swing_index.map(|i| shapes[i].rotation).unwrap_or_default(),
                phase: 0.0,
                swing_t: None,
            };
            let _ = world.insert(entity, (ShapeGroup { shapes }, rig, BodyComposed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vordar_game::class::{ClassDef, PoseParams, RaceDef, RaceModel};

    fn libraries() -> Resources {
        let mut resources = Resources::new();
        let mut races = RaceLibrary::new();
        races.insert("human", vec![sub(), sub()]);
        // A mesh race (Phase C) alongside the SDF one.
        races.insert_def("golem", RaceDef {
            body:  Vec::new(),
            model: Some(RaceModel {
                asset: "content/models/golem.glb".into(),
                idle: "Idle".into(), walk: "Walk".into(), run: "Run".into(),
                attack: "Attack".into(), death: "Death".into(), hit: "Hit".into(),
                walk_speed: 0.1, run_speed: 3.0, forward_offset: 0.0,
            }),
        });
        let mut classes = ClassLibrary::new();
        classes.insert(ClassDef {
            id: "ravager".into(),
            abilities: Vec::new(),
            outfit: vec![sub(), sub(), sub()],
            pose: PoseParams::default(),
            tint: Some(glam::Vec3::new(1.0, 0.5, 0.5)),
        });
        resources.insert(races);
        resources.insert(classes);
        resources
    }

    fn sub() -> engine_core::components::SubShape {
        engine_core::components::SubShape {
            shape: Default::default(),
            offset: glam::Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: glam::Vec3::ONE,
            color: glam::Vec3::ONE,
        }
    }

    #[test]
    fn composes_base_plus_outfit_once() {
        let mut world = World::new();
        let mut resources = libraries();
        let dressed = world.spawn((RaceId { id: "human".into() }, ClassId { id: "ravager".into() }));
        let bare = world.spawn((RaceId { id: "human".into() },));

        let mut sys = BodyComposeSystem;
        sys.run(&mut world, &mut resources, 0.016);
        assert_eq!(world.get::<&ShapeGroup>(dressed).unwrap().shapes.len(), 5, "base 2 + outfit 3");
        assert_eq!(world.get::<&ShapeGroup>(bare).unwrap().shapes.len(), 2, "no class = bare base");

        // Idempotent: a second run must not stack outfits.
        sys.run(&mut world, &mut resources, 0.016);
        assert_eq!(world.get::<&ShapeGroup>(dressed).unwrap().shapes.len(), 5);
    }

    #[test]
    fn mesh_race_gets_render_mesh_and_locomotion_not_shapegroup() {
        let mut world = World::new();
        let mut resources = libraries();
        let entity = world.spawn((RaceId { id: "golem".into() }, ClassId { id: "ravager".into() }));

        let mut sys = BodyComposeSystem;
        sys.run(&mut world, &mut resources, 0.016);

        // Skinned-character bundle, and crucially NO ShapeGroup/PoseRig (those
        // would double-draw against the mesh).
        let mesh = world.get::<&RenderMesh>(entity).expect("mesh race gets a RenderMesh");
        assert_eq!(mesh.asset, "content/models/golem.glb");
        assert_eq!(mesh.tint, glam::Vec3::new(1.0, 0.5, 0.5), "class tint applied");
        let clips = world.get::<&LocomotionClips>(entity).expect("locomotion clips");
        assert_eq!(clips.run, "Run");
        assert!(world.get::<&AnimController>(entity).is_ok());
        assert!(world.get::<&ShapeGroup>(entity).is_err(), "no SDF body for a mesh race");
        assert!(world.get::<&PoseRig>(entity).is_err(), "no pose rig for a mesh race");
        assert!(world.get::<&BodyComposed>(entity).is_ok(), "marked composed");
    }

    /// END-TO-END WIRING: the actual sandbox player (Human race, ravager class)
    /// composed from the REAL content libraries must become the skinned human
    /// mesh with locomotion + the ravager tint — not the SDF body. This is the
    /// on-disk-content equivalent of the synthetic test above; if it fails, the
    /// live player silently falls back and none of the animation work is visible.
    #[test]
    fn real_ravager_player_composes_to_the_human_mesh() {
        let races_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/races");
        if !std::path::Path::new(races_dir).exists() {
            return;
        }
        let mut resources = Resources::new();
        let mut races = RaceLibrary::new();
        races.load_dir(races_dir);
        let mut classes = ClassLibrary::new();
        classes.load_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/classes"));
        resources.insert(races);
        resources.insert(classes);

        let mut world = World::new();
        let player = world.spawn((RaceId { id: "human".into() }, ClassId { id: "ravager".into() }));
        BodyComposeSystem.run(&mut world, &mut resources, 0.016);

        let mesh = world.get::<&RenderMesh>(player).expect("player must render as a mesh");
        assert_eq!(mesh.asset, "content/models/human.glb");
        assert_ne!(mesh.tint, Vec3::ONE, "ravager tint applied");
        let clips = world.get::<&LocomotionClips>(player).expect("player must have locomotion");
        assert_eq!((clips.idle.as_str(), clips.run.as_str()), ("idle", "run"));
        assert!((clips.forward_offset - std::f32::consts::PI).abs() < 1e-3, "π forward offset from race RON");
        assert!(world.get::<&AnimController>(player).is_ok());
        assert!(world.get::<&ShapeGroup>(player).is_err(), "NOT the SDF fallback");
    }
}
