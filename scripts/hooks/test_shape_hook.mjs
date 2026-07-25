#!/usr/bin/env node
// PostToolUse hook (Edit/Write). Runs the lint-test-shape scan on the edited
// Rust file, but reports only tests that the same scan does not already flag
// in HEAD, so a pre-existing vacuous test blocks the diff that introduces it
// and nothing after. Any stdin/parse/read failure degrades to exit 0 rather
// than blocking the harness.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { SCOPE_DIRS, repoRoot, scanSource, loadAllowlist } from '../lint-test-shape.mjs';

function main() {
  let payload;
  try {
    payload = JSON.parse(readFileSync(0, 'utf8'));
  } catch {
    process.exit(0);
  }

  const filePath = payload?.tool_input?.file_path;
  if (!filePath || !filePath.endsWith('.rs')) process.exit(0);

  const root = repoRoot();
  const rel = path.relative(root, path.resolve(filePath)).split(path.sep).join('/');
  if (!rel || rel.startsWith('..') || !SCOPE_DIRS.includes(rel.split('/')[0])) process.exit(0);

  let src;
  try {
    src = readFileSync(filePath, 'utf8');
  } catch {
    process.exit(0);
  }

  const allow = loadAllowlist(root);
  const hits = scanSource(src).filter((h) => !allow.has(`${rel}:${h.name}`));
  if (hits.length === 0) process.exit(0);

  const head = spawnSync('git', ['show', `HEAD:${rel}`], { cwd: root, encoding: 'utf8' });
  const prior = new Set(head.status === 0 ? scanSource(head.stdout).map((h) => h.name) : []);
  const fresh = hits.filter((h) => !prior.has(h.name));
  if (fresh.length === 0) process.exit(0);

  const lines = fresh.map((h) => `  ${rel}:${h.line}: ${h.name}`).join('\n');
  process.stderr.write(
    `lint-test-shape: ${fresh.length} new test(s) reach no code - the body only uses literals, ` +
      `consts and assertion macros, so it passes without exercising anything:\n${lines}\n` +
      `Call the production path the test is meant to cover, or pin it in ` +
      `scripts/lint-test-shape-allowlist.txt as path:test_name.\n`,
  );
  process.exit(2);
}

try {
  main();
} catch {
  process.exit(0);
}
