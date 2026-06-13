// Client-only world dressing + HUD (Phase 7.5): the readable ground plane,
// portal monuments, and the minimap data feed. Nothing here is replicated or
// simulated — pure presentation, shared by the sandbox and the networked bin.

use crate::ui::minimap::{HudDot, HudState};
use crate::CastState;
use engine_app::input::MouseState;
use engine_app::scheduler::System;
use engine_core::components::{RenderShape, RenderShapeType, ShapeGroup, Transform};
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext};
use engine_core::World;
use glam::{Vec2, Vec3};
use hecs::Entity;
use vordar_game::combat::projectile::spawn_projectile;
use vordar_game::skills::{skill, SkillEffect};
use vordar_game::zones::ZonesDef;
use vordar_game::Player;

/// Excluded from the minimap (the ground would be one giant dot).
pub struct HudHidden;

/// Marks client-local zone scenery (floor, portals) — torn down and rebuilt
/// when the current zone changes.
pub struct ZoneDressing;

/// Which zone this client believes it is in. Starts at "start"; updated by
/// the Redirect handler online (the sandbox never changes it).
pub struct CurrentZone(pub String);

/// Ground palette per zone — each zone should read as a different place.
fn zone_palette(zone: &str) -> Vec3 {
    match zone {
        "start" => Vec3::new(0.30, 0.42, 0.22), // meadow green
        "east" => Vec3::new(0.50, 0.40, 0.20),  // ochre badlands
        _ => Vec3::new(0.32, 0.32, 0.34),       // unknown: slate
    }
}

/// Spawns/rebuilds the floor and portal monuments whenever the zone changes.
pub struct ZoneDressingSystem {
    applied: Option<String>,
}

impl ZoneDressingSystem {
    pub fn new() -> Self {
        Self { applied: None }
    }
}

impl System for ZoneDressingSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let zone = resources.get::<CurrentZone>().map(|z| z.0.clone()).unwrap_or_default();
        if self.applied.as_deref() == Some(zone.as_str()) {
            return;
        }
        self.applied = Some(zone.clone());

        // Tear down the previous zone's scenery.
        let old: Vec<Entity> = world.query::<(Entity, &ZoneDressing)>().iter().map(|(e, _)| e).collect();
        {
            let queue = resources.get_mut::<DespawnQueue>().unwrap();
            for entity in old {
                queue.push(entity, None);
            }
        }

        // The ground: one big slab whose top face sits at y = -0.5, flush
        // with every unit's hitbox bottom. shape_type 7 = the readable
        // ground pattern (checker + gridlines + axis lines); params[0] is
        // the gridline period in world units.
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

/// Publishes the minimap: tracked player at the center, every visible entity
/// as a dot in its own body color, portals as rim markers. Runs once per
/// display frame. (The bolt cooldown moved to the action bar.)
pub struct HudSyncSystem;

impl System for HudSyncSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let own = crate::net::own_entity(resources)
            .or_else(|| world.query::<(Entity, &Player)>().iter().next().map(|(e, _)| e));
        let center = own.and_then(|e| world.get::<&Transform>(e).ok().map(|t| t.position));

        let mut dots: Vec<HudDot> = Vec::new();
        if center.is_some() {
            for (entity, transform, shape) in world.query::<(Entity, &Transform, &RenderShape)>().iter() {
                if Some(entity) == own || world.get::<&HudHidden>(entity).is_ok() {
                    continue;
                }
                dots.push(HudDot {
                    pos: Vec2::new(transform.position.x, transform.position.z),
                    color: shape.color.to_array(),
                });
            }
            for (entity, transform, group) in world.query::<(Entity, &Transform, &ShapeGroup)>().iter() {
                if Some(entity) == own || world.get::<&HudHidden>(entity).is_ok() {
                    continue;
                }
                let color = group.shapes.first().map(|s| s.color.to_array()).unwrap_or([1.0; 3]);
                dots.push(HudDot {
                    pos: Vec2::new(transform.position.x, transform.position.z),
                    color,
                });
            }
        }

        let zone = resources.get::<CurrentZone>().map(|z| z.0.clone()).unwrap_or_default();
        let markers: Vec<Vec2> = resources
            .get::<ZonesDef>()
            .and_then(|def| def.zones.iter().find(|z| z.name == zone))
            .map(|z| z.portals.iter().map(|p| Vec2::new(p.pos.x, p.pos.z)).collect())
            .unwrap_or_default();

        let heading = engine_renderer::camera_yaw(resources);

        let Some(hud) = resources.get_mut::<HudState>() else { return };
        hud.open = center.is_some();
        hud.center = center.map(|p| Vec2::new(p.x, p.z));
        hud.heading = heading;
        hud.dots = dots;
        hud.markers = markers;
        hud.range = 45.0;
        hud.label = zone;
    }
}

/// Offline left-click bolt (sandbox parity with the networked MouseCastSystem):
/// fires the shared projectile directly — same skill numbers, no server.
pub struct SandboxCastSystem;

impl System for SandboxCastSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let Some(target) = poll_cast_target(resources, delta) else { return };
        let Some((player, origin)) = world
            .query::<(Entity, &Transform, &Player)>()
            .iter()
            .next()
            .map(|(e, t, _)| (e, t.position))
        else {
            return;
        };
        let Some(def) = skill("bolt") else { return };
        let SkillEffect::Projectile { prefab, speed, damage, ttl_secs, spawn_offset } = def.effect else {
            return;
        };
        let mut dir = target - origin;
        dir.y = 0.0;
        if dir.length_squared() < 1e-6 {
            return;
        }
        let dir = dir.normalize();
        resources.get_mut::<CastState>().unwrap().bolt.fire();
        spawn_projectile(
            world,
            resources,
            prefab,
            origin + dir * spawn_offset,
            dir,
            speed,
            damage,
            ttl_secs,
            player,
            false,
        );
    }
}

/// Shared cast gate: ticks all cooldowns, and when the left button is held,
/// the bolt is ready, and the cursor hits the ground, returns the ground
/// point to fire at. The caller commits by calling `CastState.bolt.fire()`.
pub(crate) fn poll_cast_target(resources: &mut Resources, delta: f32) -> Option<Vec3> {
    {
        let cast = resources.get_mut::<CastState>()?;
        cast.tick(delta);
        if !cast.bolt.ready() {
            return None;
        }
    }
    let lmb = resources
        .get::<MouseState>()
        .map(|m| m.is_pressed(winit::event::MouseButton::Left))
        .unwrap_or(false);
    if !lmb {
        return None;
    }
    let cursor = resources.get::<MouseState>().and_then(|m| m.cursor())?;
    engine_renderer::screen_to_ground(cursor, resources)
}
