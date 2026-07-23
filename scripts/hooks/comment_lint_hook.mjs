#!/usr/bin/env node
// PostToolUse hook (Edit/Write). Runs scripts/lint-comments.sh, but only
// when the edited file falls under that script's workspace scope, so
// out-of-scope edits (docs, tasks/, etc.) stay fast. Any stdin/parse/spawn
// failure degrades to exit 0 rather than blocking the harness.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const SCOPE_DIRS = ['smirk', 'game', 'client', 'server', 'benchmarks', 'testing'];
const BASH_EXE = 'C:\\Program Files\\Git\\usr\\bin\\bash.exe';

function repoRoot() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
}

function inScope(filePath, root) {
  const rel = path.relative(root, path.resolve(filePath));
  if (!rel || rel.startsWith('..') || path.isAbsolute(rel)) return false;
  return SCOPE_DIRS.includes(rel.split(path.sep)[0]);
}

function main() {
  let payload;
  try {
    payload = JSON.parse(readFileSync(0, 'utf8'));
  } catch {
    process.exit(0);
  }

  const filePath = payload?.tool_input?.file_path;
  if (!filePath) process.exit(0);

  const root = repoRoot();
  if (!inScope(filePath, root)) process.exit(0);

  const result = spawnSync(BASH_EXE, ['scripts/lint-comments.sh'], { cwd: root, encoding: 'utf8' });
  if (result.error) {
    process.stderr.write(`comment_lint_hook: bash unavailable, skipping lint (${result.error.message})\n`);
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
