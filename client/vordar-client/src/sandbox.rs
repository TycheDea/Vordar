// The sandbox binary's offline ability casting — gameplay input, not
// presentation: ClientPlugin's local analogue of net::cast's networked
// AbilityCastSystem, used only by the no-networking sandbox build.

use crate::CastState;
use engine_app::input::MouseState;
use engine_app::scheduler::System;
use engine_core::components::Transform;
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;
use hecs::Entity;
use vordar_game::combat::projectile::spawn_projectile;
use vordar_game::skills::AbilityEffect;
use vordar_game::Player;

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
            // Turn toward the cursor's ground point (cosmetic, works standing).
            if let Some(target) = resources
                .get::<MouseState>()
                .and_then(|m| m.cursor())
                .and_then(|c| engine_renderer::screen_to_ground(c, resources))
            {
                crate::locomotion::aim_at(world, player, target);
            }
            let tint = crate::vfx::class_tint(resources, &class);
            crate::vfx::cast_burst(world, resources, player, &ability.id, tint);

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
