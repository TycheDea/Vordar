// Ability definitions — the shapes an ability's tuning can take. Data-driven
// via ClassLibrary (player::class), which loads these from RON per class.

use crate::combat::stats::DamageType;

#[derive(Clone, serde::Deserialize)]
pub struct AbilityDef {
    pub id: String,
    /// Display label for the action bar.
    pub name: String,
    pub cooldown_micros: u64,
    /// Animation clip the caster plays (all rigs share KayKit clip names, so
    /// this is class-level data). `None` falls back to the race's default
    /// attack clip. Cosmetic — only the client reads it.
    #[serde(default)]
    pub anim: Option<String>,
    /// How long the cast one-shot latches before locomotion resumes; `None`
    /// uses the client's default.
    #[serde(default)]
    pub anim_secs: Option<f32>,
    pub effect: AbilityEffect,
}

#[derive(Clone, serde::Deserialize)]
pub enum AbilityEffect {
    /// Scheduled-snapshot area mechanic (DESIGN.md §3): telegraph broadcast,
    /// hit test fires once at T = telegraph completion.
    Scheduled {
        /// Prefab clients spawn for the telegraph visual.
        telegraph_prefab: String,
        radius: f32,
        damage: i32,
        #[serde(default)]
        damage_type: DamageType,
        /// Cast time = telegraph duration. T (the hit test) lands at its end.
        cast_micros: u64,
        /// Max distance from caster to target position (server-validated).
        max_range: f32,
    },
    /// A replicated projectile entity launched toward the target point;
    /// damage on contact (favor-the-shooter is just collision — no rewind).
    Projectile {
        prefab: String,
        speed: f32,
        damage: i32,
        #[serde(default)]
        damage_type: DamageType,
        ttl_secs: f32,
        /// Spawn this far in front of the caster (clears the caster's hitbox).
        spawn_offset: f32,
    },
    /// Gap-closer: the caster dashes to the target point over the cast time
    /// and a Scheduled-style hit test fires there at arrival (same field
    /// shape as Scheduled; the dash is the only extra behavior).
    Leap {
        /// Prefab clients spawn for the arrival telegraph visual.
        telegraph_prefab: String,
        radius: f32,
        damage: i32,
        #[serde(default)]
        damage_type: DamageType,
        /// Dash duration; the arrival hit test lands at its end.
        cast_micros: u64,
        /// Max dash distance (server-validated).
        max_range: f32,
    },
}
