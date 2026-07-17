//! Extracts `PointLight` components into the GPU point-light array each
//! display frame: lerped position, offset rotated by the entity, capped to
//! the `MAX_POINT_LIGHTS` nearest the camera focus, deterministic flicker
//! applied. Writes the point-light slice of `light_state` and uploads it —
//! field ownership is shared with `facade::set_light`/`set_fog` (every
//! writer owns its own fields and re-uploads the whole struct).

use crate::camera::{GpuPointLight, MAX_POINT_LIGHTS};
use crate::state::RendererState;
use engine_app::dev_stats::DevStats;
use engine_app::scheduler::{InterpolationAlpha, System};
use engine_core::components::{PointLight, PreviousTransform, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;

pub(crate) struct LightCandidate {
    pub position:  Vec3,
    pub color:     Vec3,
    pub intensity: f32,
    pub radius:    f32,
}

/// Deterministic per-light flicker: two off-frequency sines so the
/// modulation never repeats on a visible cycle. `amount` is the fraction of
/// intensity `PointLight.flicker` may remove; `phase` de-syncs lights that
/// share the same `time`. `n` ranges over [0, 1], so the result stays in
/// `[1 - amount, 1]`.
pub(crate) fn flicker_factor(time: f32, phase: f32, amount: f32) -> f32 {
    let n = 0.5 + 0.3 * (13.0 * time + phase).sin() + 0.2 * (7.3 * time + 1.7 * phase).sin();
    1.0 - amount * n
}

/// Keep the `MAX_POINT_LIGHTS` candidates nearest `focus`, mapped into the
/// GPU struct array; unfilled tail slots stay zeroed.
pub(crate) fn select_point_lights(
    candidates: &mut [LightCandidate],
    focus: Vec3,
) -> ([GpuPointLight; MAX_POINT_LIGHTS as usize], u32) {
    candidates.sort_by(|a, b| {
        a.position.distance_squared(focus).total_cmp(&b.position.distance_squared(focus))
    });
    let mut points = [GpuPointLight { position: [0.0; 3], radius: 0.0, color: [0.0; 3], intensity: 0.0 };
        MAX_POINT_LIGHTS as usize];
    let count = candidates.len().min(MAX_POINT_LIGHTS as usize);
    for (slot, candidate) in points.iter_mut().zip(candidates.iter().take(count)) {
        *slot = GpuPointLight {
            position:  candidate.position.to_array(),
            radius:    candidate.radius,
            color:     candidate.color.to_array(),
            intensity: candidate.intensity,
        };
    }
    (points, count as u32)
}

/// Extracts every `PointLight` entity into `light_state.points` each frame
/// and uploads the light buffer. Headless (no `RendererState`) is a no-op.
#[derive(Default)]
pub struct PointLightSyncSystem {
    time:       f32,
    candidates: Vec<LightCandidate>,
}

impl System for PointLightSyncSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        if resources.get::<RendererState>().is_none() {
            return;
        }
        self.time += delta;
        let alpha = resources.get::<InterpolationAlpha>().map(|a| a.0).unwrap_or(1.0);

        self.candidates.clear();
        for (entity, transform, prev, light) in world
            .query::<(hecs::Entity, &Transform, Option<&PreviousTransform>, &PointLight)>()
            .iter()
        {
            let render_pos = match prev {
                Some(p) => p.position.lerp(transform.position, alpha),
                None    => transform.position,
            };
            let position = render_pos + transform.rotation * light.offset;
            let phase = entity.id() as f32 * 2.399963;
            let intensity = light.intensity * flicker_factor(self.time, phase, light.flicker);
            self.candidates.push(LightCandidate {
                position,
                color: light.color,
                intensity,
                radius: light.radius,
            });
        }

        let state = resources.expect_mut::<RendererState>();
        let focus = state.camera.target;
        let (points, count) = select_point_lights(&mut self.candidates, focus);
        state.light_state.points = points;
        state.light_state.point_count = count;
        state.queue.write_buffer(&state.light_buffer, 0, bytemuck::cast_slice(&[state.light_state]));

        if let Some(stats) = resources.get_mut::<DevStats>() {
            stats.set("lights", format!("{count}/{MAX_POINT_LIGHTS}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `radius` doubles as a distance marker so kept/dropped candidates are
    /// identifiable after selection.
    fn candidate_at(distance: f32) -> LightCandidate {
        LightCandidate {
            position:  Vec3::new(distance, 0.0, 0.0),
            color:     Vec3::ONE,
            intensity: 1.0,
            radius:    distance,
        }
    }

    #[test]
    fn select_point_lights_keeps_the_nearest_within_cap() {
        let mut candidates: Vec<LightCandidate> =
            (0..20).map(|i| candidate_at(i as f32 + 1.0)).collect();
        let (points, count) = select_point_lights(&mut candidates, Vec3::ZERO);
        assert_eq!(count, MAX_POINT_LIGHTS);

        let kept_max: f32 = points.iter().map(|p| p.radius).fold(f32::MIN, f32::max);
        let dropped_min = 17.0; // distances 1..=16 are kept, 17..=20 are dropped
        assert!(
            kept_max < dropped_min,
            "every kept light ({kept_max}) must be nearer than every dropped light ({dropped_min})"
        );
    }

    #[test]
    fn select_point_lights_below_cap_zero_fills_the_tail() {
        let mut candidates: Vec<LightCandidate> = (0..3).map(|i| candidate_at(i as f32 + 1.0)).collect();
        let (points, count) = select_point_lights(&mut candidates, Vec3::ZERO);
        assert_eq!(count, 3);
        for p in points.iter().take(3) {
            assert_ne!(p.radius, 0.0);
        }
        for p in points.iter().skip(3) {
            assert_eq!(p.radius, 0.0);
            assert_eq!(p.intensity, 0.0);
        }
    }

    #[test]
    fn flicker_factor_is_identity_when_amount_is_zero() {
        for i in 0..10 {
            let t = i as f32 * 0.37;
            assert_eq!(flicker_factor(t, 0.0, 0.0), 1.0);
        }
    }

    #[test]
    fn flicker_factor_stays_bounded_and_varies_with_amount() {
        let amount = 0.6;
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for i in 0..100 {
            let t = i as f32 * 0.083;
            let f = flicker_factor(t, 0.0, amount);
            assert!(
                (1.0 - amount - 1e-6..=1.0 + 1e-6).contains(&f),
                "flicker_factor {f} out of [{}, 1.0]", 1.0 - amount
            );
            min = min.min(f);
            max = max.max(f);
        }
        assert!(min < max, "flicker_factor should vary over time, got constant {min}");
    }
}
