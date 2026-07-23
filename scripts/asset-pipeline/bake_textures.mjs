// Transcodes shipped material maps (glTF-embedded PNGs, prop JPEGs, ground-set
// JPEGs) into self-describing DX10 DDS sidecars via texconv, and regenerates
// the engine's tiny BC test fixtures. Sidecar convention:
//   - glTF asset <dir>/foo.glb or foo.gltf -> <dir>/foo.textures/
//       img<N>.dds (one per material-referenced image) + manifest.json
//       { source, sha256, images: [{ index, slot, file }] }
//   - ground set <dir> (diff/nor_gl/rough jpgs) -> DDS written alongside the
//     sources in <dir>, plus manifest.json { source, images: [{ slot, file,
//     source, sha256 }] }; the metallic-roughness map is composed from the
//     rough map via texconv swizzle "rrr1" — roughness replicated into every
//     colour channel. texconv only parses uniform masks reliably (any mask
//     mixing selectors and 0/1 constants, e.g. the old "0r01", silently
//     produced ZERO channels — mirror-flat ground), so the shader's G read
//     gets roughness and metallic comes from the material's metallic_factor
//     (0.0 for ground sets), never from the texture's B channel.
//
// Usage:
//   node bake_textures.mjs gltf <asset.glb|asset.gltf> ...
//   node bake_textures.mjs ground <set-dir> ...
//   node bake_textures.mjs fixtures
//
// texconv.exe is located via the TEXCONV env var, else smirk/texconv.exe;
// get it from https://github.com/microsoft/DirectXTex/releases if missing.

import { readFileSync, writeFileSync, mkdirSync, mkdtempSync, readdirSync, rmSync, renameSync } from "node:fs";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";

const TEXCONV = process.env.TEXCONV ?? "smirk/texconv.exe";

function checkTexconv() {
  const probe = spawnSync(TEXCONV, ["--help"], { stdio: "ignore" });
  if (probe.error) {
    console.error(`texconv not found at "${TEXCONV}" (set TEXCONV env var or place it at smirk/texconv.exe).`);
    console.error("Download it from https://github.com/microsoft/DirectXTex/releases");
    process.exit(1);
  }
}

function runTexconv(args) {
  const result = spawnSync(TEXCONV, args, { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`texconv ${args.join(" ")} failed (exit ${result.status}):\n${result.stdout ?? ""}\n${result.stderr ?? ""}`);
  }
  return result.stdout;
}

function sha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

// Blender's PNG writer stamps an sRGB+gAMA+cHRM chunk on every image it
// saves, including Non-Color (linear) data -- the chunk lies about mr/ao/
// normal source PNGs. texconv's WIC reader honors that chunk regardless of
// the requested output format, so converting to a non-_SRGB DXGI format
// (BC7_UNORM, BC5_UNORM) silently applies a real sRGB->linear decode to
// already-linear bytes unless --ignore-srgb tells it to trust the requested
// format instead of the file's metadata.
const SLOT_FLAGS = {
  base: ["-f", "BC7_UNORM_SRGB", "-srgb", "-m", "0", "-dx10", "-y"],
  emissive: ["-f", "BC7_UNORM_SRGB", "-srgb", "-m", "0", "-dx10", "-y"],
  mr: ["-f", "BC7_UNORM", "-m", "0", "-dx10", "-y", "--ignore-srgb"],
  ao: ["-f", "BC7_UNORM", "-m", "0", "-dx10", "-y", "--ignore-srgb"],
  normal: ["-f", "BC5_UNORM", "-m", "0", "-dx10", "-y", "--ignore-srgb"],
};
const SLOT_CLASS = { base: "srgb", emissive: "srgb", mr: "linear", ao: "linear", normal: "normal" };
const SLOT_PRIORITY = ["base", "mr", "normal", "emissive", "ao"];

function readGlb(file) {
  const buf = readFileSync(file);
  if (buf.readUInt32LE(0) !== 0x46546c67) throw new Error(`${file}: not a glb`);
  const jsonLen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString("utf8"));
  const rest = buf.subarray(20 + jsonLen); // BIN chunk: 8-byte header, then data
  const bin = rest.subarray(8);
  return {
    json,
    getImageBytes: (img) => {
      const bv = json.bufferViews[img.bufferView];
      const start = bv.byteOffset ?? 0;
      return bin.subarray(start, start + bv.byteLength);
    },
  };
}

function readGltf(file) {
  const json = JSON.parse(readFileSync(file, "utf8"));
  const dir = path.dirname(file);
  return {
    json,
    getImageBytes: (img) => {
      if (img.bufferView !== undefined) throw new Error(`${file}: embedded bufferView images not supported in .gltf mode`);
      return readFileSync(path.join(dir, decodeURIComponent(img.uri)));
    },
  };
}

function extFor(img, bytes) {
  if (img.mimeType === "image/png") return ".png";
  if (img.mimeType === "image/jpeg") return ".jpg";
  return bytes.readUInt32BE(0) === 0x89504e47 ? ".png" : ".jpg";
}

function pngDims(buf) {
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

function jpegDims(buf) {
  let offset = 2;
  while (offset < buf.length) {
    if (buf[offset] !== 0xff) { offset++; continue; }
    const marker = buf[offset + 1];
    if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) { offset += 2; continue; }
    if (marker === 0xd9) break;
    const len = buf.readUInt16BE(offset + 2);
    if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
      return { height: buf.readUInt16BE(offset + 5), width: buf.readUInt16BE(offset + 7) };
    }
    offset += 2 + len;
  }
  throw new Error("no JPEG SOF marker found");
}

function dimsFor(ext, bytes) {
  return ext === ".png" ? pngDims(bytes) : jpegDims(bytes);
}

// Maps each material-referenced image index to the one pbr slot it bakes as.
// Images pulled into slots from two different color-space classes (e.g. base
// and normal) are ambiguous — warn and drop rather than guess.
function classifySlots(json) {
  const slotsByImage = new Map();
  const add = (texRef, slot) => {
    if (texRef?.index === undefined) return;
    const imgIdx = json.textures[texRef.index].source;
    if (!slotsByImage.has(imgIdx)) slotsByImage.set(imgIdx, new Set());
    slotsByImage.get(imgIdx).add(slot);
  };
  for (const mat of json.materials ?? []) {
    add(mat.pbrMetallicRoughness?.baseColorTexture, "base");
    add(mat.pbrMetallicRoughness?.metallicRoughnessTexture, "mr");
    add(mat.normalTexture, "normal");
    add(mat.emissiveTexture, "emissive");
    add(mat.occlusionTexture, "ao");
  }
  const primarySlot = new Map();
  for (const [imgIdx, slots] of slotsByImage) {
    const classes = new Set([...slots].map((s) => SLOT_CLASS[s]));
    if (classes.size > 1) {
      console.warn(`  image ${imgIdx}: conflicting slots ${[...slots].join(",")} - skipping`);
      continue;
    }
    primarySlot.set(imgIdx, SLOT_PRIORITY.find((s) => slots.has(s)));
  }
  return primarySlot;
}

function bakeGltfAsset(assetPath) {
  console.log(`gltf: ${assetPath}`);
  const isGlb = assetPath.toLowerCase().endsWith(".glb");
  const { json, getImageBytes } = isGlb ? readGlb(assetPath) : readGltf(assetPath);
  const primarySlot = classifySlots(json);

  const assetDir = path.dirname(assetPath);
  const stem = path.basename(assetPath, path.extname(assetPath));
  const outDir = path.join(assetDir, `${stem}.textures`);
  mkdirSync(outDir, { recursive: true });

  const tmp = mkdtempSync(path.join(tmpdir(), "bake_textures-"));
  const images = [];
  try {
    for (const [imgIdx, slot] of [...primarySlot].sort((a, b) => a[0] - b[0])) {
      const img = json.images[imgIdx];
      const bytes = getImageBytes(img);
      const ext = extFor(img, bytes);
      const { width, height } = dimsFor(ext, bytes);
      if (width % 4 !== 0 || height % 4 !== 0) {
        console.warn(`  image ${imgIdx}: ${width}x${height} not a multiple of 4 - skipping`);
        continue;
      }
      const tmpFile = path.join(tmp, `img${imgIdx}${ext}`);
      writeFileSync(tmpFile, bytes);
      runTexconv([...SLOT_FLAGS[slot], "-o", outDir, tmpFile]);
      images.push({ index: imgIdx, slot, file: `img${imgIdx}.dds` });
    }
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }

  const manifest = { source: path.basename(assetPath), sha256: sha256(readFileSync(assetPath)), images };
  writeFileSync(path.join(outDir, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
  console.log(`  -> ${outDir} (${images.length} image(s))`);
}

function bakeGroundSet(setDir) {
  console.log(`ground: ${setDir}`);
  const files = readdirSync(setDir).filter((f) => !f.endsWith(".dds"));
  const find = (tag) => {
    const f = files.find((f) => f.includes(tag));
    if (!f) throw new Error(`${setDir}: no file matching "${tag}"`);
    return f;
  };
  const diffFile = find("diff");
  const norFile = find("nor_gl");
  const roughFile = find("rough");

  const bakeOne = (file, flags, outFile, slot) => {
    const full = path.join(setDir, file);
    runTexconv([...flags, "-o", setDir, full]);
    return { slot, file: outFile, source: file, sha256: sha256(readFileSync(full)) };
  };

  const images = [
    bakeOne(diffFile, SLOT_FLAGS.base, `${path.basename(diffFile, path.extname(diffFile))}.dds`, "diff"),
    bakeOne(norFile, SLOT_FLAGS.normal, `${path.basename(norFile, path.extname(norFile))}.dds`, "normal"),
    bakeOne(
      roughFile,
      ["-f", "BC7_UNORM", "-m", "0", "-dx10", "-y", "-swizzle", "rrr1", "-sx", "_mr"],
      `${path.basename(roughFile, path.extname(roughFile))}_mr.dds`,
      "mr",
    ),
  ];

  // The old "0r01" swizzle silently baked all-zero channels (texconv parses
  // only uniform masks reliably), shipping mirror-flat ground. Decode the MR
  // bake back and refuse a degenerate roughness channel.
  const mrDds = path.join(setDir, images[2].file);
  const probeDir = mkdtempSync(path.join(tmpdir(), "mrprobe-"));
  try {
    runTexconv(["-ft", "bmp", "-y", "-o", probeDir, mrDds]);
    const bmp = readFileSync(path.join(probeDir, `${path.basename(mrDds, ".dds")}.bmp`));
    const off = bmp.readUInt32LE(10);
    let sum = 0;
    const n = Math.min(4096, (bmp.length - off) / 3 | 0);
    for (let i = 0; i < n; i++) sum += bmp[off + i * 3 + 1]; // G channel, BGR order
    if (sum / n < 5) throw new Error(`${mrDds}: roughness channel is ~zero after bake — swizzle regression`);
  } finally {
    rmSync(probeDir, { recursive: true, force: true });
  }

  const manifest = { source: path.basename(setDir), images };
  writeFileSync(path.join(setDir, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
  console.log(`  -> ${setDir} (${images.length} image(s))`);
}

function buildBmp(width, height, [r, g, b]) {
  const rowSize = width * 3;
  const pixelDataSize = rowSize * height;
  const buf = Buffer.alloc(54 + pixelDataSize);
  buf.write("BM", 0, "ascii");
  buf.writeUInt32LE(buf.length, 2);
  buf.writeUInt32LE(54, 10);
  buf.writeUInt32LE(40, 14);
  buf.writeInt32LE(width, 18);
  buf.writeInt32LE(height, 22);
  buf.writeUInt16LE(1, 26);
  buf.writeUInt16LE(24, 28);
  buf.writeUInt32LE(0, 30);
  buf.writeUInt32LE(pixelDataSize, 34);
  for (let row = 0; row < height; row++) {
    for (let col = 0; col < width; col++) {
      const off = 54 + row * rowSize + col * 3;
      buf[off] = b;
      buf[off + 1] = g;
      buf[off + 2] = r;
    }
  }
  return buf;
}

function bakeFixtures() {
  console.log("fixtures");
  const outDir = "smirk/engine-renderer/tests/data";
  mkdirSync(outDir, { recursive: true });
  const tmp = mkdtempSync(path.join(tmpdir(), "bake_textures-"));
  try {
    const bake = (name, rgb, flags, outName) => {
      const bmp = path.join(tmp, `${name}.bmp`);
      writeFileSync(bmp, buildBmp(8, 8, rgb));
      runTexconv([...flags, "-o", outDir, bmp]);
      const produced = path.join(outDir, `${name}.dds`);
      const wanted = path.join(outDir, outName);
      if (path.resolve(produced) !== path.resolve(wanted)) renameSync(produced, wanted);
    };
    bake("red8x8", [255, 0, 0], SLOT_FLAGS.base, "red8x8_bc7_srgb.dds");
    bake("gray8x8", [128, 128, 128], SLOT_FLAGS.mr, "gray8x8_bc7_linear.dds");
    bake("tilt8x8", [200, 128, 235], SLOT_FLAGS.normal, "tilt8x8_bc5.dds");
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
  console.log(`  -> ${outDir}`);
}

const [mode, ...args] = process.argv.slice(2);
checkTexconv();
switch (mode) {
  case "gltf":
    for (const a of args) bakeGltfAsset(a);
    break;
  case "ground":
    for (const a of args) bakeGroundSet(a);
    break;
  case "fixtures":
    bakeFixtures();
    break;
  default:
    console.error("usage: node bake_textures.mjs <gltf|ground|fixtures> [args...]");
    process.exit(1);
}
