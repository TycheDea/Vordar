#!/usr/bin/env node
// PostToolUse hook (Edit/Write). Runs `naga` on the shaders that parse
// standalone; the build.rs-preprocessed shaders and snippets/ files splice
// includes and consts at build time, so they never parse on their own and
// are skipped. Any stdin/parse/spawn failure degrades to exit 0.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const STANDALONE = new Set([
  'bloom.wgsl',
  'ibl.wgsl',
  'mipgen.wgsl',
  'particle_shader.wgsl',
  'shadow.wgsl',
  'tonemap.wgsl',
]);

function main() {
  let payload;
  try {
    payload = JSON.parse(readFileSync(0, 'utf8'));
  } catch {
    process.exit(0);
  }

  const filePath = payload?.tool_input?.file_path;
  if (!filePath || !filePath.endsWith('.wgsl')) process.exit(0);
  if (!STANDALONE.has(path.basename(filePath))) process.exit(0);

  const result = spawnSync('naga', [filePath], { encoding: 'utf8' });
  if (result.error) {
    process.stderr.write(`wgsl_hook: naga unavailable, skipping check (${result.error.message})\n`);
    process.exit(0);
  }

  if (result.status === 0) process.exit(0);

  process.stderr.write((result.stdout ?? '') + (result.stderr ?? ''));
  process.exit(2);
}

try {
  main();
} catch {
  process.exit(0);
}
