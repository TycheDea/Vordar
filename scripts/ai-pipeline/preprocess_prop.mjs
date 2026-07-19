// Generated-prop gltf-transform pass: prune + dedup + texture resize to the
// dimension/size caps below, the same @gltf-transform/functions recipe
// scripts/preprocess-characters/preprocess.mjs proved for character glbs.
// Defaults are the prop caps (VQ-B2); the character chain overrides
// --max-bytes to the 16 MB character cap.
// Run: node preprocess_prop.mjs <textured.glb> <final.glb> [--max-bytes N] [--max-dim N]
import { NodeIO } from '@gltf-transform/core';
import { prune, dedup, textureCompress } from '@gltf-transform/functions';
import { statSync } from 'node:fs';

const DEFAULT_MAX_TEXTURE_DIM = 1024;
const DEFAULT_MAX_OUT_BYTES = 8 * 1024 * 1024;

const positional = [];
let maxTextureDim = DEFAULT_MAX_TEXTURE_DIM;
let maxOutBytes = DEFAULT_MAX_OUT_BYTES;
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i++) {
  if (argv[i] === '--max-bytes') maxOutBytes = Number(argv[++i]);
  else if (argv[i] === '--max-dim') maxTextureDim = Number(argv[++i]);
  else positional.push(argv[i]);
}

const [inPath, outPath] = positional;
if (!inPath || !outPath) {
  console.error('usage: node preprocess_prop.mjs <textured.glb> <final.glb> [--max-bytes N] [--max-dim N]');
  process.exit(1);
}

const io = new NodeIO();
const doc = await io.read(inPath);

await doc.transform(prune(), dedup());

// Only resize (and thereby re-encode) textures that actually exceed the cap:
// the fallback ndarray-pixels encoder used without a 'sharp' instance isn't as
// tight as Blender's PNG writer, so recompressing an already-compliant image
// can grow it. A no-op resize would silently violate the smaller-or-equal
// output invariant this stage exists to guarantee.
const oversized = doc.getRoot().listTextures().some((tex) => {
  const size = tex.getSize();
  return size && (size[0] > maxTextureDim || size[1] > maxTextureDim);
});
if (oversized) {
  await doc.transform(textureCompress({ resize: [maxTextureDim, maxTextureDim] }));
}

await io.write(outPath, doc);

const size = statSync(outPath).size;
if (size >= maxOutBytes) {
  console.error(`ASSERT FAILED: ${outPath} is ${size} bytes, exceeds ${maxOutBytes}`);
  process.exit(1);
}
console.log(`${outPath} ${(size / 1e6).toFixed(2)}MB`);
