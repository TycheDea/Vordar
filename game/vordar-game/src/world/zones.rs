// Zone topology: named zones connected by portals.
//
// Zones are shared content like world events — the server runs one App per
// zone and uses portals to hand players off; the client only ever learns
// about zones through Redirect messages. Ports/addresses are deployment
// detail and deliberately absent from the content.

use glam::Vec3;

#[derive(serde::Deserialize)]
pub struct ZonesDef {
    pub zones: Vec<ZoneDef>,
}

#[derive(Clone, serde::Deserialize)]
pub struct ZoneDef {
    pub name: String,
    /// Chapter content this zone runs (e.g. "chapter01"); the server main
    /// maps it to a plugin. None = empty zone.
    #[serde(default)]
    pub chapter: Option<String>,
    #[serde(default)]
    pub portals: Vec<PortalDef>,
    /// Visual dressing (client-only; the server parses but never reads it).
    #[serde(default)]
    pub visuals: ZoneVisuals,
}

/// Per-zone presentation: environment HDRI (IBL + sky), sun/ambient/exposure,
/// distance fog, ground material, scattered props. All optional with dusk
/// defaults.
#[derive(Clone, serde::Deserialize)]
pub struct ZoneVisuals {
    /// Radiance .hdr path; None = the shared dusk default.
    #[serde(default)]
    pub env: Option<String>,
    /// Sun azimuth in degrees (`Camera::recompute_eye`'s XZ convention: 0 =
    /// +X, increasing toward +Z). Only takes effect paired with
    /// `sun_elevation_deg` — either alone falls back to the dusk default
    /// direction matched to the shared HDRI's sun disc.
    #[serde(default)]
    pub sun_azimuth_deg: Option<f32>,
    #[serde(default)]
    pub sun_elevation_deg: Option<f32>,
    /// Sun tint; multiplied by `sun_intensity` for the final light color.
    #[serde(default = "default_sun_color")]
    pub sun_color: Vec3,
    #[serde(default = "default_sun_intensity")]
    pub sun_intensity: f32,
    /// IBL ambient scale (1.0 = the environment as authored).
    #[serde(default = "default_ambient")]
    pub ambient: f32,
    /// Tonemap exposure (1.0 = neutral).
    #[serde(default = "default_exposure")]
    pub exposure: f32,
    #[serde(default = "default_fog_color")]
    pub fog_color: Vec3,
    #[serde(default = "default_fog_density")]
    pub fog_density: f32,
    /// Height fog: density is attenuated above `fog_height` by
    /// exp(-fog_height_falloff · height above it); 0/0 = pure distance fog.
    #[serde(default)]
    pub fog_height: f32,
    #[serde(default)]
    pub fog_height_falloff: f32,
    /// PBR ground texture set; None keeps the dev slab.
    #[serde(default)]
    pub ground: Option<GroundDef>,
    #[serde(default)]
    pub props: Vec<PropDef>,
}

impl Default for ZoneVisuals {
    fn default() -> Self {
        Self {
            env:                None,
            sun_azimuth_deg:    None,
            sun_elevation_deg:  None,
            sun_color:          default_sun_color(),
            sun_intensity:      default_sun_intensity(),
            ambient:            default_ambient(),
            exposure:           default_exposure(),
            fog_color:          default_fog_color(),
            fog_density:        default_fog_density(),
            fog_height:         0.0,
            fog_height_falloff: 0.0,
            ground:             None,
            props:              Vec::new(),
        }
    }
}

#[derive(Clone, serde::Deserialize)]
pub struct GroundDef {
    /// Directory with `diff/nor_gl/rough` maps (Poly Haven jpg convention).
    pub texture_dir: String,
    /// World units per texture repeat.
    #[serde(default = "default_ground_tile")]
    pub tile: f32,
    /// Ground mesh side length, centred on the origin.
    #[serde(default = "default_ground_size")]
    pub size: f32,
}

#[derive(Clone, serde::Deserialize)]
pub struct PropDef {
    /// glTF path (e.g. "content/models/props/rock_09/rock_09_1k.gltf").
    pub model: String,
    pub pos: Vec3,
    #[serde(default = "default_prop_scale")]
    pub scale: f32,
    /// Yaw in degrees.
    #[serde(default)]
    pub yaw: f32,
}

fn default_fog_color() -> Vec3 {
    Vec3::new(0.30, 0.26, 0.28) // dusk haze
}
fn default_fog_density() -> f32 {
    0.0
}
fn default_ground_tile() -> f32 {
    6.0
}
fn default_ground_size() -> f32 {
    400.0
}
fn default_prop_scale() -> f32 {
    1.0
}

/// Sun direction (points TOWARD the light), matched to the baked default
/// HDRI's sun disc — the fallback whenever a zone authors no
/// azimuth/elevation override.
const DEFAULT_SUN_DIR: Vec3 = Vec3::new(0.11897, 0.13917, 0.98309);

fn default_sun_color() -> Vec3 {
    Vec3::new(1.5, 1.38, 1.2) // castilian_plateau_dusk_2k tint x the dusk key's intensity
}
fn default_sun_intensity() -> f32 {
    1.0
}
fn default_ambient() -> f32 {
    1.0
}
fn default_exposure() -> f32 {
    1.0
}

/// Direction pointing toward a light source at `azimuth_deg`/`elevation_deg`
/// — `Camera::recompute_eye`'s XZ convention (0° = +X, increasing toward +Z).
fn sun_dir_from_angles(azimuth_deg: f32, elevation_deg: f32) -> Vec3 {
    let (az, el) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
    Vec3::new(az.cos() * el.cos(), el.sin(), az.sin() * el.cos())
}

/// This zone's sun direction: the authored azimuth/elevation pair if both are
/// set, else the dusk default matched to the shared HDRI.
pub fn resolve_sun_dir(visuals: &ZoneVisuals) -> Vec3 {
    match (visuals.sun_azimuth_deg, visuals.sun_elevation_deg) {
        (Some(az), Some(el)) => sun_dir_from_angles(az, el),
        _ => DEFAULT_SUN_DIR,
    }
}

/// This zone's sun color: authored tint × intensity.
pub fn resolve_sun_color(visuals: &ZoneVisuals) -> Vec3 {
    visuals.sun_color * visuals.sun_intensity
}

#[derive(Clone, serde::Deserialize)]
pub struct PortalDef {
    pub pos: Vec3,
    pub radius: f32,
    pub target_zone: String,
    /// Arrival position in the target zone. Must sit clear of every portal
    /// there (see `validate_zones`) so arrival can't re-trigger a transfer.
    pub target_pos: Vec3,
}

/// Load zone definitions. Panics on failure — broken content is an authoring
/// bug the author must see immediately (same policy as chapters).
pub fn load_zones(path: &str) -> ZonesDef {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("zones '{path}' unreadable: {e}"));
    let def: ZonesDef = ron::from_str(&text)
        .unwrap_or_else(|e| panic!("zones '{path}' parse error: {e}"));
    log::info!("zones loaded: {}", def.zones.iter().map(|z| z.name.as_str()).collect::<Vec<_>>().join(", "));
    def
}

/// Margin every portal arrival point must keep beyond the destination
/// portals' radii, so a transfer can never land inside a portal.
pub const PORTAL_ARRIVAL_MARGIN: f32 = 2.0;

/// Structural sanity: every portal targets an existing zone, and lands far
/// enough from every portal there that arrival can't bounce straight back.
pub fn validate_zones(def: &ZonesDef) -> Result<(), String> {
    for zone in &def.zones {
        for portal in &zone.portals {
            let Some(target) = def.zones.iter().find(|z| z.name == portal.target_zone) else {
                return Err(format!(
                    "zone '{}': portal targets unknown zone '{}'",
                    zone.name, portal.target_zone
                ));
            };
            for other in &target.portals {
                let dist = portal.target_pos.distance(other.pos);
                if dist < other.radius + PORTAL_ARRIVAL_MARGIN {
                    return Err(format!(
                        "zone '{}': portal to '{}' arrives {dist:.1} from a portal there \
                         (needs ≥ {:.1})",
                        zone.name, portal.target_zone,
                        other.radius + PORTAL_ARRIVAL_MARGIN
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The portal containing `pos`, if any. Exact radius test (cell-granular
/// borders would flap, same reasoning as AOI).
pub fn portal_hit(portals: &[PortalDef], pos: Vec3) -> Option<&PortalDef> {
    portals.iter().find(|p| pos.distance(p.pos) <= p.radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_zones() -> ZonesDef {
        ZonesDef {
            zones: vec![
                ZoneDef {
                    name: "start".into(),
                    chapter: Some("chapter01".into()),
                    portals: vec![PortalDef {
                        pos: Vec3::new(22.0, 0.0, 0.0),
                        radius: 2.0,
                        target_zone: "east".into(),
                        target_pos: Vec3::new(-16.0, 0.0, 0.0),
                    }],
                    visuals: ZoneVisuals::default(),
                },
                ZoneDef {
                    name: "east".into(),
                    chapter: None,
                    portals: vec![PortalDef {
                        pos: Vec3::new(-22.0, 0.0, 0.0),
                        radius: 2.0,
                        target_zone: "start".into(),
                        target_pos: Vec3::new(16.0, 0.0, 0.0),
                    }],
                    visuals: ZoneVisuals::default(),
                },
            ],
        }
    }

    #[test]
    fn valid_topology_passes() {
        assert!(validate_zones(&two_zones()).is_ok());
    }

    #[test]
    fn unknown_target_zone_rejected() {
        let mut def = two_zones();
        def.zones[0].portals[0].target_zone = "nowhere".into();
        assert!(validate_zones(&def).unwrap_err().contains("nowhere"));
    }

    #[test]
    fn camping_arrival_point_rejected() {
        let mut def = two_zones();
        // Arrive right on east's return portal — would re-trigger instantly.
        def.zones[0].portals[0].target_pos = Vec3::new(-22.0, 0.0, 0.0);
        assert!(validate_zones(&def).is_err());
    }

    #[test]
    fn portal_hit_exact_radius() {
        let def = two_zones();
        let portals = &def.zones[0].portals;
        assert!(portal_hit(portals, Vec3::new(21.0, 0.0, 0.0)).is_some());
        assert!(portal_hit(portals, Vec3::new(22.0, 0.0, 2.0)).is_some()); // on the rim
        assert!(portal_hit(portals, Vec3::new(19.0, 0.0, 0.0)).is_none());
    }
}
