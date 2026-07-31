// zone_review: headless offscreen ship-gate render for one zone's live prop
// dressing (content/zones/zones.ron) — a wide establishing shot, mid shots
// per proximity cluster, one close-up per distinct prop model (with the
// player model for scale), and interior shots inside the start town's
// chapel prop, all under the zone's real HDRI/fog/sun. Built because
// turntable-style renders (fixed full-prop framing, no player, no
// destination lighting) cleared five props that then failed the in-game
// feel-check on exactly the axes this tool now captures — see
// tasks/lessons/2026-07-23-review-in-engine-at-gameplay-framing.md. Mirrors
// gear_render.rs/turntable.rs's offscreen-harness pattern one crate over.
//
// `--visuals-override <path.ron>` replaces the zone's authored lighting with
// a hand-written RON file so candidate looks can be rendered without editing
// zones.ron — see `LightingOverride` for the accepted shape (every field
// optional; ground/fog/props/env stay as the zone authored them).

use engine_renderer::anim::LocalTransform;
use engine_renderer::mesh::{load_gltf_data, MeshData, PrimitiveData};
use engine_renderer::offscreen::{OffscreenRenderer, TestLight};
use engine_renderer::review;
use glam::{Mat4, Quat, Vec3};
use image::RgbaImage;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::process::exit;
use vordar_client::ground::{generate_ground, height as ground_height, load_ground_material, GROUND_TOP_Y};
use vordar_client::presentation::DETAIL_TEXTURE_DIR;
use vordar_game::zones::{load_zones, resolve_sun_color, resolve_sun_dir, ZoneVisuals};

const ZONES_PATH:  &str = "content/zones/zones.ron";
const PLAYER_GLB:  &str = "content/models/human.glb";
// Matches ZoneDressingSystem's fallback (presentation.rs) when a zone
// authors no `env`.
const DEFAULT_HDRI: &str = "content/textures/env/castilian_plateau_overcast_2k.hdr";
const THREE_QUARTER_YAW: f32 = std::f32::consts::TAU / 8.0;

/// Props farther than this from the origin still draw (horizon landmarks)
/// but are excluded from the wide shot's framing centroid — otherwise the
/// establishing shot centers on the whole map instead of the dressed area.
const WIDE_RADIUS: f32 = 80.0;
/// XZ proximity threshold for grouping placements into one mid-shot cluster.
const CLUSTER_RADIUS: f32 = 20.0;
/// Matches `Camera::new`'s own default 3rd-person pitch — the live game's
/// actual follow-camera framing. The mid shot aims at a cluster's centroid
/// from a fixed distance (via `set_camera_lookat`); the wide shot instead
/// sphere-fits the full scene bounds (`set_camera_turntable`) so it scales
/// from a handful of small props up to tens-of-metres chapter buildings
/// without cropping either.
const GAMEPLAY_PITCH: f32 = 0.8;
/// Tighter than a close-up, wide enough to hold one prop cluster.
const MID_RADIUS: f32 = 14.0;

/// `Camera::recompute_eye`'s orbit formula: eye at `radius`/`pitch`/`azimuth`
/// from `target`, floored at ground level like the real camera (`MIN_EYE_Y`).
fn orbit_eye(target: Vec3, radius: f32, azimuth: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        target.x + radius * azimuth.cos() * pitch.cos(),
        (target.y + radius * pitch.sin()).max(0.0),
        target.z + radius * azimuth.sin() * pitch.cos(),
    )
}
/// `orbit_eye` at the wide/mid shots' fixed three-quarter azimuth/pitch.
fn gameplay_eye(target: Vec3, radius: f32) -> Vec3 {
    orbit_eye(target, radius, THREE_QUARTER_YAW, GAMEPLAY_PITCH)
}
/// Average human eye height above the ground — the close-up camera's height.
const EYE_HEIGHT: f32 = 1.6;
/// Camera-to-target distance for close-ups: the task's ~2-2.5 m
/// gameplay-inspection distance (close enough to read texel density).
const CLOSE_DISTANCE: f32 = 2.3;
/// Lateral clearance from the prop's local silhouette to where the scale
/// player stands — floored so slender props (crosses, columns) don't put
/// the player inside them, capped so wide ones (arches, rock faces) can't
/// push the player outside the close-up's frame.
const PLAYER_OFFSET_MIN: f32 = 1.0;
const PLAYER_OFFSET_MAX: f32 = 1.3;
/// Contact-sheet thumbnail cell size — tiling zone_review's ~20-30 full-res
/// frames at source resolution (gear_render.rs's approach) would make a
/// sheet too large to eyeball or view.
const THUMB: (u32, u32) = (480, 270);

/// The start town's chapel nave anchor: the kit chapel prop at (-30, -29),
/// nave interior x∈[-38,-22], z∈[-32.5,-25.5]. Chapter03-specific, not
/// zone-agnostic, so the interior shot only runs when the reviewed zone
/// actually runs chapter03.
const NAVE_TARGET: Vec3 = Vec3::new(-30.0, EYE_HEIGHT, -29.0);
/// Camera-to-anchor distance that stays inside the 16 m x 7 m nave.
const NAVE_RADIUS: f32 = 5.0;
const NAVE_PITCH: f32 = 0.3;
/// Looking down the nave toward the apse (west): eye east of the anchor.
const NAVE_YAW_APSE: f32 = 0.0;
/// Looking back down the nave toward the east-face door: eye west of it.
const NAVE_YAW_DOOR: f32 = std::f32::consts::PI;

struct Args {
    zone: String,
    out:  String,
    size: (u32, u32),
    visuals_override: Option<String>,
}

fn usage(msg: &str) -> ! {
    eprintln!("zone_review: {msg}");
    eprintln!(
        "usage: zone_review <zone> [--out <dir>] [--size WxH] [--visuals-override <path.ron>]"
    );
    exit(2);
}

fn die(e: String) -> ! {
    eprintln!("zone_review: {e}");
    exit(1);
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn parse_args() -> Args {
    let (mut zone, mut out, mut size, mut visuals_override) = (None, None, (1600u32, 900u32), None);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out"  => out = it.next(),
            "--size" => size = it.next().as_deref().and_then(parse_size).unwrap_or_else(|| usage("--size needs WxH")),
            "--visuals-override" => visuals_override = Some(it.next().unwrap_or_else(|| usage("--visuals-override needs a path"))),
            _ if a.starts_with("--") => usage(&format!("unknown flag {a}")),
            _ => zone = Some(a),
        }
    }
    let zone = zone.unwrap_or_else(|| usage("required: <zone> (see content/zones/zones.ron)"));
    let out = out.unwrap_or_else(|| format!("target/zone-review/{zone}"));
    Args { zone, out, size, visuals_override }
}

/// `--visuals-override`'s RON shape: the six `ZoneVisuals` lighting fields
/// plus the two other dominant look contributors (fog tint, environment
/// HDRI), every one optional — unset fields keep the zone's authored value
/// (ground/props are never touched by an override). `env_hdr` takes any
/// filesystem path, not just one under `content/` (`load_environment_hdr`
/// opens it directly, no prefix assumed) — the candidate looks live under
/// `target/lighting-looks/`.
#[derive(serde::Deserialize)]
struct LightingOverride {
    #[serde(default)]
    sun_azimuth_deg: Option<f32>,
    #[serde(default)]
    sun_elevation_deg: Option<f32>,
    #[serde(default)]
    sun_color: Option<Vec3>,
    #[serde(default)]
    sun_intensity: Option<f32>,
    #[serde(default)]
    ambient: Option<f32>,
    #[serde(default)]
    exposure: Option<f32>,
    #[serde(default)]
    fog_color: Option<Vec3>,
    #[serde(default)]
    env_hdr: Option<String>,
}

/// Layers a `LightingOverride` file's fields onto `visuals`, leaving every
/// unset field (and every non-lighting field) as the zone authored it.
fn apply_visuals_override(mut visuals: ZoneVisuals, path: &str) -> ZoneVisuals {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| die(format!("visuals override {path}: {e}")));
    let ov: LightingOverride = ron::from_str(&text)
        .unwrap_or_else(|e| die(format!("visuals override {path}: parse error: {e}")));
    if let Some(v) = ov.sun_azimuth_deg { visuals.sun_azimuth_deg = Some(v); }
    if let Some(v) = ov.sun_elevation_deg { visuals.sun_elevation_deg = Some(v); }
    if let Some(v) = ov.sun_color { visuals.sun_color = v; }
    if let Some(v) = ov.sun_intensity { visuals.sun_intensity = v; }
    if let Some(v) = ov.ambient { visuals.ambient = v; }
    if let Some(v) = ov.exposure { visuals.exposure = v; }
    if let Some(v) = ov.fog_color { visuals.fog_color = v; }
    if let Some(v) = ov.env_hdr { visuals.env = Some(v); }
    visuals
}

// ── Prop placements ──────────────────────────────────────────────────────

struct PropInstance {
    model:   String,
    prop_id: String,
    pos:     Vec3,
    yaw_deg: f32,
    scale:   f32,
}

/// The model's containing directory name (e.g. ".../props/rock_09/rock_09_1k.gltf"
/// → "rock_09") — the stable per-asset id `zones.ron` placements share.
fn prop_id(model: &str) -> String {
    Path::new(model)
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| model.to_string())
}

fn load_props(visuals: &ZoneVisuals) -> Vec<PropInstance> {
    visuals.props.iter().map(|p| PropInstance {
        model:   p.model.clone(),
        prop_id: prop_id(&p.model),
        pos:     p.pos,
        yaw_deg: p.yaw,
        scale:   p.scale,
    }).collect()
}

fn transform_for(p: &PropInstance) -> Mat4 {
    Mat4::from_scale_rotation_translation(Vec3::splat(p.scale), Quat::from_rotation_y(p.yaw_deg.to_radians()), p.pos)
}

fn load_prop_mesh(path: &str) -> MeshData {
    load_gltf_data(path).unwrap_or_else(|e| die(format!("prop {path}: {e}")))
}

/// Transform every vertex of `mesh`'s primitives into world space by `world`.
/// Unlike weapons.rs's socket-follow (rotation + translation only, scale
/// dropped), prop placements carry meaningful uniform scale, so it stays.
fn place(mesh: MeshData, world: Mat4) -> Vec<PrimitiveData> {
    mesh.primitives.into_iter().map(|mut prim| {
        for v in &mut prim.vertices {
            let pos = Vec3::from_array(v.position);
            let nrm = Vec3::from_array(v.normal);
            let tan = Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2]);
            v.position = world.transform_point3(pos).to_array();
            v.normal = world.transform_vector3(nrm).normalize_or_zero().to_array();
            let tn = world.transform_vector3(tan).normalize_or_zero();
            v.tangent = [tn.x, tn.y, tn.z, v.tangent[3]];
        }
        prim
    }).collect()
}

/// Single-linkage clusters of placements within `CLUSTER_RADIUS` (XZ
/// distance) of another member, singletons dropped — a "cluster" of one is
/// already fully covered by the wide shot (in-context) and that model's
/// close-up (in detail), so it earns no separate mid shot.
fn cluster_props(props: &[PropInstance]) -> Vec<Vec<&PropInstance>> {
    fn find(parent: &mut [usize], x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }
    let n = props.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in (i + 1)..n {
            let (a, b) = (props[i].pos, props[j].pos);
            let dist = ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt();
            if dist <= CLUSTER_RADIUS {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    // Keyed by numeric root so iteration order is deterministic across runs.
    let mut groups: BTreeMap<usize, Vec<&PropInstance>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(&props[i]);
    }
    groups.into_values().filter(|g| g.len() >= 2).collect()
}

// ── Ground + player ──────────────────────────────────────────────────────

/// Flat ground fallback extent — unused by either shipped zone (both author
/// a `ground` set) but keeps a missing one from panicking here. Wider than
/// turntable.rs/gear_render.rs's GROUND_EXTENT (single-prop framing); this
/// one has to cover a whole zone.
const GROUND_EXTENT: f32 = 100.0;

/// The zone's real ground mesh (heightmap grid + tiling PBR set) — the exact
/// same `generate_ground`/`load_ground_material` call ZoneDressingSystem
/// makes, so material response under the zone's IBL matches the live game.
fn build_ground(visuals: &ZoneVisuals) -> Vec<PrimitiveData> {
    match &visuals.ground {
        Some(g) => {
            let material = load_ground_material(&g.texture_dir).unwrap_or_else(|e| die(format!("ground: {e}")));
            generate_ground(g.size, g.tile, material).primitives
        }
        None => vec![review::ground_quad(GROUND_TOP_Y, GROUND_EXTENT)],
    }
}

/// The player model in its rest pose, translated to `pos` (no rotation —
/// gear_render.rs's precedent leaves the character's authored bind-pose
/// facing alone too; this is a scale reference, not a staged shot).
fn player_prims_at(pos: Vec3) -> Vec<PrimitiveData> {
    let mut data = load_gltf_data(PLAYER_GLB).unwrap_or_else(|e| die(format!("player {PLAYER_GLB}: {e}")));
    let skel = data.skeleton.as_ref().unwrap_or_else(|| die(format!("{PLAYER_GLB}: no skeleton")));
    let pose: Vec<LocalTransform> = skel.joints.iter().map(|j| j.rest).collect();
    review::skin_to_pose(&mut data, &pose);
    place(data, Mat4::from_translation(pos))
}

// ── Shots ─────────────────────────────────────────────────────────────────

fn render(r: &mut OffscreenRenderer, data: MeshData, w: u32, h: u32) -> RgbaImage {
    let target = r.target(w, h);
    r.render_mesh(&target, data, wgpu::Color::BLACK);
    let pixels = r.read(&target);
    RgbaImage::from_raw(w, h, pixels).expect("readback size matches WxH")
}

/// Establishing shot over the dressed area: every prop drawn, camera fit
/// (`set_camera_turntable`'s bounding-sphere fit, three-quarter yaw) to the
/// placements within `WIDE_RADIUS` so distant horizon landmarks don't pull
/// the shot off-centre or force too wide a fit.
fn render_wide(r: &mut OffscreenRenderer, visuals: &ZoneVisuals, props: &[PropInstance], w: u32, h: u32) -> RgbaImage {
    let mut prims = Vec::new();
    let mut near_min = Vec3::splat(f32::INFINITY);
    let mut near_max = Vec3::splat(f32::NEG_INFINITY);
    for p in props {
        let placed = place(load_prop_mesh(&p.model), transform_for(p));
        if (p.pos.x * p.pos.x + p.pos.z * p.pos.z).sqrt() <= WIDE_RADIUS {
            let (pmin, pmax) = review::aabb(&placed);
            near_min = near_min.min(pmin);
            near_max = near_max.max(pmax);
        }
        prims.extend(placed);
    }
    if !near_min.x.is_finite() {
        let (a, b) = review::aabb(&prims); // no prop within WIDE_RADIUS: fall back to everything
        near_min = a;
        near_max = b;
    }
    r.set_camera_turntable(near_min, near_max, THREE_QUARTER_YAW);
    prims.extend(build_ground(visuals));
    render(r, MeshData { primitives: prims, skeleton: None, clips: Vec::new() }, w, h)
}

/// Inside the town chapel's nave, looking down its length toward the apse
/// (west) or back toward the door (east) — `NAVE_RADIUS` keeps the camera
/// inside the walls. Draws the full prop dressing: the chapel itself is a
/// zones.ron prop (chapter03 is collision-only), and the street outside
/// reads through the door opening.
fn render_interior(r: &mut OffscreenRenderer, visuals: &ZoneVisuals, props: &[PropInstance], yaw: f32, w: u32, h: u32) -> RgbaImage {
    let eye = orbit_eye(NAVE_TARGET, NAVE_RADIUS, yaw, NAVE_PITCH);
    r.set_camera_lookat(eye, NAVE_TARGET);
    let mut prims: Vec<PrimitiveData> = props
        .iter()
        .flat_map(|p| place(load_prop_mesh(&p.model), transform_for(p)))
        .collect();
    prims.extend(build_ground(visuals));
    render(r, MeshData { primitives: prims, skeleton: None, clips: Vec::new() }, w, h)
}

/// Mid shot of one proximity cluster, isolated from the rest of the zone's
/// dressing so the group reads clearly.
fn render_mid(r: &mut OffscreenRenderer, visuals: &ZoneVisuals, cluster: &[&PropInstance], w: u32, h: u32) -> RgbaImage {
    let mut prims = Vec::new();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for p in cluster {
        let placed = place(load_prop_mesh(&p.model), transform_for(p));
        let (pmin, pmax) = review::aabb(&placed);
        min = min.min(pmin);
        max = max.max(pmax);
        prims.extend(placed);
    }
    let cx = (min.x + max.x) * 0.5;
    let cz = (min.z + max.z) * 0.5;
    let target = Vec3::new(cx, ground_height(cx, cz), cz);
    r.set_camera_lookat(gameplay_eye(target, MID_RADIUS), target);
    prims.extend(build_ground(visuals));
    render(r, MeshData { primitives: prims, skeleton: None, clips: Vec::new() }, w, h)
}

/// Close-up on one prop instance, isolated (no other props) so nothing
/// occludes it: camera at `CLOSE_DISTANCE` from the prop at eye height, the
/// player standing beside it for scale.
fn render_close(r: &mut OffscreenRenderer, visuals: &ZoneVisuals, prop: &PropInstance, w: u32, h: u32) -> RgbaImage {
    let prop_prims = place(load_prop_mesh(&prop.model), transform_for(prop));
    let (pmin, pmax) = review::aabb(&prop_prims);
    let center = Vec3::new((pmin.x + pmax.x) * 0.5, 0.0, (pmin.z + pmax.z) * 0.5);

    // Horizontal approach direction the camera looks along. Aiming at the
    // raw AABB center works for human-scale decor but puts the camera deep
    // inside solid geometry for large horizon props (rock faces spanning
    // tens of metres) — aim at the near-surface point along this direction
    // instead, so CLOSE_DISTANCE always clears the prop regardless of its
    // footprint.
    let az = Vec3::new(THREE_QUARTER_YAW.cos(), 0.0, THREE_QUARTER_YAW.sin());
    let near_extent = prop_prims.iter().flat_map(|p| p.vertices.iter())
        .map(|v| (Vec3::from_array(v.position) - center).dot(az))
        .fold(0.0f32, f32::max);
    let aim = center + az * near_extent;

    let eye_y = ground_height(aim.x, aim.z) + EYE_HEIGHT;
    // Level gaze at eye height when the prop is tall enough to contain it;
    // otherwise aim at its own vertical span so short props (stumps) aren't
    // shot from a camera pointed over their heads.
    let target_y = eye_y.clamp(pmin.y + 0.15, (pmax.y - 0.15).max(pmin.y + 0.15));
    let target = Vec3::new(aim.x, target_y, aim.z);

    let dy = eye_y - target_y;
    let horiz = (CLOSE_DISTANCE * CLOSE_DISTANCE - dy * dy).max(0.25).sqrt();
    let eye = Vec3::new(target.x + az.x * horiz, eye_y, target.z + az.z * horiz);
    r.set_camera_lookat(eye, target);

    // Player stands to the side (perpendicular to the view axis) rather than
    // in front of or behind the prop, at a clearance derived from the prop's
    // local extent along that axis near the aim point (clamped — see
    // PLAYER_OFFSET_MIN/MAX).
    let right_h = az.cross(Vec3::Y).normalize_or_zero();
    let half_width_right = prop_prims.iter().flat_map(|p| p.vertices.iter())
        .map(|v| (Vec3::from_array(v.position) - target).dot(right_h).abs())
        .fold(0.0f32, f32::max);
    let side_offset = (half_width_right + 0.6).clamp(PLAYER_OFFSET_MIN, PLAYER_OFFSET_MAX);
    let player_xz = Vec3::new(target.x, 0.0, target.z) + right_h * side_offset;
    let player_pos = Vec3::new(player_xz.x, ground_height(player_xz.x, player_xz.z), player_xz.z);

    let mut prims = prop_prims;
    prims.extend(player_prims_at(player_pos));
    prims.extend(build_ground(visuals));
    render(r, MeshData { primitives: prims, skeleton: None, clips: Vec::new() }, w, h)
}

fn save(img: &RgbaImage, path: &Path) {
    if let Err(e) = img.save(path) {
        eprintln!("zone_review: cannot write {}: {e}", path.display());
        exit(1);
    }
}

fn main() {
    let args = parse_args();
    let (w, h) = args.size;

    let def = load_zones(ZONES_PATH);
    let zone = def.zones.iter().find(|z| z.name == args.zone).unwrap_or_else(|| {
        let have: Vec<&str> = def.zones.iter().map(|z| z.name.as_str()).collect();
        die(format!("no zone {:?} (zones.ron has: [{}])", args.zone, have.join(", ")))
    });
    let visuals = match &args.visuals_override {
        Some(path) => apply_visuals_override(zone.visuals.clone(), path),
        None => zone.visuals.clone(),
    };
    let visuals = &visuals;

    let Some(mut r) = OffscreenRenderer::new(w as f32 / h as f32) else {
        eprintln!("zone_review: no GPU adapter available");
        exit(1);
    };
    r.set_ssao(true);
    // Without this, the review harness would render stone props with the
    // detail layer absent — blind to the feature it exists to check.
    match load_ground_material(DETAIL_TEXTURE_DIR) {
        Ok(material) => r.set_detail_material(material),
        Err(e) => eprintln!("zone_review: detail tile not loaded ({DETAIL_TEXTURE_DIR}): {e}"),
    }
    let hdri = visuals.env.as_deref().unwrap_or(DEFAULT_HDRI);
    if let Err(e) = r.load_environment_hdr(hdri) {
        eprintln!("zone_review: failed to load HDRI {hdri}: {e}");
        exit(1);
    }
    r.draw_sky = true;
    r.set_fog(visuals.fog_color, visuals.fog_density);
    r.set_fog_height(visuals.fog_height, visuals.fog_height_falloff);
    r.set_light(TestLight { direction: resolve_sun_dir(visuals), color: resolve_sun_color(visuals), ambient: visuals.ambient });
    r.set_exposure(visuals.exposure);

    let out = Path::new(&args.out);
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("zone_review: cannot create {}: {e}", args.out);
        exit(1);
    }

    let props = load_props(visuals);
    let mut sheet_frames: Vec<RgbaImage> = Vec::new();

    let wide = render_wide(&mut r, visuals, &props, w, h);
    save(&wide, &out.join("wide.png"));
    sheet_frames.push(wide);

    // The chapel nave anchor is hardcoded (NAVE_TARGET) — only run this
    // shot against the chapter03 town it was measured against.
    if zone.chapter.as_deref() == Some("chapter03") {
        let apse = render_interior(&mut r, visuals, &props, NAVE_YAW_APSE, w, h);
        save(&apse, &out.join("interior_apse.png"));
        sheet_frames.push(apse);
        let door = render_interior(&mut r, visuals, &props, NAVE_YAW_DOOR, w, h);
        save(&door, &out.join("interior_door.png"));
        sheet_frames.push(door);
    }

    let clusters = cluster_props(&props);
    for (i, cluster) in clusters.iter().enumerate() {
        let img = render_mid(&mut r, visuals, cluster, w, h);
        save(&img, &out.join(format!("mid_{i:02}.png")));
        sheet_frames.push(img);
    }

    let mut seen = HashSet::new();
    let mut close_count = 0;
    for p in &props {
        if seen.insert(p.model.clone()) {
            let img = render_close(&mut r, visuals, p, w, h);
            save(&img, &out.join(format!("close_{}.png", p.prop_id)));
            sheet_frames.push(img);
            close_count += 1;
        }
    }

    let sheet = review::contact_sheet(&sheet_frames, THUMB);
    save(&sheet, &out.join("contact_sheet.png"));

    let interior_count = if zone.chapter.as_deref() == Some("chapter03") { 2 } else { 0 };
    println!(
        "zone_review: wrote wide + {} mid + {close_count} close + {interior_count} interior + contact sheet to {}",
        clusters.len(), args.out
    );
}
