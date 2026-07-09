// Material repair for glbs that round-tripped through FBX (Mixamo/Blender):
// FBX carries no alphaMode or metallic-roughness, so Blender exports opaque
// default-PBR materials. This forces the values the VRoid body needs:
//   - alphaMode MASK (cutoff 0.5) on any material whose base-color PNG can
//     carry alpha (color type 4/6 or a tRNS chunk) — face/eye overlay planes
//   - metallic 0 / roughness 0.8 everywhere (VRoid textures are unlit toon)
//
// Usage: node fix_glb_materials.mjs <in.glb> [out.glb]   (in-place if no out)

import { readFileSync, writeFileSync } from "node:fs";

const [src, dstArg] = process.argv.slice(2);
const dst = dstArg ?? src;
const buf = readFileSync(src);
if (buf.readUInt32LE(0) !== 0x46546c67) throw new Error(`${src}: not a glb`);
const jsonLen = buf.readUInt32LE(12);
const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString("utf8"));
const rest = buf.subarray(20 + jsonLen);
const binStart = 8; // within `rest`: BIN chunk header, then data

function pngHasAlpha(data) {
  if (data.readUInt32BE(0) !== 0x89504e47) return false; // not PNG (jpg etc.)
  const colorType = data[25]; // IHDR: 8 sig + 8 chunk hdr + w4 h4 depth1
  if (colorType === 4 || colorType === 6) return true;
  // Walk chunks for tRNS (palette transparency).
  let off = 8;
  while (off + 8 <= data.length) {
    const len = data.readUInt32BE(off);
    const type = data.toString("ascii", off + 4, off + 8);
    if (type === "tRNS") return true;
    if (type === "IDAT") return false;
    off += 12 + len;
  }
  return false;
}

for (const mat of json.materials ?? []) {
  mat.pbrMetallicRoughness ??= {};
  mat.pbrMetallicRoughness.metallicFactor = 0.0;
  mat.pbrMetallicRoughness.roughnessFactor = 0.8;

  const texIdx = mat.pbrMetallicRoughness.baseColorTexture?.index;
  if (texIdx === undefined) continue;
  const img = json.images[json.textures[texIdx].source];
  if (img.bufferView === undefined) continue;
  const bv = json.bufferViews[img.bufferView];
  const data = rest.subarray(binStart + (bv.byteOffset ?? 0),
                             binStart + (bv.byteOffset ?? 0) + bv.byteLength);
  if (pngHasAlpha(data)) {
    mat.alphaMode = "MASK";
    mat.alphaCutoff = 0.5;
  }
  console.log(`${mat.name ?? "material"}: alphaMode ${mat.alphaMode ?? "OPAQUE"}`);
}

let jsonOut = Buffer.from(JSON.stringify(json), "utf8");
if (jsonOut.length % 4 !== 0)
  jsonOut = Buffer.concat([jsonOut, Buffer.alloc(4 - (jsonOut.length % 4), 0x20)]);
const header = Buffer.alloc(20);
header.writeUInt32LE(0x46546c67, 0);
header.writeUInt32LE(2, 4);
header.writeUInt32LE(20 + jsonOut.length + rest.length, 8);
header.writeUInt32LE(jsonOut.length, 12);
header.writeUInt32LE(0x4e4f534a, 16);
writeFileSync(dst, Buffer.concat([header, jsonOut, rest]));
console.log(`${dst}: ${20 + jsonOut.length + rest.length} bytes`);
