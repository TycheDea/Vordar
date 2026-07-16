/// Exponential distance fog with a height falloff; density 0 disables.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let dist = length(camera.eye.xyz - world_pos);
    let h = max(world_pos.y - light.fog_height, 0.0);
    let height_atten = exp(-light.fog_height_falloff * h);
    let t = 1.0 - exp(-light.fog_density * dist * height_atten);
    return mix(color, light.fog_color, t);
}
