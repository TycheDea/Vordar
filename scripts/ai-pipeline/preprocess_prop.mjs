// Generated-prop gltf-transform pass: prune + dedup + texture resize to
// <=1024^2, the same @gltf-transform/functions recipe
// scripts/preprocess-characters/preprocess.mjs proved for character glbs.
// Run: node preprocess_prop.mjs <textured.glb> <final.glb>
import { NodeIO } from '@gltf-transform/core';
import { prune, dedup, textureCompress } from '@gltf-transform/functions';
import { statSync } from 'node:fs';

const MAX_TEXTURE_DIM = 1024;
const MAX_OUT_BYTES = 8 * 1024 * 1024;

const [inPath, outPath] = process.argv.slice(2);
if (!inPath || !outPath) {
  console.error('usage: node preprocess_prop.mjs <textured.glb> <final.glb>');
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
  return size && (size[0] > MAX_TEXTURE_DIM || size[1] > MAX_TEXTURE_DIM);
});
if (oversized) {
  await doc.transform(textureCompress({ resize: [MAX_TEXTURE_DIM, MAX_TEXTURE_DIM] }));
}

await io.write(outPath, doc);

const size = statSync(outPath).size;
if (size >= MAX_OUT_BYTES) {
  console.error(`ASSERT FAILED: ${outPath} is ${size} bytes, exceeds ${MAX_OUT_BYTES}`);
  process.exit(1);
}
console.log(`${outPath} ${(size / 1e6).toFixed(2)}MB`);
