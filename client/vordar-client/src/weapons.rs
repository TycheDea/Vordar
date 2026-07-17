// Placeholder sword + shield riding the character's hand socket bones —
// blocky procedural meshes whose only job is showing how the Mixamo clips
// align with held weapons (the real gear pipeline comes with skinned
// attachments later).
//
// WeaponAttachSystem runs in Phase::RenderSync after MeshRenderSyncSystem
// (the VfxSystem slot): it reads the freshly rebuilt `SocketTransforms` and
// copies each socket's world matrix onto its weapon entity's Transform, so a
// weapon trails its hand by at most one frame. Weapons spawn lazily for every
// animated mesh character (`LocomotionClips`) and despawn with their owner.

use crate::locomotion::LocomotionClips;
use crate::presentation::HudHidden;
use engine_app::scheduler::System;
use engine_core::components::{RenderMesh, Transform};
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use engine_renderer::mesh::{MaterialData, MeshData, PrimitiveData};
use engine_renderer::tangent::generate_tangents;
use engine_renderer::{MeshVertex, SocketTransforms};
use glam::{Mat4, Vec3};
use hecs::Entity;

const SWORD_KEY:  &str = "weapon:sword";
const SHIELD_KEY: &str = "weapon:shield";

/// Marks a weapon entity as riding `owner`'s socket `bone`.
pub struct WeaponAttachment {
    pub owner: Entity,
    pub bone:  &'static str,
    /// Grip-alignment tuning knob, composed onto the socket matrix. Identity
    /// until the feel-check says otherwise.
    pub local: Mat4,
}

/// One axis-aligned cuboid appended to the buffers: 6 faceted faces, 4 verts
/// each, CCW seen from outside, quad UVs per face.
fn push_cuboid(
    positions: &mut Vec<[f32; 3]>,
    normals:   &mut Vec<[f32; 3]>,
    uvs:       &mut Vec<[f32; 2]>,
    indices:   &mut Vec<u32>,
    center: Vec3,
    half:   Vec3,
) {
    // (normal, u, v) with u × v = normal, so [0,1,2, 0,2,3] winds CCW
    // around the outward normal.
    const FACES: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X,     Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y,     Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z,     Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    for (n, u, v) in FACES {
        let base = positions.len() as u32;
        let fc = center + n * (n.abs().dot(half));
        let hu = u * u.abs().dot(half);
        let hv = v * v.abs().dot(half);
        for (su, sv, uv) in [
            (-1.0, -1.0, [0.0, 0.0]),
            (1.0, -1.0, [1.0, 0.0]),
            (1.0, 1.0, [1.0, 1.0]),
            (-1.0, 1.0, [0.0, 1.0]),
        ] {
            positions.push((fc + hu * su + hv * sv).to_array());
            normals.push(n.to_array());
            uvs.push(uv);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Cuboid list → one solid-color primitive.
fn primitive(parts: &[(Vec3, Vec3)], color: [f32; 4], metallic: f32, roughness: f32) -> PrimitiveData {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (center, half) in parts {
        push_cuboid(&mut positions, &mut normals, &mut uvs, &mut indices, *center, *half);
    }
    let tangents = generate_tangents(&positions, &normals, &uvs, &indices);
    let vertices = positions
        .iter()
        .zip(&normals)
        .zip(&uvs)
        .zip(&tangents)
        .map(|(((p, n), uv), t)| MeshVertex { position: *p, normal: *n, uv: *uv, tangent: *t })
        .collect();
    PrimitiveData {
        vertices,
        indices,
        material: MaterialData {
            base_color_factor: color,
            metallic_factor: metallic,
            roughness_factor: roughness,
            ..Default::default()
        },
        skin: None,
    }
}

const STEEL: ([f32; 4], f32, f32) = ([0.55, 0.56, 0.58, 1.0], 0.85, 0.35);
const WOOD:  ([f32; 4], f32, f32) = ([0.24, 0.15, 0.08, 1.0], 0.0, 0.8);

/// Arming sword, grip at the origin, blade along +Y (socket bones point +Y
/// along the hand bone). ~1.1 m overall for the 1.75 m body.
pub fn sword_mesh() -> MeshData {
    let steel = primitive(
        &[
            (Vec3::new(0.0, 0.545, 0.0), Vec3::new(0.035, 0.425, 0.008)), // blade
            (Vec3::new(0.0, 0.10, 0.0), Vec3::new(0.11, 0.02, 0.035)),    // crossguard
            (Vec3::new(0.0, -0.16, 0.0), Vec3::new(0.035, 0.025, 0.035)), // pommel
        ],
        STEEL.0, STEEL.1, STEEL.2,
    );
    let grip = primitive(
        &[(Vec3::new(0.0, -0.02, 0.0), Vec3::new(0.022, 0.11, 0.022))],
        WOOD.0, WOOD.1, WOOD.2,
    );
    MeshData { primitives: vec![steel, grip], skeleton: None, clips: Vec::new() }
}

/// Round-ish shield slab, face normal +Z, strapped at the origin.
pub fn shield_mesh() -> MeshData {
    let plate = primitive(
        &[(Vec3::new(0.0, 0.0, 0.04), Vec3::new(0.22, 0.30, 0.015))],
        WOOD.0, WOOD.1, WOOD.2,
    );
    let boss = primitive(
        &[(Vec3::new(0.0, 0.0, 0.065), Vec3::new(0.055, 0.055, 0.018))],
        STEEL.0, STEEL.1, STEEL.2,
    );
    MeshData { primitives: vec![plate, boss], skeleton: None, clips: Vec::new() }
}

/// Spawns sword/shield for animated mesh characters and glues them to the
/// hand sockets each frame. See module docs for scheduling.
#[derive(Default)]
pub struct WeaponAttachSystem {
    tried:        bool,
    meshes_ready: bool,
}

impl System for WeaponAttachSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        if !self.tried {
            self.tried = true;
            self.meshes_ready =
                engine_renderer::register_procedural_mesh(SWORD_KEY, sword_mesh(), resources)
                    && engine_renderer::register_procedural_mesh(SHIELD_KEY, shield_mesh(), resources);
        }

        // Lazily arm every animated mesh character that has no weapons yet.
        if self.meshes_ready {
            let armed: std::collections::HashSet<Entity> = world
                .query::<(Entity, &WeaponAttachment)>()
                .iter()
                .map(|(_, a)| a.owner)
                .collect();
            let unarmed: Vec<Entity> = world
                .query::<(Entity, &LocomotionClips)>()
                .iter()
                .filter(|(e, _)| !armed.contains(e))
                .map(|(e, _)| e)
                .collect();
            for owner in unarmed {
                for (key, bone) in [(SWORD_KEY, "handslot.r"), (SHIELD_KEY, "handslot.l")] {
                    world.spawn((
                        Transform::default(),
                        RenderMesh { asset: key.into(), tint: Vec3::ONE },
                        WeaponAttachment { owner, bone, local: Mat4::IDENTITY },
                        HudHidden,
                    ));
                }
            }
        }

        // Follow the sockets; orphaned weapons (owner despawned) go away.
        struct Follow {
            entity: Entity,
            owner:  Entity,
            bone:   &'static str,
            local:  Mat4,
        }
        let mut follows: Vec<Follow> = Vec::new();
        let mut orphans: Vec<Entity> = Vec::new();
        for (entity, att) in world.query::<(Entity, &WeaponAttachment)>().iter() {
            if world.contains(att.owner) {
                follows.push(Follow { entity, owner: att.owner, bone: att.bone, local: att.local });
            } else {
                orphans.push(entity);
            }
        }
        for f in follows {
            let Some(socket) = resources
                .get::<SocketTransforms>()
                .and_then(|s| s.0.get(&f.owner).and_then(|bones| bones.get(f.bone)).copied())
            else {
                continue; // not posed this frame: keep the last transform
            };
            // The socket matrix folds in the armature's baked cm→m scale;
            // weapons are authored in metres, so take rotation + translation
            // only.
            let (_, rotation, position) = (socket * f.local).to_scale_rotation_translation();
            if let Ok(mut t) = world.get::<&mut Transform>(f.entity) {
                t.position = position;
                t.rotation = rotation;
                t.scale = Vec3::ONE;
            }
        }
        if !orphans.is_empty()
            && let Some(queue) = resources.get_mut::<DespawnQueue>() {
                for entity in orphans {
                    queue.push(entity, None);
                }
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    use std::collections::HashMap;

    #[test]
    fn weapon_meshes_are_well_formed() {
        for mesh in [sword_mesh(), shield_mesh()] {
            assert_eq!(mesh.primitives.len(), 2);
            for p in &mesh.primitives {
                assert!(!p.vertices.is_empty());
                assert_eq!(p.indices.len() % 3, 0);
                assert!(p.indices.iter().all(|&i| (i as usize) < p.vertices.len()));
                for v in &p.vertices {
                    let n = Vec3::from_array(v.normal);
                    assert!((n.length() - 1.0).abs() < 1e-4, "unit normals");
                    assert!(v.tangent.iter().all(|c| c.is_finite()), "finite tangents");
                }
            }
        }
    }

    #[test]
    fn cuboid_faces_wind_ccw_around_their_normal() {
        let (mut pos, mut nrm, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        push_cuboid(&mut pos, &mut nrm, &mut uv, &mut idx, Vec3::ZERO, Vec3::ONE);
        for tri in idx.chunks(3) {
            let [a, b, c] = [pos[tri[0] as usize], pos[tri[1] as usize], pos[tri[2] as usize]]
                .map(Vec3::from_array);
            let face_normal = Vec3::from_array(nrm[tri[0] as usize]);
            assert!(
                (b - a).cross(c - a).dot(face_normal) > 0.0,
                "triangle winds CCW seen from outside"
            );
        }
    }

    /// The follow path: a fabricated socket matrix (with the armature's baked
    /// 0.01 scale) lands on the weapon as rotation + translation, scale ONE.
    #[test]
    fn weapon_follows_socket_rotation_and_translation() {
        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(DespawnQueue::new());

        let owner = world.spawn((Transform::default(), LocomotionClips::default()));
        let weapon = world.spawn((
            Transform::default(),
            WeaponAttachment { owner, bone: "handslot.r", local: Mat4::IDENTITY },
        ));

        let rot = Quat::from_rotation_y(1.2);
        let socket = Mat4::from_scale_rotation_translation(
            Vec3::splat(0.01),
            rot,
            Vec3::new(1.0, 1.4, -2.0),
        );
        let mut sockets = SocketTransforms::default();
        sockets.0.insert(owner, HashMap::from([("handslot.r".to_string(), socket)]));
        resources.insert(sockets);

        // tried=true skips mesh registration (headless) but not the follow.
        let mut sys = WeaponAttachSystem { tried: true, meshes_ready: false };
        sys.run(&mut world, &mut resources, 0.016);

        let t = world.get::<&Transform>(weapon).unwrap();
        assert!(t.position.abs_diff_eq(Vec3::new(1.0, 1.4, -2.0), 1e-5));
        assert!(t.rotation.abs_diff_eq(rot, 1e-4), "socket rotation copied");
        assert!(t.scale.abs_diff_eq(Vec3::ONE, 1e-6), "baked cm scale NOT copied");
    }

    #[test]
    fn orphaned_weapon_is_despawned_with_its_owner() {
        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(DespawnQueue::new());
        resources.insert(SocketTransforms::default());

        let owner = world.spawn((Transform::default(), LocomotionClips::default()));
        world.spawn((
            Transform::default(),
            WeaponAttachment { owner, bone: "handslot.r", local: Mat4::IDENTITY },
        ));

        let mut sys = WeaponAttachSystem { tried: true, meshes_ready: false };
        sys.run(&mut world, &mut resources, 0.016);
        assert!(resources.get::<DespawnQueue>().unwrap().0.is_empty(), "owner alive");

        world.despawn(owner).unwrap();
        sys.run(&mut world, &mut resources, 0.016);
        assert_eq!(resources.get::<DespawnQueue>().unwrap().0.len(), 1, "weapon queued");
    }
}
