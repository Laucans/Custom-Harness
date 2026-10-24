// The gate `mock-handlers` of .github/workflows/gates.yml: the mock server's
// handlers are one file per unit, and no id is defined twice.
//
//   node scripts/mock-handlers-gate.mjs [<base sha>]
//
// - every mock/handlers/<id>.js (mock/handlers/_lib/ is the helpers') writes
//   `export const reads = {…}` and/or `export const commands = {…}`, keyed by
//   its own id only — the server imports `<id>.js` for `<id>`, so a key of
//   another id there is a handler nobody reaches;
// - no id is defined twice: not twice in one literal (the later key silently
//   wins — it hid a payload mismatch once), not in its file and in the legacy
//   mock/src/reads.js / mock/src/commands.js;
// - with a base (a pull request), once mock/handlers/ holds a handler file
//   (the README `harness init-repo` seeds and _lib/ do not count — the
//   upgrade lands before any split): no key the base did not have is added
//   to a legacy file — a new handler gets its own file. The legacy handlers
//   already there stay editable until scripts/split-handlers.mjs splits them.
//
// Reads the files as text, runs none of them. Every finding is reported
// (`::error file=…::`), the verdict comes last. Installed by
// `harness init-repo`; the repository owns it from here.

import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import { join, posix } from 'node:path';
import { HANDLERS, LEGACY, Refusal, keysOf, tokenize } from './split-handlers.mjs';

const root = process.cwd();
const base = process.argv[2] || '';
let failed = 0;
const error = (file, message) => {
  failed += 1;
  process.stdout.write(`::error file=${file}::${message}\n`);
};

/** The tokens of `text`; null when it does not parse, reported. */
function tokensOf(text, file) {
  try {
    return tokenize(text, file);
  } catch (e) {
    if (!(e instanceof Refusal)) throw e;
    error(file, `cannot be read: ${e.message}`);
    return null;
  }
}

/** The keys of `name` in `text`, each with its line: `null` without that literal, `[]` when it does not parse (reported). */
function keys(text, name, file, tokens) {
  try {
    return keysOf(text, name, file, tokens);
  } catch (e) {
    if (!(e instanceof Refusal)) throw e;
    error(file, `cannot be read: ${e.message}`);
    return [];
  }
}

/** Reports the keys `found` defines twice. */
function onceEach(found, file, where) {
  const seen = new Map();
  for (const { key, line } of found) {
    if (seen.has(key)) error(file, `${key} is defined twice in ${where} (lines ${seen.get(key)} and ${line}): the later silently wins — keep one`);
    else seen.set(key, line);
  }
}

// Where each id is defined, by kind: id → file.
const defined = { reads: new Map(), commands: new Map() };

for (const legacy of LEGACY) {
  const path = join(root, legacy.path);
  if (!fs.existsSync(path)) continue;
  const found = keys(fs.readFileSync(path, 'utf8'), legacy.literal, legacy.path) || [];
  onceEach(found, legacy.path, legacy.literal);
  for (const { key } of found) defined[legacy.kind].set(key, legacy.path);
}

const dir = join(root, HANDLERS);
const handlers = fs.existsSync(dir) && fs.statSync(dir).isDirectory();
let files = 0;
if (handlers) {
  for (const name of fs.readdirSync(dir).sort()) {
    if (!name.endsWith('.js') || fs.statSync(join(dir, name)).isDirectory()) continue;
    files += 1;
    const file = posix.join(HANDLERS, name);
    const id = name.slice(0, -'.js'.length);
    const text = fs.readFileSync(join(dir, name), 'utf8');
    const tokens = tokensOf(text, file);
    if (!tokens) continue;
    let exported = 0;
    for (const legacy of LEGACY) {
      const found = keys(text, legacy.kind, file, tokens);
      if (found === null) continue;
      exported += 1;
      onceEach(found, file, legacy.kind);
      for (const key of new Set(found.map((k) => k.key))) {
        if (key !== id) {
          error(file, `defines ${key}: a handler file defines its own id only — ${key} belongs in ${HANDLERS}/${key}.js`);
          continue;
        }
        const other = defined[legacy.kind].get(key);
        if (other) error(file, `${key} is defined twice: ${file} and ${other} — keep one`);
        defined[legacy.kind].set(key, file);
      }
    }
    if (!exported) error(file, 'exports neither `export const reads = {…}` nor `export const commands = {…}`');
  }
}

/** The text of `path` at `rev`, empty when it is not there. */
function at(rev, path) {
  try {
    return execFileSync('git', ['show', `${rev}:${path}`], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
  } catch (e) {
    return '';
  }
}

if (base && files > 0) {
  let known = true;
  try {
    execFileSync('git', ['cat-file', '-e', base], { stdio: 'ignore' });
  } catch (e) {
    known = false;
    process.stdout.write(`mock-handlers: no base ${base} to compare with, the legacy files are not compared\n`);
  }
  for (const legacy of LEGACY) {
    if (!known || !fs.existsSync(join(root, legacy.path))) continue;
    const before = at(base, legacy.path);
    let had;
    try {
      had = new Set((keysOf(before, legacy.literal, `${legacy.path} at the base`) || []).map((k) => k.key));
    } catch (e) {
      if (!(e instanceof Refusal)) throw e;
      had = null;
    }
    const now = keys(fs.readFileSync(join(root, legacy.path), 'utf8'), legacy.literal, legacy.path) || [];
    for (const { key, line } of now) {
      const old = had ? had.has(key) : before.includes(`'${key}'`) || before.includes(`"${key}"`);
      if (!old) error(legacy.path, `line ${line} adds ${key} to ${legacy.path}: a new handler goes in its own file, ${HANDLERS}/${key}.js`);
    }
  }
}

const ids = defined.reads.size + defined.commands.size;
if (failed) {
  process.stdout.write(`mock-handlers: ${failed} finding(s)\n`);
  process.exit(1);
}
process.stdout.write(`mock-handlers: ${files} handler file(s), ${ids} id(s) defined once each\n`);
