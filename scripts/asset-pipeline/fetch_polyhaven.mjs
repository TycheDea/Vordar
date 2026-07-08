// Fetch Poly Haven models (CC0) as glTF into content/models/props/<slug>/.
// No dependencies — node >= 18 (global fetch).
//
//   node fetch_polyhaven.mjs [--res 1k] <slug> [<slug> ...]
//
// Downloads the .gltf plus every file it includes (bin + textures) at the
// chosen resolution, preserving relative paths so `load_gltf_data` resolves
// them. Record every asset fetched here in content/source/CREDITS.md.

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, basename } from "node:path";

const args = process.argv.slice(2);
const resIdx = args.indexOf("--res");
const res = resIdx >= 0 ? args.splice(resIdx, 2)[1] : "1k";
const slugs = args;
if (slugs.length === 0) {
  console.error("usage: node fetch_polyhaven.mjs [--res 1k] <slug> ...");
  process.exit(1);
}

async function download(url, dest) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  await mkdir(dirname(dest), { recursive: true });
  await writeFile(dest, Buffer.from(await r.arrayBuffer()));
  console.log(`  ${dest}`);
}

for (const slug of slugs) {
  console.log(`${slug} (${res}):`);
  const files = await (await fetch(`https://api.polyhaven.com/files/${slug}`)).json();
  const entry = files?.gltf?.[res]?.gltf;
  if (!entry) {
    console.error(`  no gltf/${res} for '${slug}' — available: ${Object.keys(files?.gltf ?? {})}`);
    process.exitCode = 1;
    continue;
  }
  const root = join("content", "models", "props", slug);
  await download(entry.url, join(root, basename(new URL(entry.url).pathname)));
  for (const [rel, file] of Object.entries(entry.include ?? {})) {
    await download(file.url, join(root, rel));
  }
}
