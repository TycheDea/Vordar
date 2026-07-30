// Chapter graybox geometry as offscreen-renderable mesh data — lets the
// review bins (zone_review, and one-off interior probes) draw a zone's
// chapter prefabs (buildings: `ShapeGroup` of `Cube` sub-shapes, the only
// shape type any chapter authors) through the same mesh pipeline as zones.ron
// props, without a live ECS world. A bin cannot reach a sibling bin
// (review.rs's rule), so this lives in the client lib both zone_review.rs
// and a probe bin can import.

use engine_core::components::{RenderShapeType, ShapeGroup};
use engine_core::prefab::PrefabDef;
use engine_renderer::mesh::{MaterialData, PrimitiveData};
use engine_renderer::MeshVertex;
use glam::{Mat4, Vec3};
use std::collections::HashMap;

/// Every `ShapeGroup` sub-shape across chapter `id`'s prefabs, placed at its
/// `chapter.ron` spawn positions, in world space. Panics on missing/malformed
/// chapter or prefab files — same "broken content is a bug" stance as
/// `vordar_game::chapter::load_chapter`. Non-`Cube` sub-shapes are skipped:
/// no chapter prefab authors one today (graybox stage).
pub fn load_chapter_prims(id: &str) -> Vec<PrimitiveData> {
    let dir = format!("content/chapters/{id}/prefabs");
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("chapter '{id}' prefab dir '{dir}' unreadable: {e}"));

    let mut shape_groups: HashMap<String, ShapeGroup> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ron") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("prefab '{}' unreadable: {e}", path.display()));
        let def: PrefabDef = ron::from_str(&text)
            .unwrap_or_else(|e| panic!("prefab '{}' parse error: {e}", path.display()));
        if let Some(raw) = def.components.get("ShapeGroup") {
            let group: ShapeGroup = raw.into_rust()
                .unwrap_or_else(|e| panic!("prefab '{stem}': ShapeGroup: {e}"));
            shape_groups.insert(stem.to_string(), group);
        }
    }

    let chapter = vordar_game::chapter::load_chapter(&format!("content/chapters/{id}/chapter.ron"));
    let mut prims = Vec::new();
    for spawn in &chapter.initial_spawns {
        let Some(group) = shape_groups.get(&spawn.prefab) else { continue };
        for &pos in &spawn.positions {
            for sub in &group.shapes {
                if !matches!(sub.shape, RenderShapeType::Cube) {
                    log::warn!("chapter '{id}' prefab '{}': non-Cube shape not drawn by the review probe", spawn.prefab);
                    continue;
                }
                // Chapter prefabs author a bare `Transform: ()` (identity
                // rotation/scale) — the spawn position is a pure translation,
                // matching instance_sync.rs's sub_model composition.
                let world = Mat4::from_translation(pos)
                    * Mat4::from_scale_rotation_translation(sub.scale, sub.rotation, sub.offset);
                prims.push(cube_prim(sub.color, world));
            }
        }
    }
    prims
}

/// One face of a local unit cube (corners at ±0.5): its outward normal and
/// the two axes spanning it, so the 4 corners and per-face UVs fall out
/// mechanically. Distinct vertices per face (24 total, not 8 shared) so each
/// face gets its own normal/tangent — required for correct shading on a box.
const CUBE_FACES: [(Vec3, Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::NEG_Z, Vec3::Y),
    (Vec3::NEG_X, Vec3::Z, Vec3::Y),
    (Vec3::Y, Vec3::X, Vec3::NEG_Z),
    (Vec3::NEG_Y, Vec3::X, Vec3::Z),
    (Vec3::Z, Vec3::X, Vec3::Y),
    (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
];

/// A solid-color box, built in local space (`world` already carries the
/// sub-shape's own offset/rotation/scale plus the spawn translation) — same
/// role as `review::ground_quad`, one level up in vertex count.
fn cube_prim(color: Vec3, world: Mat4) -> PrimitiveData {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, right, up) in CUBE_FACES {
        let base = vertices.len() as u32;
        let center = normal * 0.5;
        let corners = [
            center - right * 0.5 - up * 0.5,
            center + right * 0.5 - up * 0.5,
            center + right * 0.5 + up * 0.5,
            center - right * 0.5 + up * 0.5,
        ];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let world_normal = world.transform_vector3(normal).normalize_or_zero();
        let world_tangent = world.transform_vector3(right).normalize_or_zero();
        for (corner, uv) in corners.into_iter().zip(uvs) {
            vertices.push(MeshVertex {
                position: world.transform_point3(corner).to_array(),
                normal: world_normal.to_array(),
                uv,
                tangent: [world_tangent.x, world_tangent.y, world_tangent.z, 1.0],
            });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    PrimitiveData {
        vertices,
        indices,
        material: MaterialData {
            base_color_factor: [color.x, color.y, color.z, 1.0],
            roughness_factor: 0.9,
            metallic_factor: 0.0,
            ..Default::default()
        },
        skin: None,
    }
}
