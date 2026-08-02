// Zone ground mesh: a heightmap-displaced grid with a tiling PBR texture set.
// Gameplay stays on the flat y = 0 plane (hitbox bottoms at GROUND_TOP_Y), so
// the surface is pinned flat inside the play radius and only rolls into gentle
// hills toward the horizon — scenery never clips feet.
//
// Pure mesh math — unit-tested without a GPU.

use engine_renderer::mesh::{load_image_rgba, ImageData, MaterialData, MeshData, PrimitiveData, SharedImage, TextureSource};
use engine_renderer::tangent::generate_tangents;
use engine_renderer::texture::load_dds_image;
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

/// A material swap over an axis-aligned rectangle of the grid, `min`/`max`
/// in world XZ. Must land on grid lines (`generate_ground`'s `step`) — the
/// assignment below tests quad centres, so an off-grid bound would silently
/// exclude the boundary quad's near half instead of raising an error.
pub struct GroundRegion {
    pub min:      (f32, f32),
    pub max:      (f32, f32),
    pub tile:     f32,
    pub material: MaterialData,
}

/// Build the ground mesh: `size`×`size` centred on the origin, UVs tiled
/// every `tile` world units, normals from the height field. `regions` layers
/// material overrides on top of `material`/`tile`; empty `regions` is the
/// single-primitive base case.
pub fn generate_ground(size: f32, tile: f32, material: MaterialData, regions: Vec<GroundRegion>) -> MeshData {
    let n = RESOLUTION;
    let step = size / (n - 1) as f32;
    let half = size / 2.0;

    if regions.is_empty() {
        return generate_uniform_ground(n, step, half, tile, material);
    }

    // Quad (ix, iz) belongs to the last region whose rectangle contains its
    // centre, else the base material. Centres sit half a step off any
    // snapped bound, so a boundary quad is never ambiguous.
    let region_of = |ix: usize, iz: usize| -> usize {
        let xc = -half + (ix as f32 + 0.5) * step;
        let zc = -half + (iz as f32 + 0.5) * step;
        regions
            .iter()
            .rposition(|r| xc >= r.min.0 && xc < r.max.0 && zc >= r.min.1 && zc < r.max.1)
            .map_or(0, |i| i + 1)
    };

    let group_count = regions.len() + 1;
    let mut positions: Vec<Vec<[f32; 3]>> = vec![Vec::new(); group_count];
    let mut normals: Vec<Vec<[f32; 3]>> = vec![Vec::new(); group_count];
    let mut uvs: Vec<Vec<[f32; 2]>> = vec![Vec::new(); group_count];
    let mut indices: Vec<Vec<u32>> = vec![Vec::new(); group_count];

    for iz in 0..n - 1 {
        for ix in 0..n - 1 {
            let g = region_of(ix, iz);
            let tile_g = if g == 0 { tile } else { regions[g - 1].tile };
            let base = positions[g].len() as u32;
            for &(cx, cz) in &[(ix, iz), (ix + 1, iz), (ix, iz + 1), (ix + 1, iz + 1)] {
                let x = -half + cx as f32 * step;
                let z = -half + cz as f32 * step;
                positions[g].push([x, height(x, z), z]);
                normals[g].push(vertex_normal(x, z, step));
                uvs[g].push([x / tile_g, z / tile_g]);
            }
            let (a, b, c, d) = (base, base + 1, base + 2, base + 3);
            indices[g].extend_from_slice(&[a, d, b, a, c, d]);
        }
    }

    // One material per group, base first, in the same order as `regions` —
    // moved out here (not cloned) since MaterialData holds Arc'd images with
    // no Clone impl.
    let materials: Vec<MaterialData> =
        std::iter::once(material).chain(regions.into_iter().map(|r| r.material)).collect();

    let mut primitives = Vec::with_capacity(group_count);
    for (g, mat) in materials.into_iter().enumerate() {
        if positions[g].is_empty() {
            continue;
        }
        let tangents = generate_tangents(&positions[g], &normals[g], &uvs[g], &indices[g]);
        let vertices = positions[g]
            .iter()
            .zip(normals[g].iter())
            .zip(uvs[g].iter())
            .zip(tangents.iter())
            .map(|(((p, nrm), uv), t)| MeshVertex { position: *p, normal: *nrm, uv: *uv, tangent: *t })
            .collect();
        primitives.push(PrimitiveData { vertices, indices: std::mem::take(&mut indices[g]), material: mat, skin: None });
    }

    MeshData { primitives, skeleton: None, clips: Vec::new() }
}

/// Central-difference normal at world (x, z), sampled `step` (clamped) apart.
fn vertex_normal(x: f32, z: f32, step: f32) -> [f32; 3] {
    let e = step.max(0.5);
    let dx = (height(x + e, z) - height(x - e, z)) / (2.0 * e);
    let dz = (height(x, z + e) - height(x, z - e)) / (2.0 * e);
    glam::Vec3::new(-dx, 1.0, -dz).normalize().to_array()
}

/// The no-region case: one primitive sharing a single `n`×`n` vertex grid.
fn generate_uniform_ground(n: usize, step: f32, half: f32, tile: f32, material: MaterialData) -> MeshData {
    let mut positions = Vec::with_capacity(n * n);
    let mut normals = Vec::with_capacity(n * n);
    let mut uvs = Vec::with_capacity(n * n);
    for iz in 0..n {
        for ix in 0..n {
            let x = -half + ix as f32 * step;
            let z = -half + iz as f32 * step;
            positions.push([x, height(x, z), z]);
            normals.push(vertex_normal(x, z, step));
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
    // Directory entry whose name contains `tag` and ends `.dds` — the baked
    // sidecar for that map slot, if the bake produced one.
    let find_dds = |tag: &str| -> Option<String> {
        std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name.contains(tag) && name.ends_with(".dds"))
                .then(|| entry.path().to_string_lossy().into_owned())
        })
    };
    // Directory entry whose name contains `tag` and is not a `.dds` — the
    // original JPG source for that map slot.
    let find_src = |tag: &str| -> Result<String, String> {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains(tag) && !name.ends_with(".dds") {
                return Ok(entry.path().to_string_lossy().into_owned());
            }
        }
        Err(format!("{dir}: no *{tag}* map"))
    };

    let base_color_image = match find_dds("diff") {
        Some(path) => TextureSource::Compressed(load_dds_image(&path)?),
        None => TextureSource::Rgba8(load_image_rgba(&find_src("diff")?)?),
    };
    let normal_image = match find_dds("nor_gl") {
        Some(path) => TextureSource::Compressed(load_dds_image(&path)?),
        None => TextureSource::Rgba8(load_image_rgba(&find_src("nor_gl")?)?),
    };
    let metallic_roughness_image = match find_dds("mr") {
        Some(path) => TextureSource::Compressed(load_dds_image(&path)?),
        None => {
            let rough = load_image_rgba(&find_src("rough")?)?;
            // glTF MR convention: g = roughness, b = metallic (0 — grounds aren't metal).
            let mr_pixels: Vec<u8> = rough
                .pixels
                .chunks_exact(4)
                .flat_map(|p| [0, p[0], 0, 255])
                .collect();
            TextureSource::Rgba8(ImageData { width: rough.width, height: rough.height, pixels: mr_pixels })
        }
    };

    Ok(MaterialData {
        base_color_image:         Some(SharedImage::new(base_color_image)),
        normal_image:             Some(SharedImage::new(normal_image)),
        metallic_roughness_image: Some(SharedImage::new(metallic_roughness_image)),
        // Ground sets are dielectric by declaration: the baked MR sidecar
        // replicates roughness into every colour channel (texconv only
        // parses uniform swizzle masks reliably), so its B channel is NOT
        // metallic-0 and the factor must kill it.
        metallic_factor:          0.0,
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
        let data = generate_ground(100.0, 5.0, MaterialData::default(), Vec::new());
        let prim = &data.primitives[0];
        assert_eq!(prim.vertices.len(), RESOLUTION * RESOLUTION);
        for v in &prim.vertices {
            assert!((v.uv[0] - v.position[0] / 5.0).abs() < 1e-4);
            assert!((v.uv[1] - v.position[2] / 5.0).abs() < 1e-4);
        }
    }

    #[test]
    fn normals_are_unit_and_upward() {
        let data = generate_ground(400.0, 6.0, MaterialData::default(), Vec::new());
        for v in &data.primitives[0].vertices {
            let nrm = glam::Vec3::from(v.normal);
            assert!((nrm.length() - 1.0).abs() < 1e-3);
            assert!(nrm.y > 0.5, "ground normals point up-ish, got {nrm}");
        }
    }

    #[test]
    fn triangles_wind_ccw_from_above() {
        let data = generate_ground(100.0, 5.0, MaterialData::default(), Vec::new());
        let prim = &data.primitives[0];
        for tri in prim.indices.chunks_exact(3).take(50) {
            let p = |i: u32| glam::Vec3::from(prim.vertices[i as usize].position);
            let n = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            assert!(n.y > 0.0, "CCW from above");
        }
    }

    #[test]
    fn region_quads_get_the_regions_material_and_tile() {
        let step = 100.0 / (RESOLUTION - 1) as f32;
        let region = GroundRegion {
            min:      (-step, -step),
            max:      (step, step),
            tile:     2.0,
            material: MaterialData { roughness_factor: 0.5, ..Default::default() },
        };
        let data = generate_ground(100.0, 5.0, MaterialData::default(), vec![region]);
        assert_eq!(data.primitives.len(), 2, "base + one region");

        let region_prim = data
            .primitives
            .iter()
            .find(|p| p.material.roughness_factor == 0.5)
            .expect("region primitive present");
        assert!(!region_prim.vertices.is_empty());
        for v in &region_prim.vertices {
            assert!((v.uv[0] - v.position[0] / 2.0).abs() < 1e-4, "region tile applied");
        }

        let base_prim = data.primitives.iter().find(|p| p.material.roughness_factor != 0.5).unwrap();
        for v in &base_prim.vertices {
            assert!((v.uv[0] - v.position[0] / 5.0).abs() < 1e-4, "base tile applied outside region");
        }
    }
}
