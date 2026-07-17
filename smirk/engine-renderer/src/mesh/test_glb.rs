// Hand-rolled GLB builders for CPU-stage tests — no asset files, no GPU.

/// Build a minimal single-triangle GLB by hand: one node (translated by
/// (1,2,3)) with positions/normals/uvs/u16 indices and a solid
/// baseColorFactor material. Exercises the whole CPU stage without
/// depending on asset files or a GPU.
pub(crate) fn write_test_glb(path: &std::path::Path) {
    let mut bin: Vec<u8> = Vec::new();
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals:   [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs:       [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let indices:   [u16; 3]      = [0, 1, 2];
    for v in positions.iter().flatten() { bin.extend_from_slice(&v.to_le_bytes()); }
    for v in normals.iter().flatten()   { bin.extend_from_slice(&v.to_le_bytes()); }
    for v in uvs.iter().flatten()       { bin.extend_from_slice(&v.to_le_bytes()); }
    for i in indices                    { bin.extend_from_slice(&i.to_le_bytes()); }

    let json = format!(r#"{{
        "asset": {{"version": "2.0"}},
        "scene": 0,
        "scenes": [{{"nodes": [0]}}],
        "nodes": [{{"mesh": 0, "translation": [1, 2, 3]}}],
        "meshes": [{{"primitives": [{{
            "attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}},
            "indices": 3, "material": 0
        }}]}}],
        "materials": [{{"pbrMetallicRoughness": {{"baseColorFactor": [0.2, 0.4, 0.8, 1.0]}},
                        "alphaMode": "MASK", "alphaCutoff": 0.35}}],
        "buffers": [{{"byteLength": {bin_len}}}],
        "bufferViews": [
            {{"buffer": 0, "byteOffset": 0,  "byteLength": 36}},
            {{"buffer": 0, "byteOffset": 36, "byteLength": 36}},
            {{"buffer": 0, "byteOffset": 72, "byteLength": 24}},
            {{"buffer": 0, "byteOffset": 96, "byteLength": 6}}
        ],
        "accessors": [
            {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}},
            {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}},
            {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"}},
            {{"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}}
        ]
    }}"#, bin_len = bin.len());

    let mut json_bytes = json.into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }
    while !bin.len().is_multiple_of(4) { bin.push(0); }

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&0x46546C67u32.to_le_bytes()); // magic "glTF"
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // "JSON"
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E4942u32.to_le_bytes()); // "BIN\0"
    glb.extend_from_slice(&bin);
    std::fs::write(path, glb).unwrap();
}

/// Hand-build a skinned + animated GLB: three vertices stacked on +Y, a
/// two-joint chain (root at origin, child at +1 Y), and a clip that rotates
/// the root 90° about Z over one second. Proves the whole skinning CPU
/// stage — skin hierarchy, inverse binds, animation channels, and the
/// bake-branch (skinned vertices stay in mesh-local space) — without a GPU.
pub(crate) fn write_skinned_glb(path: &std::path::Path) {
    // Pad to 4 bytes, append, return (offset, len).
    fn push(bin: &mut Vec<u8>, data: &[u8]) -> (usize, usize) {
        while !bin.len().is_multiple_of(4) { bin.push(0); }
        let off = bin.len();
        bin.extend_from_slice(data);
        (off, data.len())
    }
    fn f32s(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }
    fn u16s(v: &[u16]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

    let mut bin = Vec::new();
    let (pos_off, pos_len) = push(&mut bin, &f32s(&[0.0, 0.0, 0.0,  0.0, 1.0, 0.0,  0.0, 2.0, 0.0]));
    let (joi_off, joi_len) = push(&mut bin, &u16s(&[0, 0, 0, 0,  1, 0, 0, 0,  1, 0, 0, 0]));
    let (wei_off, wei_len) = push(&mut bin, &f32s(&[1.0, 0.0, 0.0, 0.0,  1.0, 0.0, 0.0, 0.0,  1.0, 0.0, 0.0, 0.0]));
    let (idx_off, idx_len) = push(&mut bin, &u16s(&[0, 1, 2]));
    // Inverse binds (column-major): joint0 = I, joint1 = translate(0,-1,0).
    let ibm = f32s(&[
        1.0, 0.0, 0.0, 0.0,  0.0, 1.0, 0.0, 0.0,  0.0, 0.0, 1.0, 0.0,  0.0,  0.0, 0.0, 1.0,
        1.0, 0.0, 0.0, 0.0,  0.0, 1.0, 0.0, 0.0,  0.0, 0.0, 1.0, 0.0,  0.0, -1.0, 0.0, 1.0,
    ]);
    let (ibm_off, ibm_len) = push(&mut bin, &ibm);
    let (ti_off, ti_len) = push(&mut bin, &f32s(&[0.0, 1.0]));
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let (ro_off, ro_len) = push(&mut bin, &f32s(&[0.0, 0.0, 0.0, 1.0,  0.0, 0.0, s, s]));

    let json = format!(r#"{{
        "asset": {{"version": "2.0"}},
        "scene": 0,
        "scenes": [{{"nodes": [0, 1]}}],
        "nodes": [
            {{"mesh": 0, "skin": 0}},
            {{"translation": [0, 0, 0], "children": [2]}},
            {{"translation": [0, 1, 0]}}
        ],
        "skins": [{{"joints": [1, 2], "inverseBindMatrices": 4}}],
        "meshes": [{{"primitives": [{{
            "attributes": {{"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}},
            "indices": 3
        }}]}}],
        "animations": [{{
            "name": "Spin",
            "channels": [{{"sampler": 0, "target": {{"node": 1, "path": "rotation"}}}}],
            "samplers": [{{"input": 5, "output": 6, "interpolation": "LINEAR"}}]
        }}],
        "buffers": [{{"byteLength": {bin_len}}}],
        "bufferViews": [
            {{"buffer": 0, "byteOffset": {pos_off}, "byteLength": {pos_len}}},
            {{"buffer": 0, "byteOffset": {joi_off}, "byteLength": {joi_len}}},
            {{"buffer": 0, "byteOffset": {wei_off}, "byteLength": {wei_len}}},
            {{"buffer": 0, "byteOffset": {idx_off}, "byteLength": {idx_len}}},
            {{"buffer": 0, "byteOffset": {ibm_off}, "byteLength": {ibm_len}}},
            {{"buffer": 0, "byteOffset": {ti_off}, "byteLength": {ti_len}}},
            {{"buffer": 0, "byteOffset": {ro_off}, "byteLength": {ro_len}}}
        ],
        "accessors": [
            {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0, 0.0, 0.0], "max": [0.0, 2.0, 0.0]}},
            {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "VEC4"}},
            {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"}},
            {{"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}},
            {{"bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4"}},
            {{"bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR",
              "min": [0.0], "max": [1.0]}},
            {{"bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC4"}}
        ]
    }}"#, bin_len = bin.len());

    let mut json_bytes = json.into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }
    while bin.len() % 4 != 0 { bin.push(0); }

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&0x46546C67u32.to_le_bytes());
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E4942u32.to_le_bytes());
    glb.extend_from_slice(&bin);
    std::fs::write(path, glb).unwrap();
}

/// Build a minimal single-triangle GLB whose material carries an embedded
/// PNG base-color texture (a solid-red 2×2 image) — the seam for testing the
/// sidecar-DDS-preference path, which needs a real base-color image slot to
/// prefer over.
pub(crate) fn write_textured_glb(path: &std::path::Path) {
    fn push(bin: &mut Vec<u8>, data: &[u8]) -> (usize, usize) {
        while !bin.len().is_multiple_of(4) { bin.push(0); }
        let off = bin.len();
        bin.extend_from_slice(data);
        (off, data.len())
    }
    fn f32s(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }
    fn u16s(v: &[u16]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

    let mut bin = Vec::new();
    let (pos_off, pos_len) = push(&mut bin, &f32s(&[0.0, 0.0, 0.0,  1.0, 0.0, 0.0,  0.0, 1.0, 0.0]));
    let (nrm_off, nrm_len) = push(&mut bin, &f32s(&[0.0, 0.0, 1.0,  0.0, 0.0, 1.0,  0.0, 0.0, 1.0]));
    let (uv_off,  uv_len)  = push(&mut bin, &f32s(&[0.0, 0.0,  1.0, 0.0,  0.0, 1.0]));
    let (idx_off, idx_len) = push(&mut bin, &u16s(&[0, 1, 2]));

    let mut png_bytes = Vec::new();
    let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .unwrap();
    let (img_off, img_len) = push(&mut bin, &png_bytes);

    let json = format!(r#"{{
        "asset": {{"version": "2.0"}},
        "scene": 0,
        "scenes": [{{"nodes": [0]}}],
        "nodes": [{{"mesh": 0}}],
        "meshes": [{{"primitives": [{{
            "attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}},
            "indices": 3, "material": 0
        }}]}}],
        "materials": [{{"pbrMetallicRoughness": {{"baseColorTexture": {{"index": 0}}}}}}],
        "textures": [{{"source": 0}}],
        "images": [{{"mimeType": "image/png", "bufferView": 4}}],
        "buffers": [{{"byteLength": {bin_len}}}],
        "bufferViews": [
            {{"buffer": 0, "byteOffset": {pos_off}, "byteLength": {pos_len}}},
            {{"buffer": 0, "byteOffset": {nrm_off}, "byteLength": {nrm_len}}},
            {{"buffer": 0, "byteOffset": {uv_off},  "byteLength": {uv_len}}},
            {{"buffer": 0, "byteOffset": {idx_off}, "byteLength": {idx_len}}},
            {{"buffer": 0, "byteOffset": {img_off}, "byteLength": {img_len}}}
        ],
        "accessors": [
            {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}},
            {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}},
            {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"}},
            {{"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}}
        ]
    }}"#, bin_len = bin.len());

    let mut json_bytes = json.into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }
    while bin.len() % 4 != 0 { bin.push(0); }

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&0x46546C67u32.to_le_bytes()); // magic "glTF"
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // "JSON"
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E4942u32.to_le_bytes()); // "BIN\0"
    glb.extend_from_slice(&bin);
    std::fs::write(path, glb).unwrap();
}

/// Same layout as `write_textured_glb`, but the embedded base-color image is
/// garbage bytes instead of a valid PNG — the seam for testing that a
/// sidecar DDS skips decoding it entirely, and that a corrupt embedded image
/// with no sidecar is a per-slot `None` rather than a whole-asset error.
pub(crate) fn write_corrupt_textured_glb(path: &std::path::Path) {
    fn push(bin: &mut Vec<u8>, data: &[u8]) -> (usize, usize) {
        while !bin.len().is_multiple_of(4) { bin.push(0); }
        let off = bin.len();
        bin.extend_from_slice(data);
        (off, data.len())
    }
    fn f32s(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }
    fn u16s(v: &[u16]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

    let mut bin = Vec::new();
    let (pos_off, pos_len) = push(&mut bin, &f32s(&[0.0, 0.0, 0.0,  1.0, 0.0, 0.0,  0.0, 1.0, 0.0]));
    let (nrm_off, nrm_len) = push(&mut bin, &f32s(&[0.0, 0.0, 1.0,  0.0, 0.0, 1.0,  0.0, 0.0, 1.0]));
    let (uv_off,  uv_len)  = push(&mut bin, &f32s(&[0.0, 0.0,  1.0, 0.0,  0.0, 1.0]));
    let (idx_off, idx_len) = push(&mut bin, &u16s(&[0, 1, 2]));

    let png_bytes = b"not a real png".to_vec();
    let (img_off, img_len) = push(&mut bin, &png_bytes);

    let json = format!(r#"{{
        "asset": {{"version": "2.0"}},
        "scene": 0,
        "scenes": [{{"nodes": [0]}}],
        "nodes": [{{"mesh": 0}}],
        "meshes": [{{"primitives": [{{
            "attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}},
            "indices": 3, "material": 0
        }}]}}],
        "materials": [{{"pbrMetallicRoughness": {{"baseColorTexture": {{"index": 0}}}}}}],
        "textures": [{{"source": 0}}],
        "images": [{{"mimeType": "image/png", "bufferView": 4}}],
        "buffers": [{{"byteLength": {bin_len}}}],
        "bufferViews": [
            {{"buffer": 0, "byteOffset": {pos_off}, "byteLength": {pos_len}}},
            {{"buffer": 0, "byteOffset": {nrm_off}, "byteLength": {nrm_len}}},
            {{"buffer": 0, "byteOffset": {uv_off},  "byteLength": {uv_len}}},
            {{"buffer": 0, "byteOffset": {idx_off}, "byteLength": {idx_len}}},
            {{"buffer": 0, "byteOffset": {img_off}, "byteLength": {img_len}}}
        ],
        "accessors": [
            {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}},
            {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}},
            {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"}},
            {{"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}}
        ]
    }}"#, bin_len = bin.len());

    let mut json_bytes = json.into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }
    while bin.len() % 4 != 0 { bin.push(0); }

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&0x46546C67u32.to_le_bytes()); // magic "glTF"
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // "JSON"
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E4942u32.to_le_bytes()); // "BIN\0"
    glb.extend_from_slice(&bin);
    std::fs::write(path, glb).unwrap();
}

/// Build a minimal single-triangle GLB with BLEND alpha mode and red semi-transparent baseColorFactor.
pub(crate) fn write_blend_glb(path: &std::path::Path) {
    let mut bin: Vec<u8> = Vec::new();
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals:   [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs:       [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let indices:   [u16; 3]      = [0, 1, 2];
    for v in positions.iter().flatten() { bin.extend_from_slice(&v.to_le_bytes()); }
    for v in normals.iter().flatten()   { bin.extend_from_slice(&v.to_le_bytes()); }
    for v in uvs.iter().flatten()       { bin.extend_from_slice(&v.to_le_bytes()); }
    for i in indices                    { bin.extend_from_slice(&i.to_le_bytes()); }

    let json = format!(r#"{{
        "asset": {{"version": "2.0"}},
        "scene": 0,
        "scenes": [{{"nodes": [0]}}],
        "nodes": [{{"mesh": 0, "translation": [1, 2, 3]}}],
        "meshes": [{{"primitives": [{{
            "attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}},
            "indices": 3, "material": 0
        }}]}}],
        "materials": [{{"pbrMetallicRoughness": {{"baseColorFactor": [1.0, 0.0, 0.0, 0.6]}},
                        "alphaMode": "BLEND"}}],
        "buffers": [{{"byteLength": {bin_len}}}],
        "bufferViews": [
            {{"buffer": 0, "byteOffset": 0,  "byteLength": 36}},
            {{"buffer": 0, "byteOffset": 36, "byteLength": 36}},
            {{"buffer": 0, "byteOffset": 72, "byteLength": 24}},
            {{"buffer": 0, "byteOffset": 96, "byteLength": 6}}
        ],
        "accessors": [
            {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}},
            {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}},
            {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"}},
            {{"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}}
        ]
    }}"#, bin_len = bin.len());

    let mut json_bytes = json.into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }
    while !bin.len().is_multiple_of(4) { bin.push(0); }

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&0x46546C67u32.to_le_bytes()); // magic "glTF"
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // "JSON"
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E4942u32.to_le_bytes()); // "BIN\0"
    glb.extend_from_slice(&bin);
    std::fs::write(path, glb).unwrap();
}
