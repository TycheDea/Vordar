// Client-only world dressing: the readable ground plane and portal monuments,
// plus the HudHidden marker that excludes ground/dressing entities from the
// minimap (whose feed lives in ui/minimap.rs). Nothing here is replicated or
// simulated — pure presentation, shared by the sandbox and the networked bin.

use engine_app::scheduler::System;
use engine_core::components::{RenderMesh, RenderShape, RenderShapeType, Transform};
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use glam::Vec3;
use hecs::Entity;
use vordar_game::zones::{resolve_sun_color, resolve_sun_dir, ZoneVisuals, ZonesDef};

/// Excluded from the minimap (the ground would be one giant dot).
pub struct HudHidden;

/// Marks client-local zone scenery (floor, portals) — torn down and rebuilt
/// when the current zone changes.
pub struct ZoneDressing;

/// Which zone this client believes it is in. Starts at "start"; updated by
/// the Redirect handler online (the sandbox never changes it).
pub struct CurrentZone(pub String);

/// The world-space detail overlay's one shared tile (chapel_arch fix phase):
/// every `vordar_detail`-opted-in stone material samples this, regardless of
/// zone. Public so `zone_review`/`asset_inspect` (offscreen review bins) bind
/// the same tile the live game does.
pub const DETAIL_TEXTURE_DIR: &str = "content/textures/detail/limestone";

/// Ground palette per zone — each zone should read as a different place.
fn zone_palette(zone: &str) -> Vec3 {
    match zone {
        "start" => Vec3::new(0.55, 0.43, 0.35), // dry cracked earth (kept set's albedo hue)
        "east" => Vec3::new(0.44, 0.44, 0.45),  // Emberwood Rest: worn-cobble plaza (grey-mauve)
        _ => Vec3::new(0.32, 0.32, 0.34),       // unknown: slate
    }
}

/// Spawns/rebuilds the floor and portal monuments whenever the zone changes.
pub struct ZoneDressingSystem {
    applied: Option<String>,
    /// Guards the one-time detail-tile load below — the tile is global (not
    /// per-zone), so it must load exactly once regardless of how many zone
    /// changes `run` processes.
    detail_loaded: bool,
}

impl Default for ZoneDressingSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl ZoneDressingSystem {
    pub fn new() -> Self {
        Self { applied: None, detail_loaded: false }
    }
}

impl System for ZoneDressingSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        if !self.detail_loaded {
            self.detail_loaded = true;
            match crate::ground::load_ground_material(DETAIL_TEXTURE_DIR) {
                Ok(material) => engine_renderer::set_detail_material(material, resources),
                // A missing/incomplete detail directory leaves the engine's
                // 1×1 neutral overlay bound (a clean no-op, not a panic) —
                // content/textures/detail/limestone may not exist yet.
                Err(e) => log::warn!("detail tile not loaded ({DETAIL_TEXTURE_DIR}): {e}"),
            }
        }

        let zone = resources.get::<CurrentZone>().map(|z| z.0.clone()).unwrap_or_default();
        if self.applied.as_deref() == Some(zone.as_str()) {
            return;
        }
        self.applied = Some(zone.clone());

        // This zone's visual dressing from zones.ron (dusk defaults when the
        // zone doesn't author any).
        let visuals: ZoneVisuals = resources
            .get::<ZonesDef>()
            .and_then(|def| def.zones.iter().find(|z| z.name == zone))
            .map(|z| z.visuals.clone())
            .unwrap_or_default();

        // Environment HDRI drives IBL ambient + the visible sky (VQ-A5/D2);
        // fog depth-cues the horizon.
        engine_renderer::set_environment(
            visuals
                .env
                .as_deref()
                .unwrap_or("content/textures/env/castilian_plateau_overcast_2k.hdr"),
            resources,
        );
        engine_renderer::set_fog(visuals.fog_color, visuals.fog_density, resources);
        engine_renderer::set_fog_height(visuals.fog_height, visuals.fog_height_falloff, resources);
        engine_renderer::set_light(resolve_sun_dir(&visuals), resolve_sun_color(&visuals), visuals.ambient, resources);
        engine_renderer::set_exposure(visuals.exposure, resources);

        // Tear down the previous zone's scenery.
        let old: Vec<Entity> = world.query::<(Entity, &ZoneDressing)>().iter().map(|(e, _)| e).collect();
        {
            let queue = resources.expect_mut::<DespawnQueue>();
            for entity in old {
                queue.push(entity, None);
            }
        }

        // The ground. With an authored texture set: a heightmap grid with the
        // tiling PBR material (VQ-A2), flat across the play area. Otherwise
        // the dev slab (shape_type 7 readable pattern) stays.
        let mut mesh_ground = false;
        if let Some(g) = &visuals.ground {
            let dir = g.texture_dir.clone();
            let (size, tile) = (g.size, g.tile);
            let key = format!("zone-ground:{zone}");
            let job = move || {
                Ok(crate::ground::generate_ground(size, tile, crate::ground::load_ground_material(&dir)?))
            };
            if engine_renderer::request_procedural_mesh(&key, job, resources) {
                world.spawn((
                    Transform::default(),
                    RenderMesh { asset: key, tint: Vec3::ONE },
                    ZoneDressing,
                    HudHidden,
                ));
                mesh_ground = true;
            }
        }
        if !mesh_ground {
            world.spawn((
                Transform {
                    position: Vec3::new(0.0, -1.0, 0.0),
                    scale: Vec3::new(400.0, 1.0, 400.0),
                    ..Transform::default()
                },
                RenderShape {
                    shape: RenderShapeType::Custom { shape_type: 7, params: [8.0, 0.0, 0.0, 0.0] },
                    color: zone_palette(&zone),
                },
                ZoneDressing,
                HudHidden,
            ));
        }

        // Scattered props (rocks, ruin stonework, cypresses — silhouettes against the dusk).
        for prop in &visuals.props {
            world.spawn((
                Transform {
                    position: prop.pos,
                    rotation: glam::Quat::from_rotation_y(prop.yaw.to_radians()),
                    scale:    Vec3::splat(prop.scale),
                },
                RenderMesh { asset: prop.model.clone(), tint: Vec3::ONE },
                ZoneDressing,
                HudHidden,
            ));
        }

        // Portal monuments at this zone's exits.
        let portals: Vec<Vec3> = resources
            .get::<ZonesDef>()
            .and_then(|def| def.zones.iter().find(|z| z.name == zone))
            .map(|z| z.portals.iter().map(|p| p.pos).collect())
            .unwrap_or_default();
        for pos in portals {
            match spawn_prefab("portal", pos, &mut SpawnContext { world, resources }) {
                Ok(entity) => {
                    let _ = world.insert(entity, (ZoneDressing, HudHidden));
                }
                Err(e) => log::error!("portal dressing spawn failed: {e}"),
            }
        }
    }
}

