#!/usr/bin/env node
// PreToolUse hook (Bash/PowerShell). Blocks force-pushes and recursive
// force-deletes outside the Claude scratchpad, where an accidental history
// rewrite or wipe can't be recovered from a chat correction. Flag matching
// is deliberately simple (word/token boundaries, not a shell parser); a
// stray "--force" inside a quoted commit message false-positiving is an
// acceptable rare cost. Any stdin/parse failure degrades to exit 0.
import { readFileSync } from 'node:fs';

const FORCE_PUSH_FLAG = /(^|\s)(-f|--force|--force-with-lease)(=|\s|$)/;
const SCRATCHPAD = /AppData[\\/]Local[\\/]Temp[\\/]claude/i;

function isForcePush(cmd) {
  return /\bgit\s+push\b/i.test(cmd) && FORCE_PUSH_FLAG.test(cmd);
}

function isRmForceRecursive(cmd) {
  if (!/\brm\b/.test(cmd)) return false;
  const tokens = cmd.split(/\s+/).filter(Boolean);
  const combinedFlag = tokens.some(
    (t) => /^-[A-Za-z]+$/.test(t) && /r/i.test(t) && /f/i.test(t),
  );
  const hasR = tokens.some((t) => t === '-r' || t === '-R' || t === '--recursive');
  const hasF = tokens.some((t) => t === '-f' || t === '--force');
  return combinedFlag || (hasR && hasF);
}

function isRemoveItemForceRecursive(cmd) {
  return /\bRemove-Item\b/i.test(cmd) && /-Recurse\b/i.test(cmd) && /-Force\b/i.test(cmd);
}

function main() {
  let payload;
  try {
    payload = JSON.parse(readFileSync(0, 'utf8'));
  } catch {
    process.exit(0);
  }

  const command = payload?.tool_input?.command;
  if (!command) process.exit(0);

  if (isForcePush(command)) {
    process.stderr.write('deny_dangerous: force-push (-f/--force/--force-with-lease) is blocked\n');
    process.exit(2);
  }

  const isDelete = isRmForceRecursive(command) || isRemoveItemForceRecursive(command);
  if (isDelete && !SCRATCHPAD.test(command)) {
    process.stderr.write('deny_dangerous: recursive force-delete outside the scratchpad is blocked\n');
    process.exit(2);
  }

  process.exit(0);
}

try {
  main();
} catch {
  process.exit(0);
}
