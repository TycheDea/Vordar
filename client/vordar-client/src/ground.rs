// Zone ground mesh: a heightmap-displaced grid with a tiling PBR texture set.
// Gameplay stays on the flat y = 0 plane (hitbox bottoms at GROUND_TOP_Y), so
// the surface is pinned flat inside the play radius and only rolls into gentle
// hills toward the horizon — scenery never clips feet.
//
// Pure mesh math — unit-tested without a GPU.

use engine_renderer::mesh::{load_image_rgba, ImageData, MaterialData, MeshData, PrimitiveData};
use engine_renderer::tangent::generate_tangents;
use engine_renderer::MeshVertex;

/// The walkable surface height: flush with every unit's hitbox bottom
/// (matches the old slab's top face).
pub const GROUND_TOP_Y: f32 = -0.5;
/// Perfectly flat inside this radius (the play area)...
const FLAT_RADIUS: f32 = 70.0;
/// ...ramping to full hill amplitude past this radius.
const HILL_RADIUS: f32 = 190.0;
const HILL_AMPLITUDE: f32 = 4.0;
/// Grid vertices per side.
const RESOLUTION: usize = 129;

/// Deterministic lattice hash → [0, 1).
fn hash(ix: i32, iz: i32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x85eb_ca6b) ^ (iz as u32).wrapping_mul(0xc2b2_ae35);
    h ^= h >> 13;
    h = h.wrapping_mul(0x27d4_eb2f);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65536.0
}

/// Smoothed bilinear value noise, period `scale` world units, range [0, 1).
fn value_noise(x: f32, z: f32, scale: f32) -> f32 {
    let (fx, fz) = (x / scale, z / scale);
    let (ix, iz) = (fx.floor() as i32, fz.floor() as i32);
    let (tx, tz) = (fx - fx.floor(), fz - fz.floor());
    // Smoothstep the lattice weights.
    let (sx, sz) = (tx * tx * (3.0 - 2.0 * tx), tz * tz * (3.0 - 2.0 * tz));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    lerp(
        lerp(hash(ix, iz), hash(ix + 1, iz), sx),
        lerp(hash(ix, iz + 1), hash(ix + 1, iz + 1), sx),
        sz,
    )
}

/// Ground height at world (x, z).
pub fn height(x: f32, z: f32) -> f32 {
    let r = (x * x + z * z).sqrt();
    let ramp = ((r - FLAT_RADIUS) / (HILL_RADIUS - FLAT_RADIUS)).clamp(0.0, 1.0);
    if ramp == 0.0 {
        return GROUND_TOP_Y;
    }
    // Two octaves of value noise, centred on 0.
    let n = value_noise(x, z, 60.0) + 0.4 * value_noise(x, z, 22.0);
    GROUND_TOP_Y + (n / 1.4 - 0.5) * 2.0 * HILL_AMPLITUDE * ramp
}

/// Build the ground mesh: `size`×`size` centred on the origin, UVs tiled
/// every `tile` world units, normals from the height field.
pub fn generate_ground(size: f32, tile: f32, material: MaterialData) -> MeshData {
    let n = RESOLUTION;
    let step = size / (n - 1) as f32;
    let half = size / 2.0;

    let mut positions = Vec::with_capacity(n * n);
    let mut normals = Vec::with_capacity(n * n);
    let mut uvs = Vec::with_capacity(n * n);
    for iz in 0..n {
        for ix in 0..n {
            let x = -half + ix as f32 * step;
            let z = -half + iz as f32 * step;
            positions.push([x, height(x, z), z]);
            // Central differences on the height field.
            let e = step.max(0.5);
            let dx = (height(x + e, z) - height(x - e, z)) / (2.0 * e);
            let dz = (height(x, z + e) - height(x, z - e)) / (2.0 * e);
            let nrm = glam::Vec3::new(-dx, 1.0, -dz).normalize();
            normals.push(nrm.to_array());
            uvs.push([x / tile, z / tile]);
        }
    }

    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for iz in 0..n - 1 {
        for ix in 0..n - 1 {
            let a = (iz * n + ix) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            // CCW seen from above (+Y), matching the front face convention.
            indices.extend_from_slice(&[a, d, b, a, c, d]);
        }
    }

    let tangents = generate_tangents(&positions, &normals, &uvs, &indices);
    let vertices = positions
        .iter()
        .zip(normals.iter())
        .zip(uvs.iter())
        .zip(tangents.iter())
        .map(|(((p, nrm), uv), t)| MeshVertex {
            position: *p,
            normal:   *nrm,
            uv:       *uv,
            tangent:  *t,
        })
        .collect();

    MeshData {
        primitives: vec![PrimitiveData { vertices, indices, material, skin: None }],
        skeleton:   None,
        clips:      Vec::new(),
    }
}

/// Load a Poly Haven-style texture set from `dir`: `*_diff_*` (sRGB albedo),
/// `*_nor_gl_*` (linear normal), `*_rough_*` (gray roughness → combined MR).
pub fn load_ground_material(dir: &str) -> Result<MaterialData, String> {
    let find = |tag: &str| -> Result<String, String> {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains(tag) {
                return Ok(entry.path().to_string_lossy().into_owned());
            }
        }
        Err(format!("{dir}: no *{tag}* map"))
    };

    let albedo = load_image_rgba(&find("diff")?)?;
    let normal = load_image_rgba(&find("nor_gl")?)?;
    let rough  = load_image_rgba(&find("rough")?)?;

    // glTF MR convention: g = roughness, b = metallic (0 — grounds aren't metal).
    let mr_pixels: Vec<u8> = rough
        .pixels
        .chunks_exact(4)
        .flat_map(|p| [0, p[0], 0, 255])
        .collect();
    let mr = ImageData { width: rough.width, height: rough.height, pixels: mr_pixels };

    Ok(MaterialData {
        base_color_image:         Some(albedo),
        normal_image:             Some(normal),
        metallic_roughness_image: Some(mr),
        metallic_factor:          1.0, // texture already encodes 0
        roughness_factor:         1.0,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_area_is_perfectly_flat() {
        for &(x, z) in &[(0.0, 0.0), (30.0, -20.0), (69.0, 0.0), (-50.0, 45.0)] {
            assert_eq!(height(x, z), GROUND_TOP_Y, "flat at ({x},{z})");
        }
    }

    #[test]
    fn horizon_rolls() {
        let heights: Vec<f32> = (0..40)
            .map(|i| height(200.0 + i as f32 * 7.0, i as f32 * 13.0))
            .collect();
        let min = heights.iter().cloned().fold(f32::MAX, f32::min);
        let max = heights.iter().cloned().fold(f32::MIN, f32::max);
        assert!(max - min > 1.0, "hills must vary: {min}..{max}");
        assert!(max <= GROUND_TOP_Y + HILL_AMPLITUDE + 1e-3);
        assert!(min >= GROUND_TOP_Y - HILL_AMPLITUDE - 1e-3);
    }

    #[test]
    fn mesh_uvs_tile_by_world_units() {
        let data = generate_ground(100.0, 5.0, MaterialData::default());
        let prim = &data.primitives[0];
        assert_eq!(prim.vertices.len(), RESOLUTION * RESOLUTION);
        for v in &prim.vertices {
            assert!((v.uv[0] - v.position[0] / 5.0).abs() < 1e-4);
            assert!((v.uv[1] - v.position[2] / 5.0).abs() < 1e-4);
        }
    }

    #[test]
    fn normals_are_unit_and_upward() {
        let data = generate_ground(400.0, 6.0, MaterialData::default());
        for v in &data.primitives[0].vertices {
            let nrm = glam::Vec3::from(v.normal);
            assert!((nrm.length() - 1.0).abs() < 1e-3);
            assert!(nrm.y > 0.5, "ground normals point up-ish, got {nrm}");
        }
    }

    #[test]
    fn triangles_wind_ccw_from_above() {
        let data = generate_ground(100.0, 5.0, MaterialData::default());
        let prim = &data.primitives[0];
        for tri in prim.indices.chunks_exact(3).take(50) {
            let p = |i: u32| glam::Vec3::from(prim.vertices[i as usize].position);
            let n = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            assert!(n.y > 0.0, "CCW from above");
        }
    }
}
