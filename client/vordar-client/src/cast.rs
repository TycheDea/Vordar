// Ability casting: reads input (LMB auto-fire, edge-triggered Q/E), applies
// client-side range clamping, and drives per-cast presentation (swing/attack
// clips, VFX, optimistic dash prediction). The only netcode is the stamp-seq-
// send seam on NetClientState (`send_cast_intent`) — the server re-validates
// class, cooldown, and range regardless.

use crate::net::NetClientState;
use engine_app::scheduler::System;
use engine_core::components::Transform;
use engine_core::traits::Resources;
use engine_core::World;
use glam::{Vec2, Vec3};

/// Keys for the edge-triggered ability slots (slot 1, slot 2). Slot 0 is the
/// LMB held-repeat attack.
const SLOT_KEYS: [winit::keyboard::KeyCode; 2] =
    [winit::keyboard::KeyCode::KeyQ, winit::keyboard::KeyCode::KeyE];

/// Casts the local class's abilities at the cursor's ground point: slot 0
/// auto-fires while LMB is held (at the cooldown rate), later slots are
/// edge-triggered keys (Q, E). Targets for ranged-capped effects are clamped
/// so an honest cast is never rejected. The client gate is display/traffic
/// hygiene — the server re-validates class, cooldown, and range.
pub struct AbilityCastSystem;

impl Default for AbilityCastSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl AbilityCastSystem {
    pub fn new() -> Self {
        Self
    }
}

impl System for AbilityCastSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        /// Slot metadata for the local class.
        struct SlotMeta {
            id: String,
            /// Range clamp for targeted effects.
            range: Option<f32>,
            cooldown_secs: f32,
            /// Leap cast time if it's a dash (drives the optimistic impulse).
            leap_micros: Option<u64>,
            /// Per-ability cast animation (cosmetic).
            anim: Option<String>,
            anim_secs: Option<f32>,
        }
        let Some(class) = crate::local_class(world, resources) else { return };
        let slots: Vec<SlotMeta> = {
            let Some(library) = resources.get::<vordar_game::class::ClassLibrary>() else { return };
            library
                .abilities_of(&class)
                .iter()
                .map(|a| {
                    let (range, leap_micros) = match &a.effect {
                        vordar_game::skills::AbilityEffect::Scheduled { max_range, .. } => (Some(*max_range), None),
                        vordar_game::skills::AbilityEffect::Projectile { .. } => (None, None),
                        vordar_game::skills::AbilityEffect::Leap { max_range, cast_micros, .. } => {
                            (Some(*max_range), Some(*cast_micros))
                        }
                    };
                    SlotMeta {
                        id: a.id.clone(),
                        range,
                        cooldown_secs: a.cooldown_micros as f32 / 1e6,
                        leap_micros,
                        anim: a.anim.clone(),
                        anim_secs: a.anim_secs,
                    }
                })
                .collect()
        };
        {
            let cooldowns: Vec<f32> = slots.iter().map(|s| s.cooldown_secs).collect();
            let cast = resources.get_mut::<crate::CastState>().unwrap();
            cast.sync(&class, &cooldowns);
            cast.tick(delta);
        }

        let mut triggered: Vec<usize> = Vec::new();
        let lmb = resources
            .get::<engine_app::input::MouseState>()
            .map(|m| m.is_pressed(winit::event::MouseButton::Left))
            .unwrap_or(false);
        if lmb {
            triggered.push(0);
        }
        for (i, key) in SLOT_KEYS.iter().enumerate() {
            let just_pressed = resources
                .get::<engine_app::input::KeyboardState>()
                .map(|kb| kb.just_pressed(*key))
                .unwrap_or(false);
            if just_pressed {
                triggered.push(i + 1);
            }
        }
        triggered.retain(|&s| {
            s < slots.len() && resources.get::<crate::CastState>().map(|c| c.ready(s)).unwrap_or(false)
        });
        if triggered.is_empty() {
            return;
        }

        let Some(cursor) = resources.get::<engine_app::input::MouseState>().and_then(|m| m.cursor()) else {
            return;
        };
        let Some(ground) = engine_renderer::screen_to_ground(cursor, resources) else { return };
        let Some(origin) = crate::net::own_entity(resources)
            .and_then(|e| world.get::<&Transform>(e).ok().map(|t| t.position))
        else {
            return;
        };

        for slot in triggered {
            let SlotMeta { id, range, leap_micros, anim, anim_secs, .. } = &slots[slot];
            let from = Vec2::new(origin.x, origin.z);
            let mut target = Vec2::new(ground.x, ground.z);
            if let Some(max_range) = range {
                let offset = target - from;
                if offset.length() > *max_range {
                    target = from + offset.normalize() * *max_range;
                }
            }
            let state = resources.get_mut::<NetClientState>().unwrap();
            if !state.send_cast_intent(id.clone(), target) {
                return;
            }
            let own = crate::net::own_entity(resources);
            let predict = resources.get::<NetClientState>().unwrap().predicting();
            resources.get_mut::<crate::CastState>().unwrap().fire(slot);
            if let Some(entity) = own {
                crate::pose::trigger_swing(world, entity);
                // Skinned-mesh cast animation (per-ability clip) — no-op if not animated.
                crate::locomotion::trigger_attack_clip(world, entity, anim.as_deref(), *anim_secs);
                // Turn toward the cast target (cosmetic, works while standing).
                crate::locomotion::aim_at(world, entity, Vec3::new(target.x, 0.0, target.y));
                let tint = crate::vfx::class_tint(resources, &class);
                crate::vfx::cast_burst(world, resources, entity, id, tint);
            }
            // Optimistic dash: same deterministic velocity math the server
            // runs, so reconciliation only ever sees ordinary drift. Rare
            // server-side rejects surface as a correction snap.
            if let (Some(cast_micros), Some(entity), true) = (leap_micros, own, predict) {
                let cast_secs = *cast_micros as f32 / 1e6;
                let to = Vec3::new(target.x, 0.0, target.y);
                let velocity = vordar_game::combat::leap::leap_velocity(origin, to, cast_secs);
                crate::net::start_predicted_leap(world, resources, entity, velocity, cast_secs);
            }
        }
    }
}
