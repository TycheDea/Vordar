// Sets a boolean glTF material extra on every material in the given glb(s),
// in place. Used to opt props into shader features gated by material extras
// (e.g. "vordar_detail", read by engine-renderer/src/mesh/gltf_import.rs).
//
// Usage: node set_material_extras.mjs <key> <asset.glb> [asset.glb ...]

import { readFileSync, writeFileSync } from "node:fs";

const [key, ...glbs] = process.argv.slice(2);
if (!key || glbs.length === 0)
  throw new Error("usage: set_material_extras.mjs <key> <asset.glb> ...");

for (const path of glbs) {
  const buf = readFileSync(path);
  if (buf.readUInt32LE(0) !== 0x46546c67) throw new Error(`${path}: not a glb`);
  const jsonLen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString("utf8"));
  const rest = buf.subarray(20 + jsonLen);

  for (const mat of json.materials ?? []) {
    mat.extras ??= {};
    mat.extras[key] = true;
    console.log(`${path}: ${mat.name ?? "material"}.extras.${key} = true`);
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
  writeFileSync(path, Buffer.concat([header, jsonOut, rest]));
}
