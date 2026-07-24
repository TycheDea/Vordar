// Preprocesses the three geometry shaders (shader.wgsl, mesh_shader.wgsl,
// skinned_mesh_shader.wgsl), the sky shader (sky.wgsl), the depth
// prepass/SSAO shaders (depth_prepass.wgsl, ssao.wgsl), and the tonemap
// shader (tonemap.wgsl):
// `//#include "snippets/x.wgsl"` lines splice in the shared PBR/shadow/uniform
// snippets, `//#const NAME` markers become `const NAME: f32 = ...;`
// declarations whose values are read straight out of shadow.rs / ibl.rs so
// the WGSL text can never drift from the Rust side. Resolved output lands in
// OUT_DIR; src/lib.rs's shader modules `include_str!` it instead of
// `wgpu::include_wgsl!`.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

const PREPROCESSED_SHADERS: [&str; 7] = [
    "shader.wgsl", "mesh_shader.wgsl", "skinned_mesh_shader.wgsl", "sky.wgsl",
    "depth_prepass.wgsl", "ssao.wgsl", "tonemap.wgsl",
];
const SNIPPETS: [&str; 6] = ["scene_uniforms.wgsl", "shadow_sample.wgsl", "pbr_common.wgsl", "fog.wgsl", "debug_channel.wgsl", "srgb_oetf.wgsl"];

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let src_dir = Path::new("src");

    let shadow_size = extract_u32_const(&src_dir.join("shadow.rs"), "SHADOW_SIZE");
    let cascade_count = extract_u32_const(&src_dir.join("shadow.rs"), "CASCADE_COUNT");
    let prefilter_mips = extract_u32_const(&src_dir.join("ibl.rs"), "PREFILTER_MIPS");
    let max_point_lights = extract_u32_const(&src_dir.join("camera.rs"), "MAX_POINT_LIGHTS");

    let mut consts: HashMap<&str, f64> = HashMap::new();
    consts.insert("SHADOW_TEXEL", 1.0 / shadow_size as f64);
    consts.insert("PREFILTER_MAX_MIP", (prefilter_mips - 1) as f64);

    let mut u32_consts: HashMap<&str, u32> = HashMap::new();
    u32_consts.insert("MAX_POINT_LIGHTS", max_point_lights);
    u32_consts.insert("CASCADE_COUNT", cascade_count);

    for name in PREPROCESSED_SHADERS {
        let path = src_dir.join(name);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let resolved = resolve(&text, src_dir, &consts, &u32_consts);
        fs::write(Path::new(&out_dir).join(name), resolved).unwrap_or_else(|e| panic!("write {name}: {e}"));
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for snippet in SNIPPETS {
        println!("cargo:rerun-if-changed={}", src_dir.join("snippets").join(snippet).display());
    }
    println!("cargo:rerun-if-changed=src/shadow.rs");
    println!("cargo:rerun-if-changed=src/ibl.rs");
    println!("cargo:rerun-if-changed=src/camera.rs");
    println!("cargo:rerun-if-changed=build.rs");
}

/// Reads `pub(crate) const NAME: u32 = <value>;` out of a Rust source file.
fn extract_u32_const(path: &Path, name: &str) -> u32 {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let marker = format!("const {name}: u32 = ");
    let start = text
        .find(&marker)
        .unwrap_or_else(|| panic!("`{marker}` not found in {}", path.display()));
    let rest = &text[start + marker.len()..];
    let end = rest
        .find(';')
        .unwrap_or_else(|| panic!("no terminating `;` for {name} in {}", path.display()));
    rest[..end]
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("parse {name} in {}: {e}", path.display()))
}

/// Resolves `//#include "snippets/x.wgsl"` and `//#const NAME` lines against
/// `src_dir`, `consts` (f32 emission), and `u32_consts` (u32 emission,
/// checked first).
fn resolve(text: &str, src_dir: &Path, consts: &HashMap<&str, f64>, u32_consts: &HashMap<&str, u32>) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rel) = trimmed
            .strip_prefix("//#include \"")
            .and_then(|s| s.strip_suffix('"'))
        {
            let snippet_path = src_dir.join(rel);
            let snippet = fs::read_to_string(&snippet_path)
                .unwrap_or_else(|e| panic!("read {}: {e}", snippet_path.display()));
            // Recurse so a snippet's own `//#const` markers resolve too.
            let resolved_snippet = resolve(&snippet, src_dir, consts, u32_consts);
            out.push_str(resolved_snippet.trim_end());
            out.push('\n');
        } else if let Some(name) = trimmed.strip_prefix("//#const ") {
            let name = name.trim();
            if let Some(value) = u32_consts.get(name) {
                out.push_str(&format!("const {name}: u32 = {value}u;\n"));
            } else {
                let value = consts
                    .get(name)
                    .unwrap_or_else(|| panic!("no injected value for const `{name}`"));
                out.push_str(&format!("const {name}: f32 = {value:?};\n"));
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
