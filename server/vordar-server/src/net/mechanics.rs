//! Scheduled-mechanic resolve pipeline (10 Hz self-gate on STAGGER).
//! At the first resolve tick past T, decide who was inside a mechanic's area AT T
//! — players via stamp-based rewind through their applied-intent history, NPCs at
//! their current position. Damage then flows through Health / HealthDepleted.

use engine_app::events::EventBus;
use engine_app::scheduler::System;
use engine_core::components::{Health, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;
use hecs::Entity;
use std::collections::VecDeque;
use vordar_game::combat::buff::ravager_mods;
use vordar_game::combat::stats::compute_damage;
use vordar_game::events::DamageDealt;
use vordar_game::motion::{step, PlayRadius};
use vordar_game::{CombatStats, Enemy, Mechanic, Provoked};
use vordar_protocol::{encode, ServerMsg};
use super::{aoi_conns, NetServerState, MAX_REWIND_MICROS, STAGGER};

/// Fixed server tick duration — each applied intent integrates exactly this.
const TICK_DT: f32 = 1.0 / 60.0;

/// The scheduled-snapshot test (DESIGN.md §3): at the first resolve tick past
/// each mechanic's T, decide who was inside its area AT T — players via
/// stamp-based rewind through their applied-intent history (an input stamped
/// ≤ T counts even though it arrived after T: favor-the-defender), NPCs at
/// their current server-driven position. Damage flows through Health, so
/// deaths take the existing HealthDepleted/despawn path.
pub struct MechanicResolveSystem {
    ticks: u64,
}

impl MechanicResolveSystem {
    pub fn new() -> Self {
        Self { ticks: 0 }
    }
}

impl System for MechanicResolveSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // PostUpdate runs at POST_HZ; resolve keeps its 10 Hz cadence.
        let due_now = self.ticks % STAGGER == 0;
        self.ticks += 1;
        if !due_now {
            return;
        }
        let now = resources.get::<NetServerState>().unwrap().server.now_micros();
        let bound = resources.get::<PlayRadius>().copied().unwrap_or_default().0;

        let due: Vec<(Entity, Mechanic, Vec3)> = world
            .query::<(Entity, &Transform, &Mechanic)>()
            .iter()
            .filter(|(_, _, m)| now >= m.resolve_at_micros)
            .map(|(e, t, m)| (e, *m, t.position))
            .collect();
        if due.is_empty() {
            return;
        }

        for (mech_entity, mech, center) in due {
            // Rewind to T, but never further back from now than the cap —
            // high-latency players get degraded forgiveness, not infinite rewind.
            let t_eff = mech.resolve_at_micros.max(now.saturating_sub(MAX_REWIND_MICROS));

            let targets: Vec<(Entity, Vec3)> = world
                .query::<(Entity, &Transform, &Health)>()
                .iter()
                .filter(|&(e, ..)| e != mech.caster)
                .map(|(e, t, _)| (e, t.position))
                .collect();

            let mut hit_entities: Vec<Entity> = Vec::new();
            {
                let state = resources.get::<NetServerState>().unwrap();
                for (entity, pos) in targets {
                    let pos_at_t = match state.conns.values().find(|pc| pc.entity == entity) {
                        Some(pc) => rewound_position(pos, &pc.history, t_eff, bound),
                        None => pos,
                    };
                    if pos_at_t.distance_squared(center) <= mech.radius * mech.radius {
                        hit_entities.push(entity);
                    }
                }
            }

            for &entity in &hit_entities {
                let dmg = {
                    let atk = world.get::<&CombatStats>(mech.caster).ok();
                    let def = world.get::<&CombatStats>(entity).ok();
                    let seed = mech.id ^ entity.to_bits().get().rotate_left(21);
                    let (bonus_power, mult) = ravager_mods(world, mech.caster, entity);
                    let base = compute_damage(mech.damage + bonus_power, mech.damage_type, atk.as_deref(), def.as_deref(), seed);
                    (base as f32 * mult).round() as i32
                };
                if let Ok(mut health) = world.get::<&mut Health>(entity) {
                    health.current -= dmg;
                    resources
                        .get_mut::<EventBus>()
                        .unwrap()
                        .emit(DamageDealt { attacker: mech.caster, target: entity, amount: dmg });
                }
                // Targeted damage wakes passive enemies, same as projectiles.
                if world.get::<&Enemy>(entity).is_ok() {
                    let _ = world.insert_one(entity, Provoked);
                }
            }

            log::info!("mechanic {} resolved: {} hit", mech.id, hit_entities.len());
            let state = resources.get_mut::<NetServerState>().unwrap();
            let hits: Vec<u32> = hit_entities.iter().map(|&e| state.repl_ids.id_for(e)).collect();
            let frame = encode(&ServerMsg::HitResult { mechanic: mech.id, hits });
            for c in aoi_conns(&state.conns, world, center) {
                state.server.send(c, frame.clone());
            }
            let _ = world.despawn(mech_entity);
        }
    }
}

/// Walk the applied-intent history backwards, undoing every tick whose intent
/// was STAMPED after `t_eff` via `step`'s exact inverse (same clamp, negated
/// velocity). Each entry stores the velocity that actually integrated that
/// tick — a dash's LeapImpulse override, not the WASD dir — so a leap is
/// rewound by the vector that really moved the player, reconstructing the
/// position the player had committed to by time T on their own synced clock.
fn rewound_position(current: Vec3, history: &VecDeque<(u64, Vec3)>, t_eff: u64, bound: f32) -> Vec3 {
    let mut pos = current;
    for &(stamp, velocity) in history.iter().rev() {
        if stamp <= t_eff {
            break;
        }
        pos = step(pos, -velocity, TICK_DT, bound);
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use vordar_game::player::movement_velocity;

    /// A player who dashed out of a mechanic's blast between T and now: the
    /// leap carried them 12 units away (30 u/s over a 0.4 s cast). Rewinding
    /// through the recorded leap velocities reconstructs the dash-truth
    /// position at T (inside the blast → HIT). Rewinding as if the player had
    /// walked (the pre-fix WASD dead-reckoning) undoes only ~2.4 units and
    /// leaves them outside the radius (a wrong MISS) — so recording the applied
    /// velocity flips favor-the-defender from a fabricated past to the real one.
    #[test]
    fn rewind_through_a_dash_reconstructs_the_leap_truth_not_wasd() {
        let bound = 1000.0; // far outside any boundary clamp
        let center = Vec3::ZERO;
        let radius = 2.0;
        let ticks: u64 = 24; // 0.4 s cast at 60 Hz
        let dash = Vec3::new(30.0, 0.0, 0.0); // 12 units over the cast
        let tick_micros: u64 = 16_667;
        let t_eff: u64 = 1_000_000;

        // Player sat on the blast center at T, then dashed +x for the cast.
        let pos_at_t = center;
        let now_pos = pos_at_t + dash * TICK_DT * ticks as f32;

        let leap_history: VecDeque<(u64, Vec3)> =
            (1..=ticks).map(|k| (t_eff + k * tick_micros, dash)).collect();
        let rewound = rewound_position(now_pos, &leap_history, t_eff, bound);
        assert!((rewound - pos_at_t).length() < 1e-3, "dash rewind must land on the truth position: {rewound:?}");
        assert!(rewound.distance_squared(center) <= radius * radius, "player was inside the blast at T → HIT");

        // Same ticks, but history storing the WASD walk velocity (the bug):
        // the rewind subtracts only walk speed and misses.
        let walk_v = movement_velocity(Vec2::new(1.0, 0.0), 6.0);
        let wasd_history: VecDeque<(u64, Vec3)> =
            (1..=ticks).map(|k| (t_eff + k * tick_micros, walk_v)).collect();
        let wasd_rewound = rewound_position(now_pos, &wasd_history, t_eff, bound);
        assert!(
            wasd_rewound.distance_squared(center) > radius * radius,
            "WASD dead-reckoning would place the player outside the blast → wrong MISS: {wasd_rewound:?}"
        );
    }
}
