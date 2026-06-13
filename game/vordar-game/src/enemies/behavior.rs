// Per-archetype enemy behavior — the divergence point.
//
// The engagement model (idle until aggro/Provoked) is shared and lives in
// EnemyAISystem; WHAT an engaged enemy does is a behavior looked up by prefab
// name in the BehaviorRegistry. The default interprets the data-driven
// `Enemy.attack` profile from the prefab RON; an archetype that outgrows data
// registers its own EnemyBehavior (see chapter-01's enemies/ modules) without
// touching anything shared.
//
// Determinism contract (DESIGN.md §6): a behavior is a pure function of its
// ctx — dt-accumulated state lives on the Enemy component; no clocks, no RNG.

use super::{AttackKind, Enemy};
use glam::Vec3;

/// Everything an engaged enemy knows when deciding its action this tick.
pub struct BehaviorCtx<'a> {
    pub enemy: &'a mut Enemy,
    pub position: Vec3,
    /// The engagement target (nearest player).
    pub target: Vec3,
    pub dt: f32,
}

/// A projectile the behavior wants launched (the enemy holds still to fire).
pub struct FireOrder {
    pub prefab: String,
    pub dir: Vec3,
    pub speed: f32,
    pub damage: i32,
    pub ttl: f32,
}

/// What the enemy does this tick.
pub enum Action {
    /// Stand still.
    Hold,
    /// Move with this velocity.
    Move(Vec3),
    /// Stand still and launch a projectile. The behavior commits the cooldown
    /// before returning this.
    Fire(FireOrder),
}

pub trait EnemyBehavior: Send + Sync {
    fn engaged(&self, ctx: &mut BehaviorCtx) -> Action;
}

/// The default: interpret the prefab's data-driven `Enemy.attack` profile.
/// Melee chases; ranged closes to its attack range, stops, and fires on a
/// dt-accumulated cooldown.
pub struct DataDriven;

pub(crate) static DATA_DRIVEN: DataDriven = DataDriven;

impl EnemyBehavior for DataDriven {
    fn engaged(&self, ctx: &mut BehaviorCtx) -> Action {
        let to_target = ctx.target - ctx.position;
        let dist = to_target.length();
        let chase_dir = if to_target.length_squared() > 0.01 {
            to_target.normalize()
        } else {
            Vec3::ZERO
        };

        match &ctx.enemy.attack {
            AttackKind::Melee => Action::Move(chase_dir * ctx.enemy.speed),
            AttackKind::Ranged { prefab, speed, damage, cooldown, range } => {
                if dist > *range {
                    Action::Move(chase_dir * ctx.enemy.speed)
                } else if ctx.enemy.cooldown_left == 0.0 && chase_dir != Vec3::ZERO {
                    let order = FireOrder {
                        prefab: prefab.clone(),
                        dir: chase_dir,
                        speed: *speed,
                        damage: *damage,
                        // Enough flight to cover the attack range plus a
                        // dodge margin.
                        ttl: range / speed + 0.4,
                    };
                    ctx.enemy.cooldown_left = *cooldown;
                    Action::Fire(order)
                } else {
                    Action::Hold
                }
            }
        }
    }
}

/// Prefab name → behavior. Inserted by CoreGamePlugin; chapter plugins
/// register their archetypes' overrides via `App::resource_or_default`.
/// Unregistered prefabs (and code-spawned enemies) get the data-driven default.
#[derive(Default)]
pub struct BehaviorRegistry {
    map: std::collections::HashMap<String, Box<dyn EnemyBehavior>>,
}

impl BehaviorRegistry {
    pub fn register(&mut self, prefab: &str, behavior: impl EnemyBehavior + 'static) {
        self.map.insert(prefab.to_owned(), Box::new(behavior));
    }

    pub fn get(&self, prefab: &str) -> &dyn EnemyBehavior {
        self.map.get(prefab).map(|b| b.as_ref()).unwrap_or(&DATA_DRIVEN)
    }
}
