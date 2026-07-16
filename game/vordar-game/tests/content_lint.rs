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
use vordar_game::player::race::{RaceLibrary, RaceModel};

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

/// VQ-E1: every authored ability has its three VFX beats — a cast beat in
/// content/vfx/<id>.ron, and (for projectiles) travel + impact on the
/// projectile prefab's VfxTrail. Scheduled abilities telegraph instead; their
/// impact fires at telegraph resolve (client code).
#[test]
fn ability_vfx_beats_exist() {
    use vordar_game::player::skills::AbilityEffect;

    let root = repo_root();
    let mut classes = vordar_game::class::ClassLibrary::new();
    classes.load_dir(root.join("content/classes").to_str().unwrap());
    let mut vfx = vordar_game::vfx::VfxLibrary::new();
    vfx.load_dir(root.join("content/vfx").to_str().unwrap());

    // Prefab dirs a projectile prefab may live in.
    let prefab_dirs = ["content/prefabs", "content/chapters/chapter01/prefabs", "content/chapters/chapter02/prefabs"];
    let find_prefab = |name: &str| -> Option<std::path::PathBuf> {
        prefab_dirs
            .iter()
            .map(|d| root.join(d).join(format!("{name}.ron")))
            .find(|p| p.exists())
    };

    #[derive(serde::Deserialize)]
    struct PrefabFile {
        components: std::collections::HashMap<String, ron::Value>,
    }

    for class_id in ["human", "ravager"] {
        for ability in classes.abilities_of(class_id) {
            let def = vfx.get(&ability.id).unwrap_or_else(|| {
                panic!("VQ-E1: ability '{}' has no content/vfx/{}.ron", ability.id, ability.id)
            });
            assert!(def.cast.is_some(), "VQ-E1: ability '{}' authors no cast beat", ability.id);

            if let AbilityEffect::Projectile { prefab, .. } = &ability.effect {
                let path = find_prefab(prefab)
                    .unwrap_or_else(|| panic!("ability '{}': prefab '{prefab}' not found", ability.id));
                let text = std::fs::read_to_string(&path).unwrap();
                let file: PrefabFile = ron::from_str(&text)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let trail = file.components.get("VfxTrail").unwrap_or_else(|| {
                    panic!("VQ-E1: projectile '{prefab}' has no VfxTrail (travel beat)")
                });
                let trail: vordar_game::vfx::VfxTrail = trail
                    .clone()
                    .into_rust()
                    .unwrap_or_else(|e| panic!("{prefab} VfxTrail: {e}"));
                assert!(
                    trail.impact.is_some(),
                    "VQ-E1: projectile '{prefab}' authors no impact beat"
                );
            }
        }
    }
}

/// Every zone visual reference resolves — env HDRI, ground texture set
/// (diff/nor_gl/rough maps), and every prop glTF parses.
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
/// (Socket names are the engine default set.)
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

/// Portal prefab emits light through data-driven spawn pipeline: RON round-trips
/// to a live PointLight component via ComponentRegistry + PrefabLibrary.
#[test]
fn portal_prefab_emits_light() {
    use engine_core::prefab::{ComponentRegistry, PrefabLibrary, register_core_components, spawn_prefab};
    use engine_core::traits::{Resources, SpawnContext};
    use engine_core::components::PointLight;
    use glam::Vec3;

    let root = repo_root();

    // Build registry + library with portal.ron
    let mut registry = ComponentRegistry::new();
    register_core_components(&mut registry);
    let mut library = PrefabLibrary::new();

    // Reuse the prefab_dirs convention from ability_vfx_beats_exist
    let prefab_dirs = ["content/prefabs", "content/chapters/chapter01/prefabs", "content/chapters/chapter02/prefabs"];
    for dir in &prefab_dirs {
        library.load_dir(root.join(dir).to_str().unwrap());
    }

    // Build world + resources + spawn context
    let mut world = hecs::World::new();
    let mut resources = Resources::new();
    resources.insert(registry);
    resources.insert(library);
    let mut ctx = SpawnContext { world: &mut world, resources: &mut resources };

    // Spawn portal and assert PointLight properties
    let entity = spawn_prefab("portal", Vec3::ZERO, &mut ctx)
        .expect("portal prefab must exist and spawn successfully");

    let light = ctx.world.get::<&PointLight>(entity)
        .expect("portal entity must have a PointLight component");

    assert!(light.radius > 0.0, "portal PointLight must have positive radius");
    assert!(light.color.z > light.color.x, "portal PointLight must have cool hue (blue > red)");
}

/// VQ-C5: character material maps within dimension cap (2048²).
#[test]
fn character_maps_within_dimension_cap() {
    const MAX_DIM: u32 = 2048;

    for (id, _model, data) in race_models() {
        for (prim_idx, prim) in data.primitives.iter().enumerate() {
            let mat = &prim.material;

            let slots = [
                ("base_color", &mat.base_color_image),
                ("normal", &mat.normal_image),
                ("metallic_roughness", &mat.metallic_roughness_image),
                ("emissive", &mat.emissive_image),
                ("occlusion", &mat.occlusion_image),
            ];

            for (slot_name, image) in &slots {
                if let Some(img) = image {
                    assert!(
                        img.width <= MAX_DIM && img.height <= MAX_DIM,
                        "VQ-C5: race '{}' primitive {} slot '{}' exceeds 2048² ({}×{})",
                        id, prim_idx, slot_name, img.width, img.height
                    );
                }
            }
        }
    }
}

/// VQ-C5: zone ground maps within dimension cap (4096²).
#[test]
fn ground_sets_within_dimension_cap() {
    use engine_renderer::mesh::load_image_rgba;

    const MAX_DIM: u32 = 4096;
    let root = repo_root();
    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());

    for zone in &def.zones {
        if let Some(g) = &zone.visuals.ground {
            let dir = root.join(&g.texture_dir);

            for tag in ["diff", "nor_gl", "rough"] {
                let path = std::fs::read_dir(&dir)
                    .unwrap_or_else(|e| panic!("zone '{}': ground dir {dir:?}: {e}", zone.name))
                    .flatten()
                    .find(|f| f.file_name().to_string_lossy().contains(tag))
                    .unwrap_or_else(|| panic!("zone '{}': ground set lacks a *{tag}* map", zone.name))
                    .path();

                let img = load_image_rgba(path.to_str().unwrap())
                    .unwrap_or_else(|e| panic!("zone '{}': failed to load {tag} map: {e}", zone.name));

                assert!(
                    img.width <= MAX_DIM && img.height <= MAX_DIM,
                    "VQ-C5: zone '{}' ground {tag} map exceeds 4096² ({}×{})",
                    zone.name, img.width, img.height
                );
            }
        }
    }
}

/// VQ-C5: total texture memory budget ≤ 1 GB (including mip chain overhead).
#[test]
fn total_texture_memory_within_budget() {
    use engine_renderer::mesh::load_image_rgba;

    const BUDGET_BYTES: u64 = 1_073_741_824; // 1 GB
    let root = repo_root();

    let mut total_bytes: u64 = 0;

    // (a) Race model image slots
    for (_id, _model, data) in race_models() {
        for prim in &data.primitives {
            let mat = &prim.material;
            let slots = [
                &mat.base_color_image,
                &mat.normal_image,
                &mat.metallic_roughness_image,
                &mat.emissive_image,
                &mat.occlusion_image,
            ];

            for image in slots {
                if let Some(img) = image {
                    // Estimate: RGBA8 + mip chain: w × h × 4 × 4/3
                    let bytes = (img.width as u64) * (img.height as u64) * 4 * 4 / 3;
                    total_bytes += bytes;
                }
            }
        }
    }

    // (b) Zone prop image slots and (c) zone ground sets
    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());

    for zone in &def.zones {
        // (c) Ground sets: diff/nor_gl/rough maps
        if let Some(g) = &zone.visuals.ground {
            let dir = root.join(&g.texture_dir);

            for tag in ["diff", "nor_gl", "rough"] {
                if let Some(path) = std::fs::read_dir(&dir)
                    .ok()
                    .and_then(|mut entries| entries.find_map(|f| {
                        let f = f.ok()?;
                        let name = f.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.contains(tag) {
                            Some(f.path())
                        } else {
                            None
                        }
                    })) {
                    if let Ok(img) = load_image_rgba(path.to_str().unwrap()) {
                        let bytes = (img.width as u64) * (img.height as u64) * 4 * 4 / 3;
                        total_bytes += bytes;
                    }
                }
            }
        }

        // (b) Prop image slots
        for prop in &zone.visuals.props {
            let path = root.join(&prop.model);
            if let Ok(data) = load_gltf_data(path.to_str().unwrap()) {
                for prim in &data.primitives {
                    let mat = &prim.material;
                    let slots = [
                        &mat.base_color_image,
                        &mat.normal_image,
                        &mat.metallic_roughness_image,
                        &mat.emissive_image,
                        &mat.occlusion_image,
                    ];

                    for image in slots {
                        if let Some(img) = image {
                            let bytes = (img.width as u64) * (img.height as u64) * 4 * 4 / 3;
                            total_bytes += bytes;
                        }
                    }
                }
            }
        }
    }

    let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
    assert!(
        total_bytes <= BUDGET_BYTES,
        "VQ-C5: total texture memory {:.1} MB exceeds 1 GB budget",
        total_mb
    );
}
