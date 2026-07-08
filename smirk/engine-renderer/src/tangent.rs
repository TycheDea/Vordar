// Tangent generation for meshes whose glTF ships no TANGENT accessor — VQ-C4.
//
// Per-triangle tangent/bitangent from UV derivatives (Lengyel), accumulated
// per vertex, then Gram-Schmidt orthonormalized against the vertex normal.
// The w component stores handedness (±1), matching the glTF TANGENT
// convention: bitangent = cross(normal, tangent.xyz) * tangent.w.
//
// Pure CPU, no GPU types — unit-tested below.

use glam::Vec3;

/// Generate one vec4 tangent per vertex. Degenerate triangles (zero UV area)
/// contribute nothing; vertices no triangle touches get a zero tangent, which
/// the shaders treat as "no tangent basis — use the vertex normal".
pub fn generate_tangents(
    positions: &[[f32; 3]],
    normals:   &[[f32; 3]],
    uvs:       &[[f32; 2]],
    indices:   &[u32],
) -> Vec<[f32; 4]> {
    let n = positions.len();
    let mut tan = vec![Vec3::ZERO; n];
    let mut bitan = vec![Vec3::ZERO; n];

    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= n || i1 >= n || i2 >= n {
            continue;
        }
        let p0 = Vec3::from(positions[i0]);
        let p1 = Vec3::from(positions[i1]);
        let p2 = Vec3::from(positions[i2]);
        let (u0, u1, u2) = (uvs[i0], uvs[i1], uvs[i2]);

        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let du1 = u1[0] - u0[0];
        let dv1 = u1[1] - u0[1];
        let du2 = u2[0] - u0[0];
        let dv2 = u2[1] - u0[1];

        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-12 {
            continue; // degenerate UVs
        }
        let r = 1.0 / det;
        let t = (e1 * dv2 - e2 * dv1) * r;
        let b = (e2 * du1 - e1 * du2) * r;

        for &i in &[i0, i1, i2] {
            tan[i] += t;
            bitan[i] += b;
        }
    }

    (0..n)
        .map(|i| {
            let nrm = Vec3::from(normals[i]);
            let t = tan[i];
            // Gram-Schmidt: project out the normal component.
            let ortho = t - nrm * nrm.dot(t);
            let Some(txyz) = ortho.try_normalize() else {
                return [0.0; 4];
            };
            let w = if nrm.cross(txyz).dot(bitan[i]) < 0.0 { -1.0 } else { 1.0 };
            [txyz.x, txyz.y, txyz.z, w]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A unit quad in the XY plane, normal +Z, with identity-like UVs:
    // u grows with +X, v grows with +Y.
    fn quad() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
        (
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0.0, 0.0, 1.0]; 4],
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    #[test]
    fn quad_tangent_follows_u_axis() {
        let (p, nrm, uv, idx) = quad();
        let tangents = generate_tangents(&p, &nrm, &uv, &idx);
        for t in &tangents {
            let txyz = Vec3::new(t[0], t[1], t[2]);
            assert!(txyz.abs_diff_eq(Vec3::X, 1e-5), "u axis is +X, got {txyz}");
        }
    }

    #[test]
    fn quad_tangents_are_orthonormal_to_normals() {
        let (p, nrm, uv, idx) = quad();
        let tangents = generate_tangents(&p, &nrm, &uv, &idx);
        for (t, n) in tangents.iter().zip(nrm.iter()) {
            let txyz = Vec3::new(t[0], t[1], t[2]);
            let nv = Vec3::from(*n);
            assert!((txyz.length() - 1.0).abs() < 1e-5, "unit length");
            assert!(txyz.dot(nv).abs() < 1e-5, "tangent ⊥ normal");
        }
    }

    #[test]
    fn handedness_flips_with_mirrored_uvs() {
        let (p, nrm, mut uv, idx) = quad();
        let plain = generate_tangents(&p, &nrm, &uv, &idx);
        assert!(plain.iter().all(|t| t[3] == 1.0), "standard UVs are right-handed");

        // Mirror v: v' = 1 - v. Bitangent flips, handedness goes negative.
        for t in uv.iter_mut() {
            t[1] = 1.0 - t[1];
        }
        let mirrored = generate_tangents(&p, &nrm, &uv, &idx);
        assert!(mirrored.iter().all(|t| t[3] == -1.0), "mirrored v flips handedness");
    }

    #[test]
    fn degenerate_uvs_yield_zero_tangent() {
        let (p, nrm, _, idx) = quad();
        let uv = vec![[0.5, 0.5]; 4]; // zero UV area everywhere
        let tangents = generate_tangents(&p, &nrm, &uv, &idx);
        assert!(tangents.iter().all(|t| *t == [0.0; 4]));
    }
}
