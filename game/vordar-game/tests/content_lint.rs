// Content lint — machine checks for the visual quality bar
// (docs/visual-quality.md). Pure CPU: glTF parsing via `load_gltf_data`,
// no GPU device.
//
// Covered here (players; enemy clauses deferred until enemies land):
//   VQ-B1 — every race with a model is rigged and names the min clip set
//   VQ-B2 — ≤ 64 joints, ≤ 16 MB on disk
//   VQ-B3 — socket bones exist in the rig
//   VQ-B4 — every clip named in a race .ron exists in the referenced .glb

use engine_renderer::mesh::{load_gltf_data, MeshData};
use engine_renderer::SocketConfig;
use std::path::{Path, PathBuf};
use vordar_game::player::class::{RaceLibrary, RaceModel};

const MAX_JOINTS: usize = 64; // engine palette cap per rig (VQ-B2)
const MAX_MODEL_BYTES: u64 = 16 * 1024 * 1024; // VQ-B2

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Every race id (RON filename stem) with its parsed definition's model.
fn race_models() -> Vec<(String, RaceModel, MeshData)> {
    let root = repo_root();
    let races_dir = root.join("content/races");
    assert!(races_dir.exists(), "content/races missing at {races_dir:?}");

    let mut races = RaceLibrary::new();
    races.load_dir(races_dir.to_str().unwrap());

    let mut out = Vec::new();
    for entry in std::fs::read_dir(&races_dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ron") {
            continue;
        }
        let id = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let model = races
            .model(&id)
            .unwrap_or_else(|| panic!("VQ-B1: race '{id}' parses but has no rigged model"))
            .clone();
        let asset = root.join(&model.asset);
        let data = load_gltf_data(asset.to_str().unwrap())
            .unwrap_or_else(|e| panic!("race '{id}': asset failed to parse: {e}"));
        out.push((id, model, data));
    }
    assert!(!out.is_empty(), "no races found in content/races");
    out
}

/// VQ-B1 + VQ-B4: rigged mesh, min clip set named, and every named clip
/// exists in the glb.
#[test]
fn race_clips_exist_in_gltf() {
    for (id, model, data) in race_models() {
        assert!(
            data.skeleton.is_some(),
            "VQ-B1: race '{id}' model {} has no skeleton",
            model.asset
        );

        let clip_names: Vec<&str> = data.clips.iter().map(|c| c.name.as_str()).collect();
        let required = [
            ("idle", &model.idle),
            ("walk", &model.walk),
            ("run", &model.run),
            ("attack", &model.attack),
            ("death", &model.death),
            ("hit", &model.hit),
        ];
        for (slot, name) in required {
            assert!(
                !name.is_empty(),
                "VQ-B1: race '{id}' names no {slot} clip (min set is idle/walk/run/attack/hit/death)"
            );
            assert!(
                clip_names.contains(&name.as_str()),
                "VQ-B4: race '{id}' {slot} clip '{name}' not in {} (has: {clip_names:?})",
                model.asset
            );
        }
    }
}

/// VQ-B2: joint count within the engine palette cap; asset within disk budget.
#[test]
fn race_models_within_budgets() {
    let root = repo_root();
    for (id, model, data) in race_models() {
        let joints = data.skeleton.as_ref().map(|s| s.joint_count()).unwrap_or(0);
        assert!(
            joints <= MAX_JOINTS,
            "VQ-B2: race '{id}' has {joints} joints (cap {MAX_JOINTS})"
        );

        let bytes = std::fs::metadata(root.join(&model.asset)).unwrap().len();
        assert!(
            bytes <= MAX_MODEL_BYTES,
            "VQ-B2: race '{id}' asset {} is {bytes} bytes (cap {MAX_MODEL_BYTES})",
            model.asset
        );
    }
}

/// Phase 6: every zone visual reference resolves — env HDRI, ground texture
/// set (diff/nor_gl/rough maps), and every prop glTF parses.
#[test]
fn zone_visual_refs_load() {
    let root = repo_root();
    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());
    for zone in &def.zones {
        let v = &zone.visuals;
        if let Some(env) = &v.env {
            assert!(root.join(env).exists(), "zone '{}': env '{}' missing", zone.name, env);
        }
        if let Some(g) = &v.ground {
            let dir = root.join(&g.texture_dir);
            for tag in ["diff", "nor_gl", "rough"] {
                let found = std::fs::read_dir(&dir)
                    .unwrap_or_else(|e| panic!("zone '{}': ground dir {dir:?}: {e}", zone.name))
                    .flatten()
                    .any(|f| f.file_name().to_string_lossy().contains(tag));
                assert!(found, "zone '{}': ground set lacks a *{tag}* map", zone.name);
            }
            assert!(g.tile > 0.0 && g.size > 0.0, "zone '{}': degenerate ground", zone.name);
        }
        for prop in &v.props {
            let path = root.join(&prop.model);
            load_gltf_data(path.to_str().unwrap())
                .unwrap_or_else(|e| panic!("zone '{}': prop failed to parse: {e}", zone.name));
        }
    }
}

/// VQ-B3: every socket bone the renderer attaches to exists in each rig.
/// (Socket names are the engine default set until Phase 5 makes them
/// data-driven per race.)
#[test]
fn race_models_expose_sockets() {
    let sockets = SocketConfig::default();
    for (id, model, data) in race_models() {
        let skeleton = data.skeleton.as_ref().expect("checked in race_clips_exist_in_gltf");
        for bone in &sockets.bones {
            assert!(
                skeleton.joints.iter().any(|j| &j.name == bone),
                "VQ-B3: race '{id}' rig {} lacks socket bone '{bone}'",
                model.asset
            );
        }
    }
}
