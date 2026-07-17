// CombatStats — the stat block powering the damage formula. Optional on
// either side of a hit: a missing CombatStats is a true no-op modifier, so
// every existing damage source (contact damage, bare projectiles, mechanics)
// stays byte-identical until a prefab actually carries the component.
//
// Damage types (CHARACTER-SYSTEM.md): the Physical→Life→Runes triangle, the
// Divine↔Corrupt opposed pair, Elemental (neutral — defenses apply, no type
// interaction), and True (ignores defense and every multiplier).

/// What flavor a hit is. `#[serde(default)]` everywhere it's carried, so
/// untyped content is Physical.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize)]
pub enum DamageType {
    #[default]
    Physical,
    Life,
    Runes,
    Divine,
    Corrupt,
    Elemental,
    True,
}

/// Multiplier when the attack type beats the defender's affinity.
pub const ADVANTAGE: f32 = 1.3;
/// Multiplier when the defender's affinity beats the attack type.
pub const RESISTED: f32 = 0.75;

/// Attack type vs. defender affinity. Triangle: Physical beats Life beats
/// Runes beats Physical; Divine and Corrupt punish each other both ways;
/// everything else (same type, Elemental, no relation) is neutral.
fn type_multiplier(attack: DamageType, affinity: DamageType) -> f32 {
    use DamageType::*;
    match (attack, affinity) {
        (Physical, Life) | (Life, Runes) | (Runes, Physical) => ADVANTAGE,
        (Life, Physical) | (Runes, Life) | (Physical, Runes) => RESISTED,
        (Divine, Corrupt) | (Corrupt, Divine) => ADVANTAGE,
        _ => 1.0,
    }
}

#[derive(Clone, serde::Deserialize)]
pub struct CombatStats {
    pub power: i32,
    pub defense: i32,
    pub crit_chance: f32,
    pub crit_mult: f32,
    /// The entity's nature — what the triangle tests incoming damage against.
    /// None = neutral target (no type interaction either way).
    #[serde(default)]
    pub affinity: Option<DamageType>,
}

/// Deterministic damage formula, in order: base + attacker.power → crit →
/// type multiplier vs. the defender's affinity → defender.defense → floor 1.
/// True damage skips the type multiplier AND defense (crit still applies —
/// it's offense, not defense). The crit roll is a hash of `roll_seed`
/// compared against crit_chance — callers derive the seed from stable
/// per-hit ids (entity/mechanic ids), never wall-clock or a local RNG,
/// matching DESIGN.md's "no local randomness in gameplay systems" rule.
pub fn compute_damage(
    base: i32,
    damage_type: DamageType,
    attacker: Option<&CombatStats>,
    defender: Option<&CombatStats>,
    roll_seed: u64,
) -> i32 {
    let mut dmg = base;
    if let Some(a) = attacker {
        dmg += a.power;
        if crit_roll(roll_seed) < a.crit_chance {
            dmg = (dmg as f32 * a.crit_mult).round() as i32;
        }
    }
    if damage_type != DamageType::True
        && let Some(d) = defender {
            if let Some(affinity) = d.affinity {
                dmg = (dmg as f32 * type_multiplier(damage_type, affinity)).round() as i32;
            }
            dmg -= d.defense;
        }
    dmg.max(1)
}

/// splitmix64 finalizer — a well-mixed, deterministic [0,1) roll from a u64 seed.
fn crit_roll(seed: u64) -> f32 {
    let mut z = seed.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 11) as f32 / (1u64 << 53) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use DamageType::*;

    fn stats(power: i32, defense: i32, crit_chance: f32, crit_mult: f32) -> CombatStats {
        CombatStats { power, defense, crit_chance, crit_mult, affinity: None }
    }

    fn affine(affinity: DamageType) -> CombatStats {
        CombatStats { power: 0, defense: 0, crit_chance: 0.0, crit_mult: 2.0, affinity: Some(affinity) }
    }

    #[test]
    fn no_stats_is_a_pure_passthrough() {
        assert_eq!(compute_damage(25, Physical, None, None, 0), 25);
        assert_eq!(compute_damage(12, Physical, None, None, 12345), 12);
    }

    #[test]
    fn attacker_power_adds() {
        let atk = stats(10, 0, 0.0, 2.0);
        assert_eq!(compute_damage(20, Physical, Some(&atk), None, 0), 30);
    }

    #[test]
    fn defender_defense_subtracts_floored_at_one() {
        let def = stats(0, 999, 0.0, 2.0);
        assert_eq!(compute_damage(20, Physical, None, Some(&def), 0), 1);
    }

    #[test]
    fn crit_chance_zero_never_crits() {
        let atk = stats(0, 0, 0.0, 2.0);
        for seed in 0..1000u64 {
            assert_eq!(compute_damage(20, Physical, Some(&atk), None, seed), 20);
        }
    }

    #[test]
    fn crit_chance_one_always_crits() {
        let atk = stats(0, 0, 1.0, 2.0);
        for seed in 0..1000u64 {
            assert_eq!(compute_damage(20, Physical, Some(&atk), None, seed), 40);
        }
    }

    #[test]
    fn triangle_advantage_every_edge() {
        // 20 × 1.3 = 26.
        assert_eq!(compute_damage(20, Physical, None, Some(&affine(Life)), 0), 26);
        assert_eq!(compute_damage(20, Life, None, Some(&affine(Runes)), 0), 26);
        assert_eq!(compute_damage(20, Runes, None, Some(&affine(Physical)), 0), 26);
    }

    #[test]
    fn triangle_resisted_every_edge() {
        // 20 × 0.75 = 15.
        assert_eq!(compute_damage(20, Life, None, Some(&affine(Physical)), 0), 15);
        assert_eq!(compute_damage(20, Runes, None, Some(&affine(Life)), 0), 15);
        assert_eq!(compute_damage(20, Physical, None, Some(&affine(Runes)), 0), 15);
    }

    #[test]
    fn divine_and_corrupt_punish_both_ways() {
        assert_eq!(compute_damage(20, Divine, None, Some(&affine(Corrupt)), 0), 26);
        assert_eq!(compute_damage(20, Corrupt, None, Some(&affine(Divine)), 0), 26);
    }

    #[test]
    fn elemental_is_neutral_but_defense_applies() {
        let mut def = affine(Physical);
        def.defense = 5;
        assert_eq!(compute_damage(20, Elemental, None, Some(&def), 0), 15);
    }

    #[test]
    fn true_damage_ignores_defense_and_affinity() {
        let mut def = affine(Runes); // would resist Physical; irrelevant for True
        def.defense = 999;
        assert_eq!(compute_damage(20, True, None, Some(&def), 0), 20);
    }

    #[test]
    fn same_type_and_no_affinity_are_neutral() {
        assert_eq!(compute_damage(20, Physical, None, Some(&affine(Physical)), 0), 20);
        assert_eq!(compute_damage(20, Divine, None, Some(&stats(0, 0, 0.0, 2.0)), 0), 20);
    }
}
