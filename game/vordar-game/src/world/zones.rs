// Zone topology: named zones connected by portals (Phase 7).
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
pub fn portal_hit<'a>(portals: &'a [PortalDef], pos: Vec3) -> Option<&'a PortalDef> {
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
