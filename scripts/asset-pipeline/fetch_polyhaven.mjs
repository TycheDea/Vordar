// Fetch Poly Haven assets (CC0) into content/{models/props,textures}/<slug>/.
// No dependencies — node >= 18 (global fetch).
//
//   node fetch_polyhaven.mjs [--res 1k] [--type models|hdris|textures] <slug> [<slug> ...]
//
// models (default): the .gltf plus every file it includes (bin + textures),
// preserving relative paths so `load_gltf_data` resolves them, into
// content/models/props/<slug>/.
// hdris: the .hdr file into content/textures/env/<slug>_<res>.hdr.
// textures: every available map (Diffuse, nor_gl, Rough, AO, ...) into
// content/textures/<slug>/<mapname>_<res>.<ext>.
//
// Record every asset fetched here in content/source/CREDITS.md.

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, basename } from "node:path";

const args = process.argv.slice(2);
const resIdx = args.indexOf("--res");
const res = resIdx >= 0 ? args.splice(resIdx, 2)[1] : "1k";
const typeIdx = args.indexOf("--type");
const type = typeIdx >= 0 ? args.splice(typeIdx, 2)[1] : "models";
const slugs = args;
if (slugs.length === 0) {
  console.error("usage: node fetch_polyhaven.mjs [--res 1k] [--type models|hdris|textures] <slug> ...");
  process.exit(1);
}
if (!["models", "hdris", "textures"].includes(type)) {
  console.error(`unknown --type '${type}' — expected models, hdris, or textures`);
  process.exit(1);
}

async function download(url, dest) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  await mkdir(dirname(dest), { recursive: true });
  await writeFile(dest, Buffer.from(await r.arrayBuffer()));
  console.log(`  ${dest}`);
}

async function fetchModel(files, slug) {
  const entry = files?.gltf?.[res]?.gltf;
  if (!entry) {
    console.error(`  no gltf/${res} for '${slug}' — available: ${Object.keys(files?.gltf ?? {})}`);
    return false;
  }
  const root = join("content", "models", "props", slug);
  await download(entry.url, join(root, basename(new URL(entry.url).pathname)));
  for (const [rel, file] of Object.entries(entry.include ?? {})) {
    await download(file.url, join(root, rel));
  }
  return true;
}

async function fetchHdri(files, slug) {
  const entry = files?.hdri?.[res]?.hdr;
  if (!entry) {
    console.error(`  no hdri/${res} for '${slug}' — available: ${Object.keys(files?.hdri ?? {})}`);
    return false;
  }
  await download(entry.url, join("content", "textures", "env", `${slug}_${res}.hdr`));
  return true;
}

// jpg preferred to match every existing raster convention in this repo; the
// fallback also skips non-raster companion entries (blend, gltf, mtlx) since
// none of those formats appear under any Poly Haven texture map.
const TEXTURE_EXTS = ["jpg", "png", "exr"];

async function fetchTextures(files, slug) {
  let any = false;
  for (const [mapName, byRes] of Object.entries(files)) {
    const byExt = byRes[res];
    const ext = byExt && TEXTURE_EXTS.find((e) => e in byExt);
    if (!ext) continue;
    await download(byExt[ext].url, join("content", "textures", slug, `${mapName}_${res}.${ext}`));
    any = true;
  }
  if (!any) {
    console.error(`  no texture maps at ${res} for '${slug}'`);
    return false;
  }
  return true;
}

const fetchers = { models: fetchModel, hdris: fetchHdri, textures: fetchTextures };

for (const slug of slugs) {
  console.log(`${slug} (${res}):`);
  const files = await (await fetch(`https://api.polyhaven.com/files/${slug}`)).json();
  if (!(await fetchers[type](files, slug))) process.exitCode = 1;
}
