// Core components — pure data, no behavior
//
// These are the engine's built-in component types. Games add their own
// components in their own crates and register them with the engine.
//
// Visual size, collision shape, and grid footprint are fully independent:
//   - Transform.scale      → visual size
//   - Hitbox.shape         → collision bounds (can differ from visual)
//   - CellOccupant.cells   → grid cells owned (can differ from both)

use glam::{Mat4, Quat, Vec3};
use smallvec::SmallVec;

// ── Spatial ──────────────────────────────────────────────────────────────────

#[derive(Clone, serde::Deserialize)]
#[serde(default)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale:    Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE }
    }
}

impl Transform {
    pub fn new(position: Vec3) -> Self {
        Self { position, rotation: Quat::IDENTITY, scale: Vec3::ONE }
    }

    pub fn to_model_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }
}

#[derive(Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Velocity {
    pub linear: Vec3,  // units per second
}

/// Saved at the start of each fixed step; used by RenderSyncSystem to lerp between
/// the previous and current position so rendering stays smooth at any display rate.
pub struct PreviousTransform {
    pub position: Vec3,
}

// Which spatial grid cells this entity currently occupies.
// Updated each frame in Phase::PostUpdate after movement resolves.
// A 1x1 entity → 1 cell. A 2x2 entity → 4 cells.
pub struct CellOccupant {
    pub cells: SmallVec<[GridCell; 4]>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GridCell {
    pub col: i32,
    pub row: i32,
}

// ── Gameplay ─────────────────────────────────────────────────────────────────

#[derive(Clone, serde::Deserialize)]
pub struct Health {
    pub current: i32,
    pub max:     i32,
}

impl Health {
    pub fn new(max: i32) -> Self { Self { current: max, max } }
    pub fn is_dead(&self) -> bool { self.current <= 0 }
}

// ── Collision ─────────────────────────────────────────────────────────────────

#[derive(Clone, serde::Deserialize)]
pub struct Hitbox {
    pub shape: CollisionShape,
}

/// Participates in physical separation — entities with this component are pushed
/// apart when they overlap rather than passing through each other.
#[derive(Clone, serde::Deserialize)]
pub struct Solid;

/// A Solid that never yields: separation pushes only the other party
/// (buildings, posted NPCs). Two Anchored solids never push each other.
#[derive(Clone, serde::Deserialize)]
pub struct Anchored;

#[derive(Clone, serde::Deserialize)]
pub enum CollisionShape {
    Aabb { half_extents: Vec3 },
    Sphere { radius: f32 },
}

// ── Rendering ─────────────────────────────────────────────────────────────────

#[derive(Clone, serde::Deserialize)]
pub struct RenderShape {
    pub shape: RenderShapeType,
    pub color: Vec3,
    // Render slot is tracked by engine-renderer via a separate InstanceSlot component.
    // RenderShape describes *what* to draw — not renderer-internal bookkeeping.
}

#[derive(Clone, Copy, serde::Deserialize)]
pub enum RenderShapeType {
    Cube,
    Sphere,
    Diamond,
    RoundedBox { corner_radius: f32 },
    Cylinder,
    Capsule,
    /// Passthrough for experiment-defined GPU shape types.
    /// shape_type is sent raw to the shader; params become shape_params vec4.
    Custom { shape_type: u32, params: [f32; 4] },
}

impl Default for RenderShapeType {
    fn default() -> Self { Self::Cube }
}

/// One element of a composed visual. Offset, rotation, and scale are in the
/// parent entity's local space. `rotation` defaults to identity so existing
/// prefab RON keeps parsing unchanged.
#[derive(Clone, serde::Deserialize)]
pub struct SubShape {
    pub shape:  RenderShapeType,
    pub offset: Vec3,
    #[serde(default)]
    pub rotation: Quat,
    pub scale:  Vec3,
    pub color:  Vec3,
}

/// Replaces `RenderShape` for multi-part entities. Attach instead of `RenderShape`.
/// The renderer emits one `SdfInstance` per sub-shape each frame.
#[derive(Clone, serde::Deserialize)]
pub struct ShapeGroup {
    pub shapes: Vec<SubShape>,
}

/// Draw a glTF model instead of primitive shapes. `asset` is a path relative
/// to the process working directory (content convention: "content/models/…").
/// `tint` multiplies the material's base color; white = unchanged.
#[derive(Clone, serde::Deserialize)]
pub struct RenderMesh {
    pub asset: String,
    #[serde(default = "white")]
    pub tint: Vec3,
}

fn white() -> Vec3 { Vec3::ONE }

/// Skeletal animation playback state for a skinned `RenderMesh` entity.
/// Code-inserted only (never RON): the engine attaches a default and the
/// game's controller drives it via `transition_to`. Sampling/skinning lives in
/// engine-renderer; this is just the cursor + crossfade bookkeeping.
#[derive(Clone)]
pub struct AnimationPlayer {
    /// Name of the clip currently playing (matched against the asset's clips;
    /// empty or unknown falls back to the first clip).
    pub clip:    String,
    /// Playback cursor, seconds into `clip`.
    pub time:    f32,
    /// Time multiplier — >1 speeds the clip up (e.g. faster strides at speed).
    pub speed:   f32,
    /// Loop the clip (locomotion) vs. hold the last frame (death/one-shots).
    pub looping: bool,
    /// The outgoing clip during a crossfade; None once the blend completes.
    pub prev:      Option<PrevClip>,
    /// Crossfade duration in seconds.
    pub blend_dur: f32,
    /// Elapsed crossfade time; blend weight = (blend_t / blend_dur).
    pub blend_t:   f32,
}

/// A frozen snapshot of the clip being blended out of.
#[derive(Clone)]
pub struct PrevClip {
    pub clip: String,
    pub time: f32,
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            clip:      String::new(),
            time:      0.0,
            speed:     1.0,
            looping:   true,
            prev:      None,
            blend_dur: 0.15,
            blend_t:   0.0,
        }
    }
}

impl AnimationPlayer {
    /// Smoothly cross-fade to `clip` over `blend` seconds. A no-op if already
    /// playing it (so calling this every frame from a controller is cheap and
    /// doesn't restart the clip). The outgoing clip is frozen at its current
    /// time and blended out.
    pub fn transition_to(&mut self, clip: &str, looping: bool, blend: f32) {
        if self.clip == clip {
            self.looping = looping;
            return;
        }
        self.prev = Some(PrevClip { clip: std::mem::take(&mut self.clip), time: self.time });
        self.clip = clip.to_owned();
        self.time = 0.0;
        self.looping = looping;
        self.blend_dur = blend.max(1e-4);
        self.blend_t = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn transform_new_identity_rotation_and_scale() {
        let t = Transform::new(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.rotation, glam::Quat::IDENTITY);
        assert_eq!(t.scale, Vec3::ONE);
    }

    #[test]
    fn transform_model_matrix_round_trips_position() {
        let t = Transform::new(Vec3::new(5.0, 0.0, -3.0));
        let m = t.to_model_matrix();
        // w_axis is the translation column in a TRS matrix
        assert_eq!(m.w_axis.truncate(), Vec3::new(5.0, 0.0, -3.0));
    }

    #[test]
    fn health_new_starts_full() {
        let h = Health::new(100);
        assert_eq!(h.current, 100);
        assert_eq!(h.max, 100);
        assert!(!h.is_dead());
    }

    #[test]
    fn health_is_dead_at_zero() {
        let h = Health { current: 0, max: 100 };
        assert!(h.is_dead());
    }

    #[test]
    fn health_is_dead_below_zero() {
        // Damage can overshoot — still considered dead
        let h = Health { current: -5, max: 100 };
        assert!(h.is_dead());
    }

    #[test]
    fn cell_occupant_smallvec_stays_inline_for_four_cells() {
        // SmallVec<[GridCell; 4]> must not heap-allocate for ≤4 cells
        let cells: smallvec::SmallVec<[GridCell; 4]> = (0..4)
            .map(|i| GridCell { col: i, row: 0 })
            .collect();
        assert!(!cells.spilled());
    }

    #[test]
    fn cell_occupant_spills_beyond_four_cells() {
        let cells: smallvec::SmallVec<[GridCell; 4]> = (0..5)
            .map(|i| GridCell { col: i, row: 0 })
            .collect();
        assert!(cells.spilled());
    }

    #[test]
    fn transform_model_matrix_applies_scale() {
        let mut t = Transform::new(Vec3::ZERO);
        t.scale = Vec3::new(2.0, 3.0, 4.0);
        let m = t.to_model_matrix();
        // With identity rotation, the diagonal of the upper-left 3x3 == scale
        assert!((m.x_axis.x - 2.0).abs() < 1e-6);
        assert!((m.y_axis.y - 3.0).abs() < 1e-6);
        assert!((m.z_axis.z - 4.0).abs() < 1e-6);
    }

    #[test]
    fn transform_model_matrix_applies_rotation() {
        use std::f32::consts::FRAC_PI_2;
        let mut t = Transform::new(Vec3::ZERO);
        t.rotation = Quat::from_rotation_y(FRAC_PI_2);
        let m = t.to_model_matrix();
        // 90° Y rotation: local X-axis maps to world -Z
        assert!((m.x_axis.x - 0.0).abs() < 1e-6);
        assert!((m.x_axis.z - (-1.0)).abs() < 1e-6);
    }
}
