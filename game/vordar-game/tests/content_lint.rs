// Content lint — machine checks for the visual quality bar
// (docs/visual-quality.md). Pure CPU: glTF parsing via `load_gltf_data`,
// no GPU device.
//
// Covered here (players; enemy clauses deferred until enemies land):
//   VQ-B1 — every race with a model is rigged and names the min clip set
//   VQ-B2 — ≤ 64 joints, ≤ 16 MB on disk
//   VQ-B3 — socket bones exist in the rig
//   VQ-B4 — every clip named in a race .ron exists in the referenced .glb

use engine_renderer::mesh::{load_gltf_data, MeshData, TextureSource};
use engine_renderer::SocketConfig;
use std::path::{Path, PathBuf};
use vordar_game::player::race::{RaceLibrary, RaceModel};

const MAX_JOINTS: usize = 64; // engine palette cap per rig (VQ-B2)
const MAX_MODEL_BYTES: u64 = 16 * 1024 * 1024; // VQ-B2
const MAX_PROP_BYTES: u64 = 32 * 1024 * 1024; // VQ-B5

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn sha256_hex(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    format!("{:x}", Sha256::digest(&bytes))
}

#[derive(serde::Deserialize)]
struct GroundManifest {
    images: Vec<GroundManifestImage>,
}
#[derive(serde::Deserialize)]
struct GroundManifestImage {
    slot: String,
    file: String,
    source: String,
    sha256: String,
}

/// VQ-C5: every image in a ground-style sidecar set (diff/nor_gl/rough +
/// `manifest.json`) is present and hashes fresh against its source. Shared by
/// zone ground sets and the world-space detail tile — both use
/// `bake_textures.mjs ground`'s manifest shape.
fn check_ground_sidecars(dir: &Path) {
    let manifest_path = dir.join("manifest.json");
    let regen = format!("node scripts/asset-pipeline/bake_textures.mjs ground {}", dir.display());
    assert!(
        manifest_path.exists(),
        "VQ-C5: ground set {dir:?} has no sidecar manifest — regenerate: {regen}"
    );
    let text = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("{manifest_path:?}: {e}"));
    let manifest: GroundManifest =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{manifest_path:?}: {e}"));

    for image in &manifest.images {
        let source = dir.join(&image.source);
        assert_eq!(
            sha256_hex(&source),
            image.sha256,
            "VQ-C5: ground set {dir:?} source '{}' sidecar is stale — regenerate: {regen}",
            image.source
        );
        let dds = dir.join(&image.file);
        assert!(
            dds.exists(),
            "VQ-C5: ground set {dir:?} manifest lists sidecar '{}' but it's missing — regenerate: {regen}",
            image.file
        );
    }
    for required in ["diff", "normal"] {
        assert!(
            manifest.images.iter().any(|i| i.slot == required),
            "VQ-C5: ground set {dir:?} manifest lacks a required '{required}' sidecar — regenerate: {regen}"
        );
    }
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
            let ground_dirs = std::iter::once(&g.texture_dir).chain(g.regions.iter().map(|r| &r.texture_dir));
            for dir in ground_dirs {
                let dir = root.join(dir);
                for tag in ["diff", "nor_gl", "rough"] {
                    let found = std::fs::read_dir(&dir)
                        .unwrap_or_else(|e| panic!("zone '{}': ground dir {dir:?}: {e}", zone.name))
                        .flatten()
                        .any(|f| f.file_name().to_string_lossy().contains(tag));
                    assert!(found, "zone '{}': ground set lacks a *{tag}* map", zone.name);
                }
            }
            assert!(g.tile > 0.0 && g.size > 0.0, "zone '{}': degenerate ground", zone.name);
            for r in &g.regions {
                assert!(r.tile > 0.0, "zone '{}': degenerate ground region tile", zone.name);
                assert!(r.min.0 < r.max.0 && r.min.1 < r.max.1, "zone '{}': degenerate ground region bounds", zone.name);
            }
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
    assert!(light.color.x > light.color.z, "portal PointLight must have candle-gold (red > blue)");
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
                if let Some(TextureSource::Rgba8(img)) = image.as_deref().map(|i| &i.source) {
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
            let ground_dirs = std::iter::once(&g.texture_dir).chain(g.regions.iter().map(|r| &r.texture_dir));
            for dir in ground_dirs {
                let dir = root.join(dir);
                for tag in ["diff", "nor_gl", "rough"] {
                    let path = std::fs::read_dir(&dir)
                        .unwrap_or_else(|e| panic!("zone '{}': ground dir {dir:?}: {e}", zone.name))
                        .flatten()
                        .find(|f| f.file_name().to_string_lossy().contains(tag) && !f.file_name().to_string_lossy().ends_with(".dds"))
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
}

/// VQ-C5: total texture memory budget ≤ 1 GB — matching what the runtime
/// actually residents: one GPU texture per unique image (content key +
/// color space, mirroring the MeshStore texture cache), each model loaded
/// once however many placements reference it, and per image a bound DDS
/// sidecar's own byte size or the RGBA8 + mip-chain estimate.
#[test]
fn total_texture_memory_within_budget() {
    use engine_renderer::mesh::{load_image_rgba, SharedImage};
    use engine_renderer::texture::load_dds_image;

    const BUDGET_BYTES: u64 = 1_073_741_824; // 1 GB

    fn image_bytes(image: &SharedImage) -> u64 {
        match &image.source {
            TextureSource::Compressed(c) => c.data.len() as u64,
            // Estimate: RGBA8 + mip chain: w × h × 4 × 4/3
            TextureSource::Rgba8(img) => (img.width as u64) * (img.height as u64) * 4 * 4 / 3,
        }
    }

    let root = repo_root();
    let mut total_bytes: u64 = 0;
    let mut counted = std::collections::HashSet::new();
    let mut add_model = |total_bytes: &mut u64, data: &MeshData| {
        for prim in &data.primitives {
            let mat = &prim.material;
            // srgb rides the slot, as in `upload_mesh` — the same image in a
            // color slot and a data slot residents two GPU textures.
            for (image, srgb) in [
                (&mat.base_color_image, true),
                (&mat.normal_image, false),
                (&mat.metallic_roughness_image, false),
                (&mat.emissive_image, true),
                (&mat.occlusion_image, false),
            ] {
                if let Some(img) = image
                    && counted.insert((img.key, srgb)) {
                        *total_bytes += image_bytes(img);
                    }
            }
        }
    };

    // (a) Race model image slots
    for (_id, _model, data) in race_models() {
        add_model(&mut total_bytes, &data);
    }

    // (b) Zone prop image slots and (c) zone ground sets
    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());

    let mut loaded_models = std::collections::HashSet::new();
    let mut counted_ground_dirs = std::collections::HashSet::new();
    for zone in &def.zones {
        // (c) Ground sets: diff/nor_gl/mr maps — same sidecar-then-source
        // preference as `client::ground::load_ground_material`. Each unique
        // dir once — the same GPU texture residents regardless of how many
        // zones or regions reference it.
        if let Some(g) = &zone.visuals.ground {
            let ground_dirs = std::iter::once(&g.texture_dir).chain(g.regions.iter().map(|r| &r.texture_dir));
            for dir in ground_dirs {
                if !counted_ground_dirs.insert(dir.clone()) {
                    continue;
                }
                let dir = root.join(dir);
                let find = |tag: &str, dds_only: bool| -> Option<PathBuf> {
                    std::fs::read_dir(&dir).ok()?.flatten().find_map(|f| {
                        let name = f.file_name().to_string_lossy().into_owned();
                        (name.contains(tag) && name.ends_with(".dds") == dds_only).then(|| f.path())
                    })
                };

                for (dds_tag, src_tag) in [("diff", "diff"), ("nor_gl", "nor_gl"), ("mr", "rough")] {
                    let bytes = match find(dds_tag, true) {
                        Some(path) => {
                            load_dds_image(path.to_str().unwrap()).ok().map(|img| img.data.len() as u64)
                        }
                        None => find(src_tag, false)
                            .and_then(|path| load_image_rgba(path.to_str().unwrap()).ok())
                            .map(|img| (img.width as u64) * (img.height as u64) * 4 * 4 / 3),
                    };
                    total_bytes += bytes.unwrap_or(0);
                }
            }
        }

        // (b) Prop image slots, each unique model once (MeshStore path dedup)
        for prop in &zone.visuals.props {
            if !loaded_models.insert(prop.model.clone()) {
                continue;
            }
            let path = root.join(&prop.model);
            if let Ok(data) = load_gltf_data(path.to_str().unwrap()) {
                add_model(&mut total_bytes, &data);
            }
        }
    }

    let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
    println!("VQ-C5: total texture memory {total_mb:.1} MB (budget 1024 MB)");
    assert!(
        total_bytes <= BUDGET_BYTES,
        "VQ-C5: total texture memory {:.1} MB exceeds 1 GB budget",
        total_mb
    );
}

/// VQ-C5: every shipped material image has a sidecar that is present and
/// fresh. A re-export that isn't re-baked (e.g. a Mixamo clip merge into
/// human.glb) would silently shift glTF image indices — binding the wrong
/// texture to a slot, or falling back to RGBA8, with no other signal.
#[test]
fn material_textures_have_fresh_sidecars() {
    #[derive(serde::Deserialize)]
    struct GltfManifest {
        sha256: String,
        images: Vec<GltfManifestImage>,
    }
    #[derive(serde::Deserialize)]
    struct GltfManifestImage {
        file: String,
    }

    fn check_gltf_sidecars(asset: &Path) {
        let sidecar_dir = asset.with_extension("textures");
        let manifest_path = sidecar_dir.join("manifest.json");
        let regen =
            format!("node scripts/asset-pipeline/bake_textures.mjs gltf {}", asset.display());
        assert!(
            manifest_path.exists(),
            "VQ-C5: {} has no sidecar manifest at {manifest_path:?} — regenerate: {regen}",
            asset.display()
        );
        let text = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("{manifest_path:?}: {e}"));
        let manifest: GltfManifest =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{manifest_path:?}: {e}"));

        assert_eq!(
            sha256_hex(asset),
            manifest.sha256,
            "VQ-C5: {} sidecar manifest is stale (source hash changed) — regenerate: {regen}",
            asset.display()
        );
        for image in &manifest.images {
            let file = sidecar_dir.join(&image.file);
            assert!(
                file.exists(),
                "VQ-C5: {} manifest lists sidecar '{}' but it's missing — regenerate: {regen}",
                asset.display(),
                image.file
            );
        }
    }

    let root = repo_root();

    for (_id, model, _data) in race_models() {
        check_gltf_sidecars(&root.join(&model.asset));
    }

    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());
    let mut checked_assets = std::collections::HashSet::new();
    let mut checked_ground = std::collections::HashSet::new();
    for zone in &def.zones {
        for prop in &zone.visuals.props {
            if checked_assets.insert(prop.model.clone()) {
                check_gltf_sidecars(&root.join(&prop.model));
            }
        }
        if let Some(g) = &zone.visuals.ground {
            let ground_dirs = std::iter::once(&g.texture_dir).chain(g.regions.iter().map(|r| &r.texture_dir));
            for dir in ground_dirs {
                if checked_ground.insert(dir.clone()) {
                    check_ground_sidecars(&root.join(dir));
                }
            }
        }
    }
}

/// VQ-C5: the world-space detail tile (shared by every `vordar_detail`
/// material) has fresh sidecars, same contract as a zone ground set.
#[test]
fn detail_set_has_fresh_sidecars() {
    let root = repo_root();
    check_ground_sidecars(&root.join("content/textures/detail/limestone"));
}

/// The detail tile is a multiplicative overlay around 0.5 — any DC offset
/// tints every stone prop, and a leaning mean normal tilts every surface by a
/// constant. No constraint on mean Z: for a normal map carrying real grain,
/// `sqrt(1-x²-y²)` structurally pulls mean Z well under 255 (Jensen's
/// inequality) — a 255±3 band would demand a flat normal map.
#[test]
fn detail_tile_is_dc_neutral() {
    use engine_renderer::mesh::load_image_rgba;

    const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722]; // matches prep_detail_tile.py

    let root = repo_root();
    let dir = root.join("content/textures/detail/limestone");

    let albedo = load_image_rgba(dir.join("diff_2048.png").to_str().unwrap()).unwrap();
    let n = albedo.width as f64 * albedo.height as f64;
    let mean_luma: f64 = albedo
        .pixels
        .chunks_exact(4)
        .map(|px| LUMA[0] * px[0] as f64 + LUMA[1] * px[1] as f64 + LUMA[2] * px[2] as f64)
        .sum::<f64>()
        / n
        / 255.0;
    assert!(
        (mean_luma - 0.5).abs() <= 0.02,
        "detail albedo mean luminance {mean_luma:.4} outside 0.5 +/- 0.02"
    );

    let normal = load_image_rgba(dir.join("nor_gl_2048.png").to_str().unwrap()).unwrap();
    let n = normal.width as f64 * normal.height as f64;
    let mean_x: f64 = normal.pixels.chunks_exact(4).map(|px| px[0] as f64).sum::<f64>() / n;
    let mean_y: f64 = normal.pixels.chunks_exact(4).map(|px| px[1] as f64).sum::<f64>() / n;
    assert!((mean_x - 128.0).abs() <= 3.0, "detail normal mean X {mean_x:.2} outside 128 +/- 3");
    assert!((mean_y - 128.0).abs() <= 3.0, "detail normal mean Y {mean_y:.2} outside 128 +/- 3");
}

#[derive(serde::Deserialize)]
struct SurfaceClass {
    detail: bool,
    metallic: f32,
    roughness: f32,
}

#[derive(serde::Deserialize)]
struct AssetEntry {
    kind: String,
    /// None for `kind: "kit"` — kit models are multi-material, classed per
    /// glTF material name instead of per asset.
    surface_class: Option<String>,
}

fn load_registry<T: serde::de::DeserializeOwned>(path: &Path) -> std::collections::HashMap<String, T> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

/// A prop placement's registry key: its model's containing directory name.
fn prop_dir_name(model: &str) -> &str {
    Path::new(model)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| panic!("prop model '{model}' has no parent directory"))
}

/// Registration is one-directional: every zone prop placement must resolve to
/// a content/models/assets.json entry keyed by its model's directory name. An
/// assets.json entry with no placement is a declaration awaiting its
/// generation sweep, not a failure — only an unregistered placement is.
#[test]
fn prop_placements_are_registered() {
    let root = repo_root();
    let assets: std::collections::HashMap<String, AssetEntry> =
        load_registry(&root.join("content/models/assets.json"));

    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());
    for zone in &def.zones {
        for prop in &zone.visuals.props {
            let name = prop_dir_name(&prop.model);
            assert!(
                assets.contains_key(name),
                "zone '{}': prop '{}' has no content/models/assets.json entry for '{name}'",
                zone.name, prop.model
            );
        }
    }
}

/// A `kind: "kit"` model's per-material contract: every glTF material's
/// name keys a family entry in surface_classes.json, which authors its
/// metallic/roughness factors and detail flag; each family ships
/// base-color + normal + metallic-roughness maps (roughness lives in the
/// map, so the factor must stay 1.0 or it silently rescales it). Read from
/// the .gltf JSON directly — `MaterialData` carries no material names.
fn check_kit_materials(path: &Path, classes: &std::collections::HashMap<String, SurfaceClass>, clauses: &mut Vec<String>) {
    const TOLERANCE: f32 = 1e-6;

    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let json: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let materials = json["materials"].as_array().unwrap_or_else(|| panic!("{path:?}: no materials array"));

    for mat in materials {
        let Some(mat_name) = mat["name"].as_str() else {
            clauses.push("unnamed material".to_string());
            continue;
        };
        let Some(class) = classes.get(mat_name) else {
            clauses.push(format!("material '{mat_name}' not in surface_classes.json"));
            continue;
        };

        let pbr = &mat["pbrMetallicRoughness"];
        let metallic = pbr["metallicFactor"].as_f64().unwrap_or(1.0) as f32;
        if (metallic - class.metallic).abs() > TOLERANCE {
            clauses.push(format!("{mat_name}: metallic_factor {metallic} != {}", class.metallic));
        }
        let roughness = pbr["roughnessFactor"].as_f64().unwrap_or(1.0) as f32;
        if (roughness - class.roughness).abs() > TOLERANCE {
            clauses.push(format!("{mat_name}: roughness_factor {roughness} != {}", class.roughness));
        }
        let detail = mat["extras"]["vordar_detail"].as_bool().unwrap_or(false);
        if detail != class.detail {
            clauses.push(format!("{mat_name}: detail {detail} != {}", class.detail));
        }
        for (slot, value) in [
            ("baseColorTexture", &pbr["baseColorTexture"]),
            ("metallicRoughnessTexture", &pbr["metallicRoughnessTexture"]),
            ("normalTexture", &mat["normalTexture"]),
        ] {
            if value.is_null() {
                clauses.push(format!("{mat_name}: {slot} missing"));
            }
        }
    }
}

/// Every shipped prop material matches its assets.json surface_class
/// contract: metallic/roughness/detail as the class authors them, and the
/// map slots the pipeline is expected to have baked for that `kind`.
/// Kit models are checked per material name instead (`check_kit_materials`).
#[test]
fn prop_material_matches_surface_class() {
    const TOLERANCE: f32 = 1e-6;

    let root = repo_root();
    let assets: std::collections::HashMap<String, AssetEntry> =
        load_registry(&root.join("content/models/assets.json"));
    let classes: std::collections::HashMap<String, SurfaceClass> =
        load_registry(&root.join("content/models/surface_classes.json"));

    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());
    let mut checked = std::collections::HashSet::new();
    let mut violations = std::collections::BTreeMap::new();

    for zone in &def.zones {
        for prop in &zone.visuals.props {
            if !checked.insert(prop.model.clone()) {
                continue;
            }
            let name = prop_dir_name(&prop.model);
            let asset = assets
                .get(name)
                .unwrap_or_else(|| panic!("prop '{name}' has no assets.json entry"));

            if asset.kind == "kit" {
                let mut clauses = Vec::new();
                check_kit_materials(&root.join(&prop.model), &classes, &mut clauses);
                if !clauses.is_empty() {
                    violations.insert(name.to_string(), clauses);
                }
                continue;
            }

            let surface_class = asset.surface_class.as_deref().unwrap_or_else(|| {
                panic!("prop '{name}' (kind '{}') has no surface_class", asset.kind)
            });
            let class = classes.get(surface_class).unwrap_or_else(|| {
                panic!("prop '{name}': surface_class '{surface_class}' not in surface_classes.json")
            });
            let downloaded = asset.kind == "downloaded";

            let data = load_gltf_data(root.join(&prop.model).to_str().unwrap())
                .unwrap_or_else(|e| panic!("prop '{name}': failed to parse: {e}"));

            let mut clauses = Vec::new();
            for prim in &data.primitives {
                let mat = &prim.material;

                if (mat.metallic_factor - class.metallic).abs() > TOLERANCE {
                    clauses.push(format!("metallic_factor {} != {}", mat.metallic_factor, class.metallic));
                }

                // A downloaded roughness map multiplies this factor, so
                // anything but 1.0 would silently rescale the authored map.
                let want_roughness = if downloaded { 1.0 } else { class.roughness };
                if (mat.roughness_factor - want_roughness).abs() > TOLERANCE {
                    clauses.push(format!("roughness_factor {} != {want_roughness}", mat.roughness_factor));
                }

                let want_detail = if downloaded { false } else { class.detail };
                if mat.detail != want_detail {
                    clauses.push(format!("detail {} != {want_detail}", mat.detail));
                }

                if downloaded {
                    // The authored roughness lives in this map; losing it
                    // would read as a uniform 1.0 with no other signal.
                    if mat.metallic_roughness_image.is_none() {
                        clauses.push("metallic_roughness_image missing".to_string());
                    }
                } else {
                    if mat.metallic_roughness_image.is_some() {
                        clauses.push("metallic_roughness_image present, expected none".to_string());
                    }
                    if mat.occlusion_image.is_none() {
                        clauses.push("occlusion_image missing".to_string());
                    }
                }
            }

            if !clauses.is_empty() {
                violations.insert(name.to_string(), clauses);
            }
        }
    }

    assert!(
        violations.is_empty(),
        "prop material violates its surface_class contract:\n{}",
        violations
            .iter()
            .map(|(name, clauses)| format!("  {name}: {}", clauses.join("; ")))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Every `.glb` under `dir`, recursing through subdirectories.
fn find_glbs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{dir:?}: {e}")).flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_glbs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("glb") {
            out.push(path);
        }
    }
}

/// VQ-B5: every shipped prop glb is within the prop disk budget.
#[test]
fn prop_models_within_byte_budget() {
    let root = repo_root();
    let props_dir = root.join("content/models/props");
    assert!(props_dir.exists(), "content/models/props missing at {props_dir:?}");

    let mut glbs = Vec::new();
    find_glbs(&props_dir, &mut glbs);
    assert!(!glbs.is_empty(), "no prop glbs found under {props_dir:?}");

    for path in glbs {
        let bytes = std::fs::metadata(&path).unwrap().len();
        assert!(
            bytes <= MAX_PROP_BYTES,
            "VQ-B5: prop {} is {bytes} bytes (cap {MAX_PROP_BYTES})",
            path.strip_prefix(&root).unwrap_or(&path).display()
        );
    }
}

// ── D5: town layout <-> collision cross-check (P2.5, tasks/town/p24-layout.md §5) ──

#[derive(serde::Deserialize)]
struct FootprintManifest {
    types: std::collections::HashMap<String, FootprintType>,
}

#[derive(serde::Deserialize)]
struct FootprintType {
    size: (f32, f32),
    tolerance: f32,
    #[serde(default)]
    members: Vec<FootprintMember>,
}

/// One collision piece of a kit type: `prefab` is its chapter03 prefab id,
/// `offsets` its position(s) in the type's local frame (model frame at yaw
/// 0) relative to the zone prop. A type with no members in `footprints.ron`
/// is a single piece under its own name, at offset (0,0). A member carries no
/// size of its own — the chapel and gate ship as one mesh each, so there is
/// no independent per-piece measurement to check a member's collision box
/// against (a size copied from the very prefab it would be checked against
/// is circular, not a check). Check (b) instead unions the placed members'
/// actual hitboxes and compares THAT to the type's own measured `size`.
#[derive(Clone, serde::Deserialize)]
struct FootprintMember {
    prefab: String,
    offsets: Vec<(f32, f32)>,
}

fn load_footprints(root: &Path) -> FootprintManifest {
    let path = root.join("content/chapters/chapter03/footprints.ron");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    ron::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

/// Every chapter03 prefab's Hitbox half-extents, keyed by prefab id (file
/// stem). Wall/gate prefabs are authored directly in their placed
/// orientation (no in-game rotation applies to a Hitbox — it's always
/// axis-aligned), so these are already world-space.
fn chapter03_hitboxes(root: &Path) -> std::collections::HashMap<String, glam::Vec3> {
    use engine_core::components::{CollisionShape, Hitbox};
    use engine_core::prefab::{register_core_components, spawn_prefab, ComponentRegistry, PrefabLibrary};
    use engine_core::traits::{Resources, SpawnContext};

    let mut registry = ComponentRegistry::new();
    register_core_components(&mut registry);
    let mut library = PrefabLibrary::new();
    library.load_dir(root.join("content/chapters/chapter03/prefabs").to_str().unwrap());
    let names = library.names();

    let mut world = hecs::World::new();
    let mut resources = Resources::new();
    resources.insert(registry);
    resources.insert(library);
    let mut ctx = SpawnContext { world: &mut world, resources: &mut resources };

    let mut out = std::collections::HashMap::new();
    for name in names {
        let entity = spawn_prefab(&name, glam::Vec3::ZERO, &mut ctx)
            .unwrap_or_else(|e| panic!("chapter03 prefab '{name}': {e}"));
        let hitbox = ctx.world.get::<&Hitbox>(entity)
            .unwrap_or_else(|_| panic!("chapter03 prefab '{name}' has no Hitbox"));
        let CollisionShape::Aabb { half_extents } = &hitbox.shape else {
            panic!("chapter03 prefab '{name}' Hitbox is not an Aabb");
        };
        out.insert(name, *half_extents);
    }
    out
}

/// Rotates a local-frame XZ vector (a member offset or size) into world XZ by
/// the same `Quat::from_rotation_y` the renderer applies to placed kit models
/// (`presentation.rs`) — reusing the engine's own rotation instead of
/// re-deriving its sign convention.
fn rotate_xz(v: (f32, f32), yaw_deg: f32) -> (f32, f32) {
    let r = glam::Quat::from_rotation_y(yaw_deg.to_radians()) * glam::Vec3::new(v.0, 0.0, v.1);
    (r.x, r.z)
}

/// D5 (a)+(b): every kit-type start-zone prop in `zones.ron` has an exact-XZ
/// (±0.01) match among chapter03's collision spawns, and every chapter03
/// spawn maps back to exactly one prop — bijection, no orphans either side.
/// A prop's `footprints.ron` entry lists one member per distinct collision
/// piece, each with its own local-frame offset(s) rotated by the prop's
/// placement yaw (casa_corner's wing, chapel's 7 pieces, gate_arch's jambs +
/// head); a type with no members list is a single piece.
///
/// Check (b) unions the matched pieces' actual (already-authored) Hitboxes in
/// world space and compares that union to the type's own measured `size`
/// (rotated the same way), within the entry's tolerance — independent
/// ground truth on one side, real placed geometry on the other. A simple
/// (single-piece) type's "union" is just its own box, so this subsumes the
/// plain per-spawn check those types had before composites existed.
///
/// Dressing props (crucero, cypress, gravestones, ...) carry no collision and
/// are skipped via `assets.json`'s `kind` field — the same "kit" flag
/// `prop_material_matches_surface_class` already keys off, not a name list.
#[test]
fn town_prop_collision_matches_footprints() {
    let root = repo_root();
    let assets: std::collections::HashMap<String, AssetEntry> =
        load_registry(&root.join("content/models/assets.json"));
    let footprints = load_footprints(&root);
    let hitboxes = chapter03_hitboxes(&root);
    let chapter = vordar_game::chapter::load_chapter(
        root.join("content/chapters/chapter03/chapter.ron").to_str().unwrap(),
    );

    struct Spawn {
        prefab: String,
        xz: (f32, f32),
        claimed: bool,
    }
    let mut spawns: Vec<Spawn> = chapter
        .initial_spawns
        .iter()
        .flat_map(|s| {
            s.positions.iter().map(move |p| Spawn { prefab: s.prefab.clone(), xz: (p.x, p.z), claimed: false })
        })
        .collect();

    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());
    let start = def.zones.iter().find(|z| z.name == "start").expect("zones.ron has a 'start' zone");

    let mut missing = Vec::new();
    let mut size_mismatches = Vec::new();
    for prop in &start.visuals.props {
        let type_name = prop_dir_name(&prop.model);
        if assets.get(type_name).map(|a| a.kind.as_str()) != Some("kit") {
            continue; // dressing prop — no collision expected
        }
        let footprint = footprints.types.get(type_name).unwrap_or_else(|| {
            panic!("D5: kit prop '{type_name}' has no content/chapters/chapter03/footprints.ron entry")
        });
        let members: Vec<FootprintMember> = if footprint.members.is_empty() {
            vec![FootprintMember { prefab: type_name.to_string(), offsets: vec![(0.0, 0.0)] }]
        } else {
            footprint.members.clone()
        };

        // World-space (x_min, x_max, z_min, z_max) of every piece this prop
        // claims, folded into a union below — the real, placed geometry half
        // of check (b).
        let mut piece_bounds: Vec<(f32, f32, f32, f32)> = Vec::new();

        for member in &members {
            for &local_offset in &member.offsets {
                let (dx, dz) = rotate_xz(local_offset, prop.yaw);
                let expect = (prop.pos.x + dx, prop.pos.z + dz);
                let found = spawns.iter_mut().find(|s| {
                    !s.claimed
                        && s.prefab == member.prefab
                        && (s.xz.0 - expect.0).abs() <= 0.01
                        && (s.xz.1 - expect.1).abs() <= 0.01
                });
                match found {
                    Some(s) => {
                        s.claimed = true;
                        let half = hitboxes
                            .get(&member.prefab)
                            .unwrap_or_else(|| panic!("D5: chapter03 has no prefab '{}'", member.prefab));
                        piece_bounds.push((s.xz.0 - half.x, s.xz.0 + half.x, s.xz.1 - half.z, s.xz.1 + half.z));
                    }
                    None => missing.push(format!(
                        "{type_name} @ ({:.1},{:.1}) yaw {}: no '{}' spawn at ({:.2},{:.2})",
                        prop.pos.x, prop.pos.z, prop.yaw, member.prefab, expect.0, expect.1
                    )),
                }
            }
        }

        if piece_bounds.is_empty() {
            continue; // every member already reported missing above
        }
        let x_min = piece_bounds.iter().map(|b| b.0).fold(f32::INFINITY, f32::min);
        let x_max = piece_bounds.iter().map(|b| b.1).fold(f32::NEG_INFINITY, f32::max);
        let z_min = piece_bounds.iter().map(|b| b.2).fold(f32::INFINITY, f32::min);
        let z_max = piece_bounds.iter().map(|b| b.3).fold(f32::NEG_INFINITY, f32::max);
        let (union_x, union_z) = (x_max - x_min, z_max - z_min);
        let (expect_x, expect_z) = {
            let (ex, ez) = rotate_xz(footprint.size, prop.yaw);
            (ex.abs(), ez.abs())
        };
        if (union_x - expect_x).abs() > footprint.tolerance || (union_z - expect_z).abs() > footprint.tolerance {
            size_mismatches.push(format!(
                "{type_name} @ ({:.1},{:.1}): collision coverage {union_x:.2}x{union_z:.2} vs measured footprint {expect_x:.2}x{expect_z:.2} (tol {})",
                prop.pos.x, prop.pos.z, footprint.tolerance
            ));
        }
    }

    let orphans: Vec<String> = spawns
        .iter()
        .filter(|s| !s.claimed)
        .map(|s| format!("{} @ ({:.2},{:.2})", s.prefab, s.xz.0, s.xz.1))
        .collect();

    assert!(
        missing.is_empty(),
        "D5: zone prop has no matching chapter03 collision spawn:\n  {}",
        missing.join("\n  ")
    );
    assert!(
        orphans.is_empty(),
        "D5: chapter03 collision spawn has no matching zone prop:\n  {}",
        orphans.join("\n  ")
    );
    assert!(
        size_mismatches.is_empty(),
        "D5: collision coverage diverges from footprints.ron's measured size:\n  {}",
        size_mismatches.join("\n  ")
    );
}

/// A player's vertical extent above the ground plane (zone_review's
/// EYE_HEIGHT is 1.6; this leaves headroom without reaching the 3.6 m
/// gate/lintel springline any collision piece is spawned aloft above). Shared
/// by the two D5 checks that must agree on what "ground level" means.
const PLAYER_HEIGHT: f32 = 2.0;

/// D5 (b') — the independent witness under check (b). `footprints.ron`'s
/// `size` is hand-authored, and check (b) grades the collision union against
/// it; for a single-piece type that union IS the one hitbox the size was
/// authored from, so the pair can agree while both drift from the model. This
/// re-measures `size` against the installed glTF, which no one authored: the
/// visual mesh may only exceed the footprint (quoins, plinths, the apse fan)
/// and only by the entry's stated `tolerance`, and may never fall short of it
/// — a collision box wider than its building is a wall you bump into in the
/// open. Measured over the same [0, 2.0] m player band `town_prop_clearances`
/// uses, since `size` is a ground-level footprint: roof eaves overhang the
/// walls by design and a player never reaches them.
#[test]
fn town_footprint_sizes_match_installed_gltf() {
    let root = repo_root();
    let assets: std::collections::HashMap<String, AssetEntry> =
        load_registry(&root.join("content/models/assets.json"));
    let footprints = load_footprints(&root);
    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());
    let start = def.zones.iter().find(|z| z.name == "start").expect("zones.ron has a 'start' zone");

    let mut seen = std::collections::HashSet::new();
    let mut faults = Vec::new();
    for prop in &start.visuals.props {
        let type_name = prop_dir_name(&prop.model);
        if assets.get(type_name).map(|a| a.kind.as_str()) != Some("kit") {
            continue;
        }
        if !seen.insert(type_name.to_string()) {
            continue;
        }
        let footprint = &footprints.types[type_name];
        let data = load_gltf_data(root.join(&prop.model).to_str().unwrap())
            .unwrap_or_else(|e| panic!("kit type '{type_name}': failed to parse: {e}"));
        let base = data
            .primitives
            .iter()
            .flat_map(|p| p.vertices.iter())
            .fold(f32::INFINITY, |m, v| m.min(v.position[1]));
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for prim in &data.primitives {
            for v in prim.vertices.iter().filter(|v| v.position[1] <= base + PLAYER_HEIGHT) {
                for (axis, i) in [(0usize, 0usize), (1, 2)] {
                    lo[axis] = lo[axis].min(v.position[i]);
                    hi[axis] = hi[axis].max(v.position[i]);
                }
            }
        }
        let mesh = (hi[0] - lo[0], hi[1] - lo[1]);
        // The manifest carries centimetre values, so a centimetre of apparent
        // shortfall is its own quantization rather than drift.
        const QUANTIZATION: f32 = 0.01;
        let over = (mesh.0 - footprint.size.0, mesh.1 - footprint.size.1);
        let bad = |o: f32| o < -QUANTIZATION || o > footprint.tolerance;
        if bad(over.0) || bad(over.1) {
            faults.push(format!(
                "{type_name}: mesh {:.3}x{:.3} vs footprint {:.3}x{:.3} — overhang {:+.3}/{:+.3} (tol {})",
                mesh.0, mesh.1, footprint.size.0, footprint.size.1, over.0, over.1, footprint.tolerance
            ));
        }
    }
    assert!(!seen.is_empty(), "D5: no kit props found in the start zone");
    assert!(
        faults.is_empty(),
        "D5: footprints.ron size does not bound the installed glTF:\n  {}",
        faults.join("\n  ")
    );
}

/// D5 (c): every kit-type start-zone prop's yaw is one of the four facings a
/// Hitbox Aabb (never rotated) can actually match.
#[test]
fn town_prop_yaw_is_axis_aligned() {
    let root = repo_root();
    let assets: std::collections::HashMap<String, AssetEntry> =
        load_registry(&root.join("content/models/assets.json"));
    let def = vordar_game::zones::load_zones(root.join("content/zones/zones.ron").to_str().unwrap());
    let start = def.zones.iter().find(|z| z.name == "start").expect("zones.ron has a 'start' zone");

    for prop in &start.visuals.props {
        let type_name = prop_dir_name(&prop.model);
        if assets.get(type_name).map(|a| a.kind.as_str()) != Some("kit") {
            continue;
        }
        let yaw = prop.yaw.rem_euclid(360.0);
        let axis_aligned = [0.0, 90.0, 180.0, 270.0].iter().any(|v: &f32| (yaw - v).abs() < 1e-3);
        assert!(
            axis_aligned,
            "D5: kit prop '{type_name}' at ({},{}) has non-axis-aligned yaw {}",
            prop.pos.x, prop.pos.z, prop.yaw
        );
    }
}

/// D5 (d): the spawn ring (r=3 at the origin) and the east portal corridor
/// (x in [0,22], the road to the gate) stay clear of every chapter03 solid a
/// player can actually walk into. The corridor's half-width (1.5) sits just
/// under the gate's own 3.2 m opening (jamb inner faces at |z| = 1.6) — the
/// jambs bound the passage by design and must not themselves read as
/// blocking it. Solids spawned aloft over a passage (gate_head at y=4.6,
/// chapel_lintel/roof) are excluded the same way: their own prefab comments
/// say the passage below stays open, so a check against ground-level
/// clearance must look at height too, not XZ alone.
#[test]
fn town_layout_clearances() {
    const RING_RADIUS: f32 = 3.0;
    const CORRIDOR_X: (f32, f32) = (0.0, 22.0);
    const CORRIDOR_HALF_Z: f32 = 1.5;

    let root = repo_root();
    let chapter = vordar_game::chapter::load_chapter(
        root.join("content/chapters/chapter03/chapter.ron").to_str().unwrap(),
    );
    let hitboxes = chapter03_hitboxes(&root);

    let mut ring_hits = Vec::new();
    let mut corridor_hits = Vec::new();
    for spawn in &chapter.initial_spawns {
        let half = hitboxes
            .get(&spawn.prefab)
            .unwrap_or_else(|| panic!("D5: chapter03 has no prefab '{}'", spawn.prefab));
        for pos in &spawn.positions {
            if pos.y - half.y >= PLAYER_HEIGHT || pos.y + half.y <= 0.0 {
                continue; // spawned aloft — doesn't block ground-level movement
            }
            let closest_x = 0.0_f32.clamp(pos.x - half.x, pos.x + half.x);
            let closest_z = 0.0_f32.clamp(pos.z - half.z, pos.z + half.z);
            let ring_dist = (closest_x * closest_x + closest_z * closest_z).sqrt();
            if ring_dist < RING_RADIUS {
                ring_hits.push(format!(
                    "{} @ ({:.2},{:.2}): {:.2} m from spawn (ring r={RING_RADIUS})",
                    spawn.prefab, pos.x, pos.z, ring_dist
                ));
            }

            let overlaps_x = (pos.x - half.x) < CORRIDOR_X.1 && (pos.x + half.x) > CORRIDOR_X.0;
            let overlaps_z = (pos.z - half.z) < CORRIDOR_HALF_Z && (pos.z + half.z) > -CORRIDOR_HALF_Z;
            if overlaps_x && overlaps_z {
                corridor_hits.push(format!("{} @ ({:.2},{:.2})", spawn.prefab, pos.x, pos.z));
            }
        }
    }

    assert!(
        ring_hits.is_empty(),
        "D5: spawn-ring r={RING_RADIUS} intrusion:\n  {}",
        ring_hits.join("\n  ")
    );
    assert!(
        corridor_hits.is_empty(),
        "D5: portal-corridor intrusion (x in {CORRIDOR_X:?}, |z| < {CORRIDOR_HALF_Z}):\n  {}",
        corridor_hits.join("\n  ")
    );
}

/// D5 (e): every town solid lies within r=55 of the origin (the play-radius
/// clamp is 65, `game/vordar-game/src/motion/movement.rs`).
#[test]
fn town_solids_within_play_radius() {
    const MAX_RADIUS: f32 = 55.0;

    let root = repo_root();
    let chapter = vordar_game::chapter::load_chapter(
        root.join("content/chapters/chapter03/chapter.ron").to_str().unwrap(),
    );
    let hitboxes = chapter03_hitboxes(&root);

    let mut worst: Option<(String, f32)> = None;
    for spawn in &chapter.initial_spawns {
        let half = hitboxes
            .get(&spawn.prefab)
            .unwrap_or_else(|| panic!("D5: chapter03 has no prefab '{}'", spawn.prefab));
        for pos in &spawn.positions {
            let r = ((pos.x.abs() + half.x).powi(2) + (pos.z.abs() + half.z).powi(2)).sqrt();
            if worst.as_ref().is_none_or(|(_, w)| r > *w) {
                worst = Some((spawn.prefab.clone(), r));
            }
            assert!(
                r <= MAX_RADIUS,
                "D5: '{}' solid at ({:.1},{:.1}) reaches r={:.2}, beyond the {MAX_RADIUS} play radius",
                spawn.prefab, pos.x, pos.z, r
            );
        }
    }
    let (name, r) = worst.expect("chapter03 has no solids");
    println!("D5: farthest town solid corner is '{name}' at r={r:.2} (cap {MAX_RADIUS})");
}

/// Every Scheduled/Leap ability's cast time is at least 2 resolve slices
/// (`server/vordar-server/src/net/mechanics.rs`: `MechanicResolveSystem`
/// self-gates to the SNAPSHOT_HZ cadence), bounding the resolve-time
/// quantization error at 50%.
#[test]
fn scheduled_cast_times_clear_resolve_slice() {
    use vordar_game::player::skills::AbilityEffect;

    let slice_micros = (1_000_000.0 / vordar_protocol::SNAPSHOT_HZ) as u64;
    let min_cast_micros = 2 * slice_micros;

    let root = repo_root();
    let mut classes = vordar_game::class::ClassLibrary::new();
    classes.load_dir(root.join("content/classes").to_str().unwrap());

    for class_id in ["human", "ravager"] {
        for ability in classes.abilities_of(class_id) {
            let cast_micros = match &ability.effect {
                AbilityEffect::Scheduled { cast_micros, .. } => *cast_micros,
                AbilityEffect::Leap { cast_micros, .. } => *cast_micros,
                AbilityEffect::Projectile { .. } => continue,
            };
            assert!(
                cast_micros >= min_cast_micros,
                "ability '{}': cast_micros={cast_micros} is below the resolve-slice floor \
                 of {min_cast_micros} (2x the {slice_micros}us SNAPSHOT_HZ slice) — quantization \
                 error would exceed 50%",
                ability.id
            );
        }
    }
}
