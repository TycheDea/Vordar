// Ported from Intel's XeGTAO (https://github.com/GameTechDev/XeGTAO,
// Copyright (C) 2016-2021, Intel Corporation, SPDX-License-Identifier: MIT)
// by way of Bevy's WGSL port (bevy_pbr::ssao, MIT OR Apache-2.0).
//
// GTAO: three compute passes over the depth prepass — prefilter_depth
// (linearize into a 5-mip chain), gtao (horizon-slice visibility + packed
// depth-difference edges), denoise (edge-aware 3×3 spatial filter) — ending
// in the full-res AO texture shade_pbr multiplies into IBL ambient.
//
// All geometry math runs in world space against the camera's
// (right, up, forward) basis: the camera uniform carries no view/projection
// split, and routing every reconstruction through view_proj/inv_view_proj
// keeps one code path valid for both the perspective and orthographic
// projection modes. Depths in the mip chain are linear distances along
// camera forward, in metres; SKY_SENTINEL marks no-geometry texels.

//#include "snippets/scene_uniforms.wgsl"

const PI:      f32 = 3.1415926535;
const HALF_PI: f32 = 1.5707963268;

// XeGTAO defaults: EffectRadius 0.5 × RadiusMultiplier 1.457, falloff over
// the outer 0.615 of the radius, 3 slices × 3 samples per slice side.
const EFFECT_RADIUS: f32 = 0.5 * 1.457;
const FALLOFF_RANGE_FRACTION: f32 = 0.615;
const SLICE_COUNT: f32 = 3.0;
const SAMPLES_PER_SLICE_SIDE: f32 = 3.0;

/// Written to the depth mips where the prepass has no geometry (cleared far
/// plane); large enough that the falloff weight of any real-geometry sample
/// against it is exactly zero.
const SKY_SENTINEL: f32 = 1e9;
const SKY_THRESHOLD: f32 = 1e8;

const HILBERT_WIDTH: i32 = 64;

// ── Shared reconstruction ────────────────────────────────────────────────────

fn camera_forward() -> vec3<f32> {
    return normalize(cross(camera.up.xyz, camera.right.xyz));
}

/// World-space ray through the screen point: `origin` on the near plane and
/// an (unnormalized) direction toward the far plane. Derived from
/// inv_view_proj columns so one matrix product covers both endpoints.
struct PixelRay {
    origin: vec3<f32>,
    dir:    vec3<f32>,
}

fn pixel_ray(uv: vec2<f32>) -> PixelRay {
    let ndc  = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - 2.0 * uv.y);
    let near = camera.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let mid  = near + camera.inv_view_proj[2] * 0.5;
    let p0   = near.xyz / near.w;
    return PixelRay(p0, mid.xyz / mid.w - p0);
}

/// The point on `ray` whose linear depth (distance along `forward` from the
/// eye) is `depth`.
fn world_on_ray(ray: PixelRay, forward: vec3<f32>, depth: f32) -> vec3<f32> {
    let t = (depth - dot(ray.origin - camera.eye.xyz, forward)) / dot(ray.dir, forward);
    return ray.origin + ray.dir * t;
}

fn world_to_uv(world: vec3<f32>) -> vec2<f32> {
    let clip = camera.view_proj * vec4<f32>(world, 1.0);
    let ndc  = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

// ── Pass 1: depth prefilter (linearize + 5-mip chain) ────────────────────────

@group(1) @binding(0) var t_src_depth: texture_depth_2d;
@group(1) @binding(1) var t_mip0: texture_storage_2d<r32float, write>;
@group(1) @binding(2) var t_mip1: texture_storage_2d<r32float, write>;
@group(1) @binding(3) var t_mip2: texture_storage_2d<r32float, write>;
@group(1) @binding(4) var t_mip3: texture_storage_2d<r32float, write>;
@group(1) @binding(5) var t_mip4: texture_storage_2d<r32float, write>;

fn linear_depth_at(coord: vec2<i32>, dims: vec2<i32>, forward: vec3<f32>) -> f32 {
    let c = clamp(coord, vec2<i32>(0), dims - 1);
    let ndc_depth = textureLoad(t_src_depth, c, 0);
    if (ndc_depth >= 1.0) {
        return SKY_SENTINEL;
    }
    let uv = (vec2<f32>(c) + 0.5) / vec2<f32>(dims);
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - 2.0 * uv.y, ndc_depth, 1.0);
    let world = camera.inv_view_proj * ndc;
    return dot(world.xyz / world.w - camera.eye.xyz, forward);
}

// Weighted average over the previous mip's 2×2 block: depths within the
// falloff range of the block minimum keep full weight, farther ones fade —
// preserving near-occluder depths that a plain average would wash out
// (XeGTAO's depth MIP filter, depth_range_scale_factor 0.75).
fn weighted_average(depth0: f32, depth1: f32, depth2: f32, depth3: f32) -> f32 {
    let effect_radius = 0.75 * EFFECT_RADIUS;
    let falloff_range = FALLOFF_RANGE_FRACTION * effect_radius;
    let falloff_from  = effect_radius * (1.0 - FALLOFF_RANGE_FRACTION);
    let falloff_mul   = -1.0 / falloff_range;
    let falloff_add   = falloff_from / falloff_range + 1.0;

    let min_depth = min(min(depth0, depth1), min(depth2, depth3));
    let weight0 = saturate((depth0 - min_depth) * falloff_mul + falloff_add);
    let weight1 = saturate((depth1 - min_depth) * falloff_mul + falloff_add);
    let weight2 = saturate((depth2 - min_depth) * falloff_mul + falloff_add);
    let weight3 = saturate((depth3 - min_depth) * falloff_mul + falloff_add);
    let weight_total = weight0 + weight1 + weight2 + weight3;

    return ((weight0 * depth0) + (weight1 * depth1) + (weight2 * depth2) + (weight3 * depth3)) / weight_total;
}

var<workgroup> previous_mip_depth: array<array<f32, 8>, 8>;

@compute
@workgroup_size(8, 8, 1)
fn prefilter_depth(@builtin(global_invocation_id) global_id: vec3<u32>, @builtin(local_invocation_id) local_id: vec3<u32>) {
    let base_coordinates = vec2<i32>(global_id.xy);
    let dims = vec2<i32>(textureDimensions(t_src_depth));
    let forward = camera_forward();

    // MIP 0 — linearize 4 source texels per invocation.
    let pixel_coordinates0 = base_coordinates * 2i;
    let pixel_coordinates1 = pixel_coordinates0 + vec2<i32>(1i, 0i);
    let pixel_coordinates2 = pixel_coordinates0 + vec2<i32>(0i, 1i);
    let pixel_coordinates3 = pixel_coordinates0 + vec2<i32>(1i, 1i);
    let depth0 = linear_depth_at(pixel_coordinates0, dims, forward);
    let depth1 = linear_depth_at(pixel_coordinates1, dims, forward);
    let depth2 = linear_depth_at(pixel_coordinates2, dims, forward);
    let depth3 = linear_depth_at(pixel_coordinates3, dims, forward);
    textureStore(t_mip0, pixel_coordinates0, vec4<f32>(depth0, 0.0, 0.0, 0.0));
    textureStore(t_mip0, pixel_coordinates1, vec4<f32>(depth1, 0.0, 0.0, 0.0));
    textureStore(t_mip0, pixel_coordinates2, vec4<f32>(depth2, 0.0, 0.0, 0.0));
    textureStore(t_mip0, pixel_coordinates3, vec4<f32>(depth3, 0.0, 0.0, 0.0));

    // MIP 1 — weighted average of MIP 0 (per invocation).
    let depth_mip1 = weighted_average(depth0, depth1, depth2, depth3);
    textureStore(t_mip1, base_coordinates, vec4<f32>(depth_mip1, 0.0, 0.0, 0.0));
    previous_mip_depth[local_id.x][local_id.y] = depth_mip1;

    workgroupBarrier();

    // MIP 2 — 4×4 invocations per workgroup.
    if all(local_id.xy % vec2<u32>(2u) == vec2<u32>(0u)) {
        let depth0 = previous_mip_depth[local_id.x + 0u][local_id.y + 0u];
        let depth1 = previous_mip_depth[local_id.x + 1u][local_id.y + 0u];
        let depth2 = previous_mip_depth[local_id.x + 0u][local_id.y + 1u];
        let depth3 = previous_mip_depth[local_id.x + 1u][local_id.y + 1u];
        let depth_mip2 = weighted_average(depth0, depth1, depth2, depth3);
        textureStore(t_mip2, base_coordinates / 2i, vec4<f32>(depth_mip2, 0.0, 0.0, 0.0));
        previous_mip_depth[local_id.x][local_id.y] = depth_mip2;
    }

    workgroupBarrier();

    // MIP 3 — 2×2 invocations per workgroup.
    if all(local_id.xy % vec2<u32>(4u) == vec2<u32>(0u)) {
        let depth0 = previous_mip_depth[local_id.x + 0u][local_id.y + 0u];
        let depth1 = previous_mip_depth[local_id.x + 2u][local_id.y + 0u];
        let depth2 = previous_mip_depth[local_id.x + 0u][local_id.y + 2u];
        let depth3 = previous_mip_depth[local_id.x + 2u][local_id.y + 2u];
        let depth_mip3 = weighted_average(depth0, depth1, depth2, depth3);
        textureStore(t_mip3, base_coordinates / 4i, vec4<f32>(depth_mip3, 0.0, 0.0, 0.0));
        previous_mip_depth[local_id.x][local_id.y] = depth_mip3;
    }

    workgroupBarrier();

    // MIP 4 — 1 invocation per workgroup.
    if all(local_id.xy % vec2<u32>(8u) == vec2<u32>(0u)) {
        let depth0 = previous_mip_depth[local_id.x + 0u][local_id.y + 0u];
        let depth1 = previous_mip_depth[local_id.x + 4u][local_id.y + 0u];
        let depth2 = previous_mip_depth[local_id.x + 0u][local_id.y + 4u];
        let depth3 = previous_mip_depth[local_id.x + 4u][local_id.y + 4u];
        let depth_mip4 = weighted_average(depth0, depth1, depth2, depth3);
        textureStore(t_mip4, base_coordinates / 8i, vec4<f32>(depth_mip4, 0.0, 0.0, 0.0));
    }
}

// ── Pass 2: GTAO main (horizon slices + edge packing) ────────────────────────

@group(2) @binding(0) var t_depth_mips: texture_2d<f32>;
@group(2) @binding(1) var t_hilbert: texture_2d<u32>;
@group(2) @binding(2) var s_point: sampler;
@group(2) @binding(3) var t_ao_out: texture_storage_2d<r32float, write>;
@group(2) @binding(4) var t_edges_out: texture_storage_2d<r32uint, write>;

fn fast_sqrt(x: f32) -> f32 {
    return bitcast<f32>(0x1fbd1df5 + (bitcast<i32>(x) >> 1u));
}

fn fast_acos(in_x: f32) -> f32 {
    let x = abs(in_x);
    var res = -0.156583 * x + HALF_PI;
    res *= fast_sqrt(1.0 - x);
    return select(PI - res, res, in_x >= 0.0);
}

fn load_noise(pixel_coordinates: vec2<i32>) -> vec2<f32> {
    let index = textureLoad(t_hilbert, pixel_coordinates % HILBERT_WIDTH, 0).r;
    // R2 sequence — http://extremelearning.com.au/unreasonable-effectiveness-of-quasirandom-sequences
    return fract(0.5 + f32(index) * vec2<f32>(0.75487766624669276005, 0.5698402909980532659114));
}

fn mip0_depth(coord: vec2<i32>, dims: vec2<i32>) -> f32 {
    return textureLoad(t_depth_mips, clamp(coord, vec2<i32>(0), dims - 1), 0).r;
}

fn load_and_reconstruct_world_position(uv: vec2<f32>, sample_mip_level: f32, forward: vec3<f32>) -> vec3<f32> {
    let depth = textureSampleLevel(t_depth_mips, s_point, uv, sample_mip_level).r;
    return world_on_ray(pixel_ray(uv), forward, depth);
}

@compute
@workgroup_size(8, 8, 1)
fn gtao(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let pixel_coordinates = vec2<i32>(global_id.xy);
    let dims = vec2<i32>(textureDimensions(t_depth_mips));
    if any(pixel_coordinates >= dims) {
        return;
    }
    let uv = (vec2<f32>(pixel_coordinates) + 0.5) / vec2<f32>(dims);

    let pixel_depth  = mip0_depth(pixel_coordinates, dims);
    let depth_left   = mip0_depth(pixel_coordinates + vec2<i32>(-1i, 0i), dims);
    let depth_right  = mip0_depth(pixel_coordinates + vec2<i32>(1i, 0i), dims);
    let depth_top    = mip0_depth(pixel_coordinates + vec2<i32>(0i, -1i), dims);
    let depth_bottom = mip0_depth(pixel_coordinates + vec2<i32>(0i, 1i), dims);

    // Depth-difference edges for the denoiser (large differences = object
    // edges), slope-adjusted so a plane at an angle doesn't read as one.
    var edge_info = vec4<f32>(depth_left, depth_right, depth_top, depth_bottom) - pixel_depth;
    let slope_left_right = (edge_info.y - edge_info.x) * 0.5;
    let slope_top_bottom = (edge_info.w - edge_info.z) * 0.5;
    let edge_info_slope_adjusted = edge_info + vec4<f32>(slope_left_right, -slope_left_right, slope_top_bottom, -slope_top_bottom);
    edge_info = min(abs(edge_info), abs(edge_info_slope_adjusted));
    let edges = saturate(1.25 - edge_info / (pixel_depth * 0.011));
    textureStore(t_edges_out, pixel_coordinates, vec4<u32>(pack4x8unorm(edges), 0u, 0u, 0u));

    if (pixel_depth > SKY_THRESHOLD) {
        textureStore(t_ao_out, pixel_coordinates, vec4<f32>(1.0, 0.0, 0.0, 0.0));
        return;
    }

    let forward = camera_forward();
    let center_ray = pixel_ray(uv);
    let pixel_position = world_on_ray(center_ray, forward, pixel_depth);
    let view_vec = -normalize(center_ray.dir);

    // XeGTAO's depth-generated normal: cross products of the edge-accepted
    // neighbor directions. The reference computes these in its left-handed
    // viewspace (x right, y up, z into screen); world space with the
    // camera's (right, up, forward) basis is the mirror of that, so every
    // cross flips sign — folded into the final negation.
    let texel = 1.0 / vec2<f32>(dims);
    let position_left   = world_on_ray(pixel_ray(uv + vec2<f32>(-texel.x, 0.0)), forward, depth_left);
    let position_right  = world_on_ray(pixel_ray(uv + vec2<f32>(texel.x, 0.0)), forward, depth_right);
    let position_top    = world_on_ray(pixel_ray(uv + vec2<f32>(0.0, -texel.y)), forward, depth_top);
    let position_bottom = world_on_ray(pixel_ray(uv + vec2<f32>(0.0, texel.y)), forward, depth_bottom);
    let accepted_normals = saturate(vec4<f32>(edges.x * edges.z, edges.z * edges.y, edges.y * edges.w, edges.w * edges.x) + 0.01);
    let dir_left   = normalize(position_left - pixel_position);
    let dir_right  = normalize(position_right - pixel_position);
    let dir_top    = normalize(position_top - pixel_position);
    let dir_bottom = normalize(position_bottom - pixel_position);
    let pixel_normal = -normalize(
        accepted_normals.x * cross(dir_left, dir_top) +
        accepted_normals.y * cross(dir_top, dir_right) +
        accepted_normals.z * cross(dir_right, dir_bottom) +
        accepted_normals.w * cross(dir_bottom, dir_left),
    );

    let falloff_range = FALLOFF_RANGE_FRACTION * EFFECT_RADIUS;
    let falloff_from  = EFFECT_RADIUS * (1.0 - FALLOFF_RANGE_FRACTION);
    let falloff_mul   = -1.0 / falloff_range;
    let falloff_add   = falloff_from / falloff_range + 1.0;

    let noise = load_noise(pixel_coordinates);
    // Screen-space extent of the world-space effect radius, per axis, so the
    // horizon march covers the same metric reach in every projection mode.
    let radius_uv = vec2<f32>(
        length(world_to_uv(pixel_position + camera.right.xyz * EFFECT_RADIUS) - uv),
        length(world_to_uv(pixel_position + camera.up.xyz * EFFECT_RADIUS) - uv),
    );

    var visibility = 0.0;
    for (var slice_t = 0.0; slice_t < SLICE_COUNT; slice_t += 1.0) {
        let slice = slice_t + noise.x;
        let phi = (PI / SLICE_COUNT) * slice;
        let omega = vec2<f32>(cos(phi), sin(phi));

        let direction = omega.x * camera.right.xyz + omega.y * camera.up.xyz;
        let orthographic_direction = direction - (dot(direction, view_vec) * view_vec);
        let axis = cross(direction, view_vec);
        let projected_normal = pixel_normal - axis * dot(pixel_normal, axis);
        let projected_normal_length = length(projected_normal);

        let sign_norm = sign(dot(orthographic_direction, projected_normal));
        let cos_norm = saturate(dot(projected_normal, view_vec) / projected_normal_length);
        let n = sign_norm * fast_acos(cos_norm);

        let min_cos_horizon_1 = cos(n + HALF_PI);
        let min_cos_horizon_2 = cos(n - HALF_PI);
        var cos_horizon_1 = min_cos_horizon_1;
        var cos_horizon_2 = min_cos_horizon_2;
        let sample_mul = vec2<f32>(omega.x, -omega.y) * radius_uv;
        for (var sample_t = 0.0; sample_t < SAMPLES_PER_SLICE_SIDE; sample_t += 1.0) {
            var sample_noise = (slice_t + sample_t * SAMPLES_PER_SLICE_SIDE) * 0.6180339887498948482;
            sample_noise = fract(noise.y + sample_noise);

            var s = (sample_t + sample_noise) / SAMPLES_PER_SLICE_SIDE;
            s *= s; // https://github.com/GameTechDev/XeGTAO#sample-distribution
            let sample = s * sample_mul;

            // https://github.com/GameTechDev/XeGTAO#memory-bandwidth-bottleneck
            let sample_mip_level = clamp(log2(length(sample * camera.viewport.xy)) - 3.3, 0.0, 4.0);
            let sample_position_1 = load_and_reconstruct_world_position(uv + sample, sample_mip_level, forward);
            let sample_position_2 = load_and_reconstruct_world_position(uv - sample, sample_mip_level, forward);

            let sample_difference_1 = sample_position_1 - pixel_position;
            let sample_difference_2 = sample_position_2 - pixel_position;
            let sample_distance_1 = length(sample_difference_1);
            let sample_distance_2 = length(sample_difference_2);
            var sample_cos_horizon_1 = dot(sample_difference_1 / sample_distance_1, view_vec);
            var sample_cos_horizon_2 = dot(sample_difference_2 / sample_distance_2, view_vec);

            let weight_1 = saturate(sample_distance_1 * falloff_mul + falloff_add);
            let weight_2 = saturate(sample_distance_2 * falloff_mul + falloff_add);
            sample_cos_horizon_1 = mix(min_cos_horizon_1, sample_cos_horizon_1, weight_1);
            sample_cos_horizon_2 = mix(min_cos_horizon_2, sample_cos_horizon_2, weight_2);

            cos_horizon_1 = max(cos_horizon_1, sample_cos_horizon_1);
            cos_horizon_2 = max(cos_horizon_2, sample_cos_horizon_2);
        }

        let horizon_1 = fast_acos(cos_horizon_1);
        let horizon_2 = -fast_acos(cos_horizon_2);
        let v1 = (cos_norm + 2.0 * horizon_1 * sin(n) - cos(2.0 * horizon_1 - n)) / 4.0;
        let v2 = (cos_norm + 2.0 * horizon_2 * sin(n) - cos(2.0 * horizon_2 - n)) / 4.0;
        visibility += projected_normal_length * (v1 + v2);
    }
    visibility /= SLICE_COUNT;
    visibility = clamp(visibility, 0.03, 1.0);

    textureStore(t_ao_out, pixel_coordinates, vec4<f32>(visibility, 0.0, 0.0, 0.0));
}

// ── Pass 3: edge-aware 3×3 spatial denoise ───────────────────────────────────

@group(3) @binding(0) var t_ao_noisy: texture_2d<f32>;
@group(3) @binding(1) var t_edges: texture_2d<u32>;
@group(3) @binding(2) var t_ao_final: texture_storage_2d<r32float, write>;

fn edges_at(coord: vec2<i32>, dims: vec2<i32>) -> vec4<f32> {
    return unpack4x8unorm(textureLoad(t_edges, clamp(coord, vec2<i32>(0), dims - 1), 0).r);
}

fn visibility_at(coord: vec2<i32>, dims: vec2<i32>) -> f32 {
    return textureLoad(t_ao_noisy, clamp(coord, vec2<i32>(0), dims - 1), 0).r;
}

@compute
@workgroup_size(8, 8, 1)
fn denoise(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let pixel_coordinates = vec2<i32>(global_id.xy);
    let dims = vec2<i32>(textureDimensions(t_ao_noisy));
    if any(pixel_coordinates >= dims) {
        return;
    }

    let left_edges   = edges_at(pixel_coordinates + vec2<i32>(-1i, 0i), dims);
    let right_edges  = edges_at(pixel_coordinates + vec2<i32>(1i, 0i), dims);
    let top_edges    = edges_at(pixel_coordinates + vec2<i32>(0i, -1i), dims);
    let bottom_edges = edges_at(pixel_coordinates + vec2<i32>(0i, 1i), dims);
    // Mutual agreement: this pixel's edge to a neighbor counts only if the
    // neighbor's opposing edge agrees, so a one-sided depth discontinuity
    // can't leak AO across the silhouette.
    var center_edges = edges_at(pixel_coordinates, dims);
    center_edges *= vec4<f32>(left_edges.y, right_edges.x, top_edges.w, bottom_edges.z);

    let center_weight = 1.2;
    let left_weight   = center_edges.x;
    let right_weight  = center_edges.y;
    let top_weight    = center_edges.z;
    let bottom_weight = center_edges.w;
    let top_left_weight     = 0.425 * (top_weight * top_edges.x + left_weight * left_edges.z);
    let top_right_weight    = 0.425 * (top_weight * top_edges.y + right_weight * right_edges.z);
    let bottom_left_weight  = 0.425 * (bottom_weight * bottom_edges.x + left_weight * left_edges.w);
    let bottom_right_weight = 0.425 * (bottom_weight * bottom_edges.y + right_weight * right_edges.w);

    var sum = visibility_at(pixel_coordinates, dims);
    sum += visibility_at(pixel_coordinates + vec2<i32>(-1i, 0i), dims) * left_weight;
    sum += visibility_at(pixel_coordinates + vec2<i32>(1i, 0i), dims) * right_weight;
    sum += visibility_at(pixel_coordinates + vec2<i32>(0i, -1i), dims) * top_weight;
    sum += visibility_at(pixel_coordinates + vec2<i32>(0i, 1i), dims) * bottom_weight;
    sum += visibility_at(pixel_coordinates + vec2<i32>(-1i, -1i), dims) * top_left_weight;
    sum += visibility_at(pixel_coordinates + vec2<i32>(1i, -1i), dims) * top_right_weight;
    sum += visibility_at(pixel_coordinates + vec2<i32>(-1i, 1i), dims) * bottom_left_weight;
    sum += visibility_at(pixel_coordinates + vec2<i32>(1i, 1i), dims) * bottom_right_weight;

    var sum_weight = center_weight;
    sum_weight += left_weight;
    sum_weight += right_weight;
    sum_weight += top_weight;
    sum_weight += bottom_weight;
    sum_weight += top_left_weight;
    sum_weight += top_right_weight;
    sum_weight += bottom_left_weight;
    sum_weight += bottom_right_weight;

    textureStore(t_ao_final, pixel_coordinates, vec4<f32>(sum / sum_weight, 0.0, 0.0, 0.0));
}
