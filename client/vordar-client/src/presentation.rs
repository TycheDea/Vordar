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
use vordar_game::skills::AbilityEffect;
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
        "east" => Vec3::new(0.46, 0.40, 0.30),  // Emberwood Rest: warm packed-earth plaza
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

        // Zone environment: the dusk HDRI drives IBL ambient + the visible
        // sky (VQ-A5/D2). Per-zone HDRI paths arrive with the Phase 6 zone
        // schema; until then every zone shares the dusk mood.
        engine_renderer::set_environment(
            "content/textures/env/evening_road_01_puresky_2k.hdr",
            resources,
        );

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

/// Offline cast (sandbox parity with the networked AbilityCastSystem): pressing
/// a slot's key — LMB / Q / E, in class-authored order — fires its cooldown and
/// plays the attack animation (swing on the SDF body, attack clip on a skinned
/// mesh). Projectile abilities also spawn their bolt locally; Scheduled/Leap
/// effects only animate offline (their damage needs the server's
/// MechanicResolveSystem, and there are no enemies in the sandbox anyway).
pub struct SandboxCastSystem;

/// Slot → its input this frame. Order matches the action bar's KEYBINDS.
fn slot_pressed(slot: usize, resources: &Resources) -> bool {
    use engine_app::input::KeyboardState;
    use winit::keyboard::KeyCode;
    match slot {
        0 => resources.get::<MouseState>().map(|m| m.is_pressed(winit::event::MouseButton::Left)).unwrap_or(false),
        1 => resources.get::<KeyboardState>().map(|k| k.is_pressed(KeyCode::KeyQ)).unwrap_or(false),
        2 => resources.get::<KeyboardState>().map(|k| k.is_pressed(KeyCode::KeyE)).unwrap_or(false),
        _ => false,
    }
}

impl System for SandboxCastSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let Some(class) = crate::local_class(world, resources) else { return };
        let abilities = {
            let Some(library) = resources.get::<vordar_game::class::ClassLibrary>() else { return };
            library.abilities_of(&class).to_vec()
        };
        {
            let cooldowns: Vec<f32> = abilities.iter().map(|a| a.cooldown_micros as f32 / 1e6).collect();
            let cast = resources.get_mut::<CastState>().unwrap();
            cast.sync(&class, &cooldowns);
            cast.tick(delta);
        }
        let Some(player) = world.query::<(Entity, &Player)>().iter().next().map(|(e, _)| e) else { return };

        for (slot, ability) in abilities.iter().enumerate() {
            if !slot_pressed(slot, resources) { continue; }
            if !resources.get::<CastState>().map(|c| c.ready(slot)).unwrap_or(false) { continue; }
            resources.get_mut::<CastState>().unwrap().fire(slot);
            crate::pose::trigger_swing(world, player);
            // Skinned-mesh cast animation (per-ability clip) — no-op on SDF bodies.
            crate::locomotion::trigger_attack_clip(world, player, ability.anim.as_deref(), ability.anim_secs);
            let tint = crate::vfx::class_tint(resources, &class);
            crate::vfx::cast_burst(world, resources, player, tint);

            // Projectile abilities also fire their bolt locally toward the cursor.
            if let AbilityEffect::Projectile { prefab, speed, damage, damage_type, ttl_secs, spawn_offset } = &ability.effect {
                let origin = world.get::<&Transform>(player).map(|t| t.position).unwrap_or(Vec3::ZERO);
                let Some(cursor) = resources.get::<MouseState>().and_then(|m| m.cursor()) else { continue };
                let Some(target) = engine_renderer::screen_to_ground(cursor, resources) else { continue };
                let mut dir = target - origin;
                dir.y = 0.0;
                if dir.length_squared() < 1e-6 { continue; }
                let dir = dir.normalize();
                spawn_projectile(
                    world, resources, prefab, origin + dir * *spawn_offset, dir,
                    *speed, *damage, *damage_type, *ttl_secs, player, false,
                );
            }
        }
    }
}
