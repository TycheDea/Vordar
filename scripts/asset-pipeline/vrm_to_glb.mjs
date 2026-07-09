// VRoid VRM -> engine-ready static glb (character look-tests).
//
// A .vrm is a standard glTF 2.0 binary with an optional "VRM" extension, so
// no Blender round-trip is needed. This strips what the engine can't use and
// fixes what would render wrong:
//   - drops the VRM extension block (metadata, MToon params, spring bones)
//   - drops skins + JOINTS_0/WEIGHTS_0 so the model loads as a static mesh
//     at bind pose (the rigged path comes later with the clip pipeline)
//   - drops KHR_materials_unlit and forces metallic 0 / roughness 0.8 —
//     VRoid materials are unlit toon; glTF's default metallicFactor 1.0
//     would shade the skin as a dark mirror under IBL
//
// Alpha modes (MASK/BLEND on face/eye/hair layers) are kept: the mesh
// shaders apply an alpha-cutoff discard from the material.
//
// Usage: node vrm_to_glb.mjs <in.vrm> <out.glb>

import { readFileSync, writeFileSync } from "node:fs";

const [src, dst] = process.argv.slice(2);
if (!src || !dst) throw new Error("usage: node vrm_to_glb.mjs <in.vrm> <out.glb>");

const buf = readFileSync(src);
if (buf.readUInt32LE(0) !== 0x46546c67) throw new Error(`${src}: not a glb container`);
const jsonLen = buf.readUInt32LE(12);
const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString("utf8"));
const rest = buf.subarray(20 + jsonLen); // BIN chunk (header + data), byte-identical passthrough

delete json.extensions?.VRM;
json.extensionsUsed = (json.extensionsUsed ?? [])
    .filter(e => e !== "VRM" && e !== "KHR_materials_unlit");
if (json.extensionsUsed.length === 0) delete json.extensionsUsed;

delete json.skins;
for (const node of json.nodes ?? []) delete node.skin;
for (const mesh of json.meshes ?? [])
    for (const prim of mesh.primitives ?? []) {
        delete prim.attributes.JOINTS_0;
        delete prim.attributes.WEIGHTS_0;
    }

for (const mat of json.materials ?? []) {
    delete mat.extensions?.KHR_materials_unlit;
    if (mat.extensions && Object.keys(mat.extensions).length === 0) delete mat.extensions;
    mat.pbrMetallicRoughness ??= {};
    mat.pbrMetallicRoughness.metallicFactor = 0.0;
    mat.pbrMetallicRoughness.roughnessFactor = 0.8;
}

// Reassemble: 12-byte header + JSON chunk (padded to 4 with spaces) + BIN as-is.
let jsonOut = Buffer.from(JSON.stringify(json), "utf8");
if (jsonOut.length % 4 !== 0)
    jsonOut = Buffer.concat([jsonOut, Buffer.alloc(4 - (jsonOut.length % 4), 0x20)]);
const header = Buffer.alloc(20);
header.writeUInt32LE(0x46546c67, 0); // magic "glTF"
header.writeUInt32LE(2, 4);          // version
header.writeUInt32LE(20 + jsonOut.length + rest.length, 8);
header.writeUInt32LE(jsonOut.length, 12);
header.writeUInt32LE(0x4e4f534a, 16); // chunk type "JSON"
writeFileSync(dst, Buffer.concat([header, jsonOut, rest]));
console.log(`${dst}: ${20 + jsonOut.length + rest.length} bytes`);
