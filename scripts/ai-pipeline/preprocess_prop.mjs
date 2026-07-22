// Generated-prop gltf-transform pass: prune + dedup + texture resize to the
// dimension/size caps below.
// Defaults are the prop caps (VQ-B2); the character chain overrides
// --max-bytes to the 16 MB character cap.
// Run: node preprocess_prop.mjs <textured.glb> <final.glb> [--max-bytes N] [--max-dim N]
import { NodeIO } from '@gltf-transform/core';
import { prune, dedup, textureCompress } from '@gltf-transform/functions';
import { statSync, renameSync, rmSync } from 'node:fs';
import { dirname, basename } from 'node:path';

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

// Write beside the final path and rename in only after the size assert
// passes, so callers gating stage-resume on outPath's existence never see
// an oversized reject left behind by a failed run. The temp name keeps the
// .glb suffix: NodeIO.write picks binary-GLB vs. split-glTF serialization
// by regexing the write path's extension, not the Document contents.
const tmpOutPath = `${dirname(outPath)}/.tmp-${basename(outPath)}`;
await io.write(tmpOutPath, doc);

const size = statSync(tmpOutPath).size;
if (size >= maxOutBytes) {
  rmSync(tmpOutPath);
  console.error(`ASSERT FAILED: ${outPath} is ${size} bytes, exceeds ${maxOutBytes}`);
  process.exit(1);
}
renameSync(tmpOutPath, outPath);
console.log(`${outPath} ${(size / 1e6).toFixed(2)}MB`);
