#!/usr/bin/env node
// Flags a #[test] whose body reaches no code at all: only literals, consts and
// assertion macros. Such a test is green by construction, so it proves nothing
// about the crate it lives in.
//
//   node scripts/lint-test-shape.mjs           # scan the workspace crates
//   node scripts/lint-test-shape.mjs a.rs b.rs # scan specific files
//
// A call of any kind - free function, method, associated function, or a macro
// outside the std assertion/formatting set - counts as reaching code, so the
// scan is deliberately one-sided: a missed vacuous test only weakens the gate,
// while a wrong hit blocks an edit. Known-good exceptions are pinned in
// lint-test-shape-allowlist.txt as path:test_name.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

export const SCOPE_DIRS = ['smirk', 'game', 'client', 'server', 'benchmarks', 'testing'];

// std macros that expand to literals, formatting or an assertion: seeing one
// is not evidence the test reached production code.
const BENIGN_MACROS = new Set([
  'assert', 'assert_eq', 'assert_ne',
  'debug_assert', 'debug_assert_eq', 'debug_assert_ne',
  'panic', 'unreachable', 'todo', 'unimplemented',
  'format', 'print', 'println', 'eprint', 'eprintln', 'write', 'writeln',
  'vec', 'matches', 'dbg', 'concat', 'stringify', 'include_str', 'include_bytes',
  'env', 'option_env', 'line', 'file', 'column', 'cfg',
]);

// Keywords that can legally sit immediately before `(` without being a callee.
const KEYWORDS = new Set([
  'if', 'while', 'for', 'match', 'return', 'in', 'else', 'loop', 'unsafe',
  'as', 'let', 'mut', 'ref', 'move', 'where', 'impl', 'fn', 'struct', 'enum',
  'const', 'static', 'type', 'use', 'mod', 'pub', 'dyn', 'break', 'continue',
  'await', 'async', 'yield', 'box', 'crate', 'super',
]);

export function repoRoot() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
}

// Blanks out comments and the contents of string/char literals, preserving
// offsets and newlines so brace matching and line numbers stay exact.
function mask(src) {
  const out = src.split('');
  const n = src.length;
  const blank = (k) => { if (src[k] !== '\n') out[k] = ' '; };
  let i = 0;
  while (i < n) {
    const c = src[i];
    if (c === '/' && src[i + 1] === '/') {
      while (i < n && src[i] !== '\n') blank(i++);
    } else if (c === '/' && src[i + 1] === '*') {
      let depth = 0;
      while (i < n) {
        if (src[i] === '/' && src[i + 1] === '*') { depth++; blank(i); blank(i + 1); i += 2; continue; }
        if (src[i] === '*' && src[i + 1] === '/') { depth--; blank(i); blank(i + 1); i += 2; if (depth === 0) break; continue; }
        blank(i++);
      }
    } else if (c === 'r' && (src[i + 1] === '"' || src[i + 1] === '#')) {
      let j = i + 1;
      let hashes = 0;
      while (src[j] === '#') { hashes++; j++; }
      if (src[j] !== '"') { i++; continue; }
      const close = '"' + '#'.repeat(hashes);
      const end = src.indexOf(close, j + 1);
      const stop = end === -1 ? n : end + close.length;
      for (let k = i; k < stop; k++) blank(k);
      i = stop;
    } else if (c === '"') {
      blank(i++);
      while (i < n) {
        if (src[i] === '\\') { blank(i); if (i + 1 < n) blank(i + 1); i += 2; continue; }
        const done = src[i] === '"';
        blank(i++);
        if (done) break;
      }
    } else if (c === "'") {
      const m = /^'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^\\'])'/.exec(src.slice(i, i + 12));
      if (m) { for (let k = i; k < i + m[0].length; k++) blank(k); i += m[0].length; } else i++;
    } else i++;
  }
  return out.join('');
}

function matchDelim(masked, open, openCh, closeCh) {
  let depth = 0;
  for (let i = open; i < masked.length; i++) {
    if (masked[i] === openCh) depth++;
    else if (masked[i] === closeCh) { depth--; if (depth === 0) return i; }
  }
  return -1;
}

// From just past a #[test] attribute, locate the function it decorates.
function findFn(masked, from) {
  const head = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  head.lastIndex = from;
  const m = head.exec(masked);
  if (!m) return null;
  if (/[{};]/.test(masked.slice(from, m.index))) return null;
  const argsOpen = masked.indexOf('(', head.lastIndex);
  if (argsOpen === -1) return null;
  const argsClose = matchDelim(masked, argsOpen, '(', ')');
  if (argsClose === -1) return null;
  const bodyOpen = masked.indexOf('{', argsClose);
  if (bodyOpen === -1) return null;
  const bodyClose = matchDelim(masked, bodyOpen, '{', '}');
  if (bodyClose === -1) return null;
  return { name: m[1], nameAt: m.index, body: masked.slice(bodyOpen + 1, bodyClose) };
}

function reachesCode(body) {
  const flat = body.replace(/::\s*<[^;{}]*?>\s*\(/g, '(');
  for (const m of flat.matchAll(/([A-Za-z_][A-Za-z0-9_]*)\s*\(/g)) {
    if (!KEYWORDS.has(m[1])) return true;
  }
  for (const m of flat.matchAll(/([A-Za-z_][A-Za-z0-9_]*)\s*!\s*[([{]/g)) {
    if (!BENIGN_MACROS.has(m[1])) return true;
  }
  return false;
}

export function scanSource(src) {
  const masked = mask(src);
  const attr = /#\s*\[\s*(?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*test\s*\]/g;
  const hits = [];
  let m;
  while ((m = attr.exec(masked)) !== null) {
    const fn = findFn(masked, attr.lastIndex);
    if (!fn || reachesCode(fn.body)) continue;
    hits.push({ name: fn.name, line: src.slice(0, fn.nameAt).split('\n').length });
  }
  return hits;
}

export function loadAllowlist(root) {
  try {
    return new Set(
      readFileSync(path.join(root, 'scripts', 'lint-test-shape-allowlist.txt'), 'utf8')
        .split('\n')
        .map((l) => l.trim())
        .filter((l) => l && !l.startsWith('#')),
    );
  } catch {
    return new Set();
  }
}

export function relKey(root, file, name) {
  return `${path.relative(root, path.resolve(file)).split(path.sep).join('/')}:${name}`;
}

function workspaceRustFiles(root) {
  const ls = spawnSync('git', ['ls-files', '-co', '--exclude-standard', '--', ...SCOPE_DIRS], {
    cwd: root,
    encoding: 'utf8',
  });
  if (ls.status !== 0) return [];
  return ls.stdout.split('\n').filter((f) => f.endsWith('.rs')).map((f) => path.join(root, f));
}

function main() {
  const root = repoRoot();
  const args = process.argv.slice(2);
  const files = args.length ? args : workspaceRustFiles(root);
  const allow = loadAllowlist(root);
  let tests = 0;
  let hits = 0;
  for (const file of files) {
    let src;
    try {
      src = readFileSync(file, 'utf8');
    } catch {
      continue;
    }
    tests += (mask(src).match(/#\s*\[\s*(?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*test\s*\]/g) ?? []).length;
    for (const hit of scanSource(src)) {
      const key = relKey(root, file, hit.name);
      if (allow.has(key)) continue;
      console.log(`${key.split(':')[0]}:${hit.line}: ${hit.name} reaches no code`);
      hits++;
    }
  }
  console.log(`lint-test-shape: ${hits} hit(s) over ${tests} test(s) in ${files.length} file(s)`);
  process.exit(hits > 0 ? 1 : 0);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
