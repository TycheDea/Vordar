// Skill definitions — the tuning for castable mechanics. Hardcoded for now;
// migrates to RON (`MechanicDef`) with the Phase 8 content pass, same as
// chapters did.

pub struct SkillDef {
    pub id: &'static str,
    pub cooldown_micros: u64,
    pub effect: SkillEffect,
}

pub enum SkillEffect {
    /// Scheduled-snapshot area mechanic (DESIGN.md §3): telegraph broadcast,
    /// hit test fires once at T = telegraph completion.
    Scheduled {
        /// Prefab clients spawn for the telegraph visual.
        telegraph_prefab: &'static str,
        radius: f32,
        damage: i32,
        /// Cast time = telegraph duration. T (the hit test) lands at its end.
        cast_micros: u64,
        /// Max distance from caster to target position (server-validated).
        max_range: f32,
    },
    /// A replicated projectile entity launched toward the target point;
    /// damage on contact (favor-the-shooter is just collision — no rewind).
    Projectile {
        prefab: &'static str,
        speed: f32,
        damage: i32,
        ttl_secs: f32,
        /// Spawn this far in front of the caster (clears the caster's hitbox).
        spawn_offset: f32,
    },
}

/// The player's left-click attack — their ONLY damage source (Phase 7.5).
pub const BOLT_COOLDOWN_SECS: f32 = 0.6;

const SKILLS: &[SkillDef] = &[
    SkillDef {
        id: "blast",
        cooldown_micros: 3_000_000,
        effect: SkillEffect::Scheduled {
            telegraph_prefab: "telegraph",
            radius: 4.0,
            damage: 25,
            cast_micros: 2_000_000,
            max_range: 15.0,
        },
    },
    SkillDef {
        id: "bolt",
        cooldown_micros: (BOLT_COOLDOWN_SECS * 1e6) as u64,
        effect: SkillEffect::Projectile {
            prefab: "bolt",
            speed: 18.0,
            damage: 12,
            ttl_secs: 1.5,
            spawn_offset: 0.9,
        },
    },
];

pub fn skill(id: &str) -> Option<&'static SkillDef> {
    SKILLS.iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blast_is_scheduled_with_phase4_numbers() {
        let blast = skill("blast").unwrap();
        assert_eq!(blast.cooldown_micros, 3_000_000);
        match blast.effect {
            SkillEffect::Scheduled { radius, damage, cast_micros, max_range, .. } => {
                assert_eq!(radius, 4.0);
                assert_eq!(damage, 25);
                assert_eq!(cast_micros, 2_000_000);
                assert_eq!(max_range, 15.0);
            }
            _ => panic!("blast must stay a scheduled mechanic"),
        }
    }

    #[test]
    fn bolt_is_a_projectile() {
        let bolt = skill("bolt").unwrap();
        assert_eq!(bolt.cooldown_micros, 600_000);
        assert!(matches!(bolt.effect, SkillEffect::Projectile { prefab: "bolt", .. }));
    }
}
