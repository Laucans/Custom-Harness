// Splits the mock server's two legacy handler files — mock/src/reads.js
// (`export const READS = {…}`) and mock/src/commands.js
// (`export const COMMANDS = {…}`) — into one file per unit,
// mock/handlers/<id>.js, which the mock server (mock/src/server.js) imports
// at run time:
//
//   export const reads = { '<Capability>': async (server, variables) => … };
//   export const commands = { '<DataCapability>': async (server, { payload }) => … };
//
// Each handler's text is carried byte for byte, with the comment lines above
// its key; what the legacy files declare beside their literal (helpers,
// constants, imports) becomes mock/handlers/_lib/reads.js and _lib/commands.js,
// every declaration exported, and each handler file imports from them only
// the names it uses. Run once per tree, from the repository's root:
//
//   node scripts/split-handlers.mjs
//
// It refuses, writing nothing, when it cannot split faithfully: node older
// than 22 (the legacy files are ES modules without a package.json `type`), a
// key computed, spread, shorthand, getter or defined twice, a handler that
// assigns a top-level `let`/`var` of its file (an imported binding is
// read-only: `x =`, `x += …`, `++x`, `[x] = …`, `for (x of …)`), a handler
// using `this` (each file's literal holds one handler), a reference to
// READS / COMMANDS outside their literal, another
// mock/src/ module still importing a legacy file (the harness's server.js
// first), or a handler file already there with other content. Before writing,
// a self-check runs the mock server twice in a scratch copy, once on the
// legacy files and once on the split ones, and requires the same handler set,
// each handler's source unchanged, and the same answers to a read of every
// Capability and a command of every DataCapability on the fixtures. The
// output is deterministic: two trees with the same legacy files split the
// same, so a branch split on its own merges cleanly with one split
// elsewhere. Run on a tree already split, it changes nothing. Installed by
// `harness init-repo`; the repository owns it from here. Written in the
// syntax an old node parses, so that one is refused rather than crashing.

import { spawnSync } from 'node:child_process';
import * as fs from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, posix, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

/** The oldest node that imports an ES module `.js` without a package.json `type`. */
export const MIN_NODE = 22;

/** Whether `version` (`process.versions.node`) is recent enough. */
export function nodeIsRecent(version) {
  return Number(String(version).split('.')[0]) >= MIN_NODE;
}

/** The legacy files, by kind: their path, the literal they export, the export of a handler file. */
export const LEGACY = [
  { kind: 'reads', path: 'mock/src/reads.js', literal: 'READS' },
  { kind: 'commands', path: 'mock/src/commands.js', literal: 'COMMANDS' },
];

/** Where the handler files go, and their shared helpers. */
export const HANDLERS = 'mock/handlers';
const LIB = 'mock/handlers/_lib';

/** A refusal: the split stops, nothing written. */
export class Refusal extends Error {}

const OPS = ['>>>=', '...', '===', '!==', '**=', '<<=', '>>=', '>>>', '&&=', '||=', '??=', '=>', '==', '!=', '<=', '>=', '&&', '||', '??', '?.', '++', '--', '+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '**', '<<', '>>'];
const ASSIGN = new Set(['=', '+=', '-=', '*=', '/=', '%=', '**=', '<<=', '>>=', '>>>=', '&=', '|=', '^=', '&&=', '||=', '??=', '++', '--']);
const BEFORE_REGEX = new Set(['return', 'typeof', 'instanceof', 'in', 'of', 'new', 'delete', 'void', 'throw', 'case', 'do', 'else', 'yield', 'await']);
const ID_START = /[A-Za-z_$#\u0080-\uffff]/;
const ID_PART = /[\w$\u0080-\uffff]/;
const CLOSER = { ')': '(', ']': '[', '}': '{' };

/** The line of offset `at` in `src`, from 1. */
function lineOf(src, at) {
  return src.slice(0, at).split('\n').length;
}

/**
 * The tokens of a JavaScript source: `{ type, value, start, end, depth }`,
 * `type` one of id, num, str, tpl, re, op; comments and blanks dropped;
 * `depth` the brackets open around the token (an opener counts outside
 * itself). Template substitutions are tokenized too. Throws a Refusal on
 * what it cannot read.
 */
export function tokenize(src, file) {
  const tokens = [];
  const stack = [];
  let i = 0;
  const fail = (why) => { throw new Refusal(`${file || 'source'}:${lineOf(src, i)}: ${why}`); };
  const push = (type, start, extra) => {
    const token = { type, value: src.slice(start, i), start, end: i, depth: stack.length };
    if (extra) Object.assign(token, extra);
    tokens.push(token);
  };
  const regexAllowed = () => {
    const prev = tokens[tokens.length - 1];
    if (!prev) return true;
    if (prev.type === 'id') return BEFORE_REGEX.has(prev.value);
    if (prev.type === 'op') return prev.value !== ')' && prev.value !== ']';
    return prev.type === 'tpl' && prev.open === true;
  };
  const chunk = (start) => {
    while (i < src.length) {
      const c = src[i];
      if (c === '\\') { i += 2; continue; }
      if (c === '`') { i += 1; push('tpl', start, { open: false }); return; }
      if (c === '$' && src[i + 1] === '{') { i += 2; push('tpl', start, { open: true }); stack.push('${'); return; }
      i += 1;
    }
    fail('a template literal never closed');
  };
  while (i < src.length) {
    const c = src[i];
    if (/\s/.test(c)) { i += 1; continue; }
    if (c === '/' && src[i + 1] === '/') { const n = src.indexOf('\n', i); i = n < 0 ? src.length : n; continue; }
    if (c === '/' && src[i + 1] === '*') { const n = src.indexOf('*/', i + 2); if (n < 0) fail('a comment never closed'); i = n + 2; continue; }
    const start = i;
    if (c === '"' || c === "'") {
      i += 1;
      while (i < src.length && src[i] !== c) {
        if (src[i] === '\n') fail('a string across lines');
        i += src[i] === '\\' ? 2 : 1;
      }
      if (i >= src.length) fail('a string never closed');
      i += 1;
      push('str', start);
      continue;
    }
    if (c === '`') { i += 1; chunk(start); continue; }
    if (c === '}' && stack[stack.length - 1] === '${') { stack.pop(); i += 1; chunk(start); continue; }
    if (c === '/' && regexAllowed()) {
      i += 1;
      let inClass = false;
      for (;;) {
        if (i >= src.length || src[i] === '\n') fail('a regular expression never closed');
        const d = src[i];
        if (d === '\\') { i += 2; continue; }
        if (inClass) { if (d === ']') inClass = false; } else if (d === '[') inClass = true; else if (d === '/') break;
        i += 1;
      }
      i += 1;
      while (i < src.length && ID_PART.test(src[i])) i += 1;
      push('re', start);
      continue;
    }
    if (ID_START.test(c)) {
      i += 1;
      while (i < src.length && ID_PART.test(src[i])) i += 1;
      push('id', start);
      continue;
    }
    if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(src[i + 1] || ''))) {
      i += 1;
      while (i < src.length && /[\w.]/.test(src[i])) i += 1;
      push('num', start);
      continue;
    }
    if (c === '(' || c === '[' || c === '{') { i += 1; push('op', start); stack.push(c); continue; }
    if (CLOSER[c]) {
      if (stack.pop() !== CLOSER[c]) fail(`an unbalanced ${c}`);
      i += 1;
      push('op', start);
      continue;
    }
    const op = OPS.find((o) => src.startsWith(o, i)) || c;
    i += op.length;
    push('op', start);
  }
  if (stack.length) fail('a bracket never closed');
  return tokens;
}

const is = (token, type, value) => Boolean(token) && token.type === type && (value === undefined || token.value === value);

/** The value of a key token: a string's text unquoted, an identifier or a number as written. */
function keyOf(token) {
  if (token.type !== 'str') return token.value;
  const inner = token.value.slice(1, -1);
  if (!inner.includes('\\')) return inner;
  return JSON.parse(`"${inner.replace(/\\'/g, "'").replace(/"/g, '\\"')}"`);
}

/**
 * The object literal of `export const <name> = { … }` at the top of a
 * file: `{ open, close, start, end }` (token indexes of its braces, and the
 * statement's offsets, its `;` included), or null when there is none.
 */
export function literalOf(tokens, name) {
  for (let k = 0; k + 4 < tokens.length; k += 1) {
    if (tokens[k].depth !== 0 || !is(tokens[k], 'id', 'export')) continue;
    if (!is(tokens[k + 1], 'id', 'const') || !is(tokens[k + 2], 'id', name) || !is(tokens[k + 3], 'op', '=') || !is(tokens[k + 4], 'op', '{')) continue;
    const open = k + 4;
    let close = open + 1;
    while (!(tokens[close].depth === 0 && is(tokens[close], 'op', '}'))) close += 1;
    const semi = is(tokens[close + 1], 'op', ';') ? close + 1 : close;
    return { open, close, start: tokens[k].start, end: tokens[semi].end };
  }
  return null;
}

/**
 * The properties of the literal between token indexes `open` and `close`:
 * `{ key, first, last, start, end, keyIndex }` each (token indexes and
 * offsets). Refuses what is not a plain `key: value` or a method.
 */
export function propertiesOf(tokens, open, close, file) {
  const depth = tokens[open].depth + 1;
  const props = [];
  let k = open + 1;
  while (k < close) {
    const first = tokens[k];
    const at = (why) => `${file}: ${why} at offset ${first.start}`;
    if (is(first, 'op', '...')) throw new Refusal(at('a spread entry in the literal — write each handler under its own key'));
    let j = k;
    while (is(tokens[j], 'id') && ['async', 'get', 'set'].includes(tokens[j].value) && !(is(tokens[j + 1], 'op') && [':', '(', ',', '}'].includes(tokens[j + 1].value))) {
      if (tokens[j].value !== 'async') throw new Refusal(at('a getter or a setter — write the handler as a function'));
      j += 1;
    }
    if (is(tokens[j], 'op', '*')) j += 1;
    const keyToken = tokens[j];
    if (is(keyToken, 'op', '[')) throw new Refusal(at('a computed key — write the identifier as a string'));
    if (!keyToken || !['str', 'id', 'num'].includes(keyToken.type)) throw new Refusal(at('an entry whose key cannot be read'));
    const after = tokens[j + 1];
    const method = is(after, 'op', '(');
    if (!method && !is(after, 'op', ':')) throw new Refusal(at(`the shorthand entry \`${keyToken.value}\` — write the handler in the literal`));
    if (!method && j !== k) throw new Refusal(at('a modifier before a `key:` entry'));
    let m = j + 1;
    while (m < close && !(tokens[m].depth === depth && is(tokens[m], 'op', ','))) m += 1;
    props.push({ key: keyOf(keyToken), keyIndex: j, first: k, last: m - 1, start: first.start, end: tokens[m - 1].end, comma: m < close ? m : -1 });
    k = m + 1;
  }
  return props;
}

/** The keys defined twice in `props`, each with the lines of its definitions. */
export function definedTwice(props, src) {
  const lines = new Map();
  for (const p of props) {
    if (!lines.has(p.key)) lines.set(p.key, []);
    lines.get(p.key).push(lineOf(src, p.start));
  }
  return [...lines].filter(([, at]) => at.length > 1).map(([key, at]) => ({ key, lines: at }));
}

/** The keys a source defines in `export const <name> = {…}`, each with its line: `null` without that literal. */
export function keysOf(src, name, file, tokens) {
  if (!tokens) tokens = tokenize(src, file);
  const literal = literalOf(tokens, name);
  if (!literal) return null;
  return propertiesOf(tokens, literal.open, literal.close, file).map((p) => ({ key: p.key, line: lineOf(src, p.start) }));
}

/** The keywords that open a statement, so a line starting with one after a complete expression begins a new statement (ASI). */
const STATEMENT = new Set(['import', 'export', 'const', 'let', 'var', 'function', 'class', 'async']);
/** The tokens after which an expression cannot be complete: a newline there continues it. */
const CONTINUES = new Set(['export', 'default', 'async', 'await', 'yield', 'new', 'typeof', 'void', 'delete', 'in', 'of', 'instanceof', 'return', 'throw', 'case', 'else', 'do', 'extends']);

/**
 * Whether the token at `n` starts a statement at the top of the file: after
 * a `;` or a `}`, or — a file written without semicolons — a statement
 * keyword at the start of a line that follows a complete expression.
 */
function startsStatement(tokens, n, src) {
  const prev = tokens[n - 1];
  if (!prev) return true;
  if (prev.depth !== 0) return false;
  if (is(prev, 'op') && (prev.value === ';' || prev.value === '}')) return true;
  const t = tokens[n];
  if (!is(t, 'id') || !STATEMENT.has(t.value) || !src.slice(prev.end, t.start).includes('\n')) return false;
  if (is(prev, 'op')) return [')', ']', '++', '--'].includes(prev.value);
  if (prev.type === 'tpl') return !prev.open;
  return !(prev.type === 'id' && CONTINUES.has(prev.value));
}

/**
 * What a file declares at its top, the literal left out: `imports` (each
 * `{ start, end, spec, names: [{ local, clause }] }`), `declared` (name →
 * `{ start, exported }`), `mutable` (the `let`/`var` names).
 */
export function topLevelOf(src, file) {
  const tokens = tokenize(src, file);
  const imports = [];
  const declared = new Map();
  const mutable = new Set();
  for (let n = 0; n < tokens.length; n += 1) {
    const t = tokens[n];
    if (t.depth !== 0 || !startsStatement(tokens, n, src)) continue;
    if (is(t, 'id', 'import') && !is(tokens[n + 1], 'op', '(') && !is(tokens[n + 1], 'op', '.')) {
      let e = n + 1;
      const names = [];
      if (!is(tokens[e], 'str')) {
        if (is(tokens[e], 'id') && !is(tokens[e], 'id', 'from')) { names.push({ local: tokens[e].value, clause: 'default' }); e += 1; if (is(tokens[e], 'op', ',')) e += 1; }
        if (is(tokens[e], 'op', '*')) { names.push({ local: tokens[e + 2].value, clause: 'namespace' }); e += 3; }
        if (is(tokens[e], 'op', '{')) {
          e += 1;
          while (!is(tokens[e], 'op', '}')) {
            const imported = tokens[e].value;
            let local = imported;
            if (is(tokens[e + 1], 'id', 'as')) { local = tokens[e + 2].value; e += 2; }
            names.push({ local, clause: imported === local ? imported : `${imported} as ${local}` });
            e += 1;
            if (is(tokens[e], 'op', ',')) e += 1;
          }
          e += 1;
        }
        if (!is(tokens[e], 'id', 'from')) throw new Refusal(`${file}:${lineOf(src, t.start)}: an import this script cannot read`);
        e += 1;
      }
      const spec = keyOf(tokens[e]);
      const end = is(tokens[e + 1], 'op', ';') ? tokens[e + 1].end : tokens[e].end;
      imports.push({ start: t.start, end, spec, names });
      continue;
    }
    let d = n;
    const exported = is(tokens[d], 'id', 'export');
    if (exported) d += 1;
    if (is(tokens[d], 'id', 'default')) continue;
    if (is(tokens[d], 'id', 'async')) d += 1;
    if (is(tokens[d], 'id', 'function') || is(tokens[d], 'id', 'class')) {
      const name = is(tokens[d + 1], 'op', '*') ? tokens[d + 2] : tokens[d + 1];
      if (is(name, 'id')) declared.set(name.value, { start: t.start, exported });
      continue;
    }
    if (is(tokens[d], 'id') && ['const', 'let', 'var'].includes(tokens[d].value)) {
      const keyword = tokens[d].value;
      let e = d + 1;
      for (;;) {
        const name = tokens[e];
        if (!is(name, 'id')) throw new Refusal(`${file}:${lineOf(src, t.start)}: a top-level destructuring declaration — declare each name on its own`);
        declared.set(name.value, { start: t.start, exported });
        if (keyword !== 'const') mutable.add(name.value);
        e += 1;
        while (e < tokens.length && !(tokens[e].depth === 0 && is(tokens[e], 'op') && (tokens[e].value === ',' || tokens[e].value === ';')) && !startsStatement(tokens, e, src)) e += 1;
        if (e >= tokens.length || tokens[e].value !== ',') break;
        e += 1;
      }
    }
  }
  return { imports, declared, mutable };
}

/** The identifiers a token range uses as bindings: not a member after `.`, not the entry's own key. */
function referencesIn(tokens, from, to, skip) {
  const used = new Set();
  for (let n = from; n <= to; n += 1) {
    if (n === skip || tokens[n].type !== 'id') continue;
    const prev = tokens[n - 1];
    if (prev && is(prev, 'op') && (prev.value === '.' || prev.value === '?.')) continue;
    used.add(tokens[n].value);
  }
  return used;
}

/** The index of the bracket closing the opener at `open`. */
function closerOf(tokens, open) {
  let c = open + 1;
  while (c < tokens.length && !(tokens[c].depth === tokens[open].depth && is(tokens[c], 'op') && CLOSER[tokens[c].value])) c += 1;
  return c;
}

/** Whether the token at `n` opens the head of a `for (… of/in …)`: after `for (` or `for await (`. */
function inForHead(tokens, n) {
  if (!is(tokens[n - 1], 'op', '(')) return false;
  return is(tokens[n - 2], 'id', 'for') || (is(tokens[n - 2], 'id', 'await') && is(tokens[n - 3], 'id', 'for'));
}

/**
 * The first assignment, in a token range, to one of `names`: the name, or
 * null. An assignment is `name = …` (or `op=`), `++name` / `name++`, a
 * destructuring pattern holding the name followed by `=` (`[name] = …`,
 * `({ name } = …)`), or the binding of a `for (name of/in …)` head.
 */
function assignsOneOf(tokens, from, to, names) {
  const named = (n) => {
    const t = tokens[n];
    if (!t || t.type !== 'id' || !names.has(t.value)) return false;
    const prev = tokens[n - 1];
    return !(prev && is(prev, 'op') && (prev.value === '.' || prev.value === '?.'));
  };
  for (let n = from; n <= to; n += 1) {
    const t = tokens[n];
    if (named(n)) {
      const prev = tokens[n - 1];
      const next = tokens[n + 1];
      if ((next && is(next, 'op') && ASSIGN.has(next.value)) || (prev && is(prev, 'op') && (prev.value === '++' || prev.value === '--'))) return t.value;
      if (inForHead(tokens, n) && (is(next, 'id', 'of') || is(next, 'id', 'in'))) return t.value;
    }
    if (!is(t, 'op', '[') && !is(t, 'op', '{')) continue;
    const declares = is(tokens[n - 1], 'id') && ['const', 'let', 'var'].includes(tokens[n - 1].value);
    if (declares) continue;
    const c = closerOf(tokens, n);
    const after = tokens[c + 1];
    const pattern = is(after, 'op', '=') || (inForHead(tokens, n) && (is(after, 'id', 'of') || is(after, 'id', 'in')));
    if (!pattern) continue;
    for (let m = n + 1; m < c; m += 1) {
      if (named(m) && !is(tokens[m + 1], 'op', ':')) return tokens[m].value;
    }
  }
  return null;
}

/** Where a relative import of a legacy file's leads once moved to `from` (a folder of the repository). */
function movedSpec(spec, from) {
  if (!spec.startsWith('.')) return spec;
  const target = posix.normalize(posix.join('mock/src', spec));
  const legacy = LEGACY.find((l) => l.path === target);
  const moved = legacy ? posix.join(LIB, posix.basename(target)) : target;
  const rel = posix.relative(from, moved);
  return rel.startsWith('.') ? rel : `./${rel}`;
}

/** The import line of `names` from `spec`. */
function importLine(names, spec) {
  const defaults = names.filter((n) => n.clause === 'default').map((n) => n.local);
  const spaces = names.filter((n) => n.clause === 'namespace').map((n) => `* as ${n.local}`);
  const named = names.filter((n) => n.clause !== 'default' && n.clause !== 'namespace').map((n) => n.clause).sort();
  const parts = defaults.concat(spaces);
  if (named.length) parts.push(`{ ${named.join(', ')} }`);
  return `import ${parts.join(', ')} from '${spec}';`;
}

/** The start of the line holding `at`. */
function lineStart(src, at) {
  return src.lastIndexOf('\n', at - 1) + 1;
}

/**
 * The split of one legacy file, in memory: `{ files: Map(path → text), ids,
 * lib }`, or a Refusal. `src` is its text; `kind`, `path` and `literal`
 * come from LEGACY.
 */
export function splitOne(src, legacy) {
  const { kind, path, literal } = legacy;
  const tokens = tokenize(src, path);
  const found = literalOf(tokens, literal);
  if (!found) throw new Refusal(`${path} has no \`export const ${literal} = {…}\``);
  const props = propertiesOf(tokens, found.open, found.close, path);
  const twice = definedTwice(props, src);
  if (twice.length) throw new Refusal(`${path} defines ${twice.map((t) => `${t.key} twice (lines ${t.lines.join(', ')})`).join('; ')}: the later silently wins — keep one`);
  const cut = lineStart(src, found.start);
  const after = src[found.end] === '\n' ? found.end + 1 : found.end;
  const rest = src.slice(0, cut) + src.slice(after);
  const top = topLevelOf(rest, path);
  const restTokens = tokenize(rest, path);
  const restUses = referencesIn(restTokens, 0, restTokens.length - 1, -1);
  for (const name of ['READS', 'COMMANDS']) {
    if (restUses.has(name)) throw new Refusal(`${path} uses ${name} outside its literal: it is gone once split`);
  }
  const sources = new Map();
  for (const name of top.declared.keys()) sources.set(name, { spec: null, clause: name });
  for (const imp of top.imports) for (const n of imp.names) sources.set(n.local, { spec: imp.spec, clause: n.clause });
  const files = new Map();
  const ids = [];
  props.forEach((p, index) => {
    if (!/^[\w.-]+$/.test(p.key) || p.key.startsWith('.')) throw new Refusal(`${path}: the key ${JSON.stringify(p.key)} cannot name a file`);
    const used = referencesIn(tokens, p.first, p.last, p.keyIndex);
    for (const name of ['READS', 'COMMANDS']) if (used.has(name)) throw new Refusal(`${path}: ${p.key} uses ${name}, which is gone once split — call the helper it needs instead`);
    const assigned = assignsOneOf(tokens, p.first, p.last, top.mutable);
    if (assigned) throw new Refusal(`${path}: ${p.key} assigns the top-level \`${assigned}\` of its file, which the split file would import read-only — move that assignment into a helper of the file`);
    for (let n = p.first; n <= p.last; n += 1) {
      const prev = tokens[n - 1];
      if (is(tokens[n], 'id', 'this') && !(prev && is(prev, 'op') && (prev.value === '.' || prev.value === '?.'))) throw new Refusal(`${path}: ${p.key} uses \`this\`, which no longer holds the other handlers once split — call a helper of the file by name instead`);
    }
    const bySpec = new Map();
    for (const name of [...used].sort()) {
      const from = sources.get(name);
      if (!from) continue;
      const spec = from.spec === null ? `./_lib/${posix.basename(path)}` : movedSpec(from.spec, HANDLERS);
      if (!bySpec.has(spec)) bySpec.set(spec, []);
      bySpec.get(spec).push({ local: name, clause: from.spec === null ? name : from.clause });
    }
    const lines = [...bySpec.keys()].sort().map((spec) => importLine(bySpec.get(spec), spec));
    const sepEnd = index === 0 ? tokens[found.open].end : tokens[props[index - 1].comma].end;
    const between = src.slice(sepEnd, lineStart(src, p.start) > sepEnd ? lineStart(src, p.start) : sepEnd);
    const leadingStart = index === 0 ? 0 : between.indexOf('\n') + 1;
    const leading = between.slice(leadingStart).replace(/^(?:[ \t]*\n)+/, '');
    const indent = lineStart(src, p.start) > sepEnd && /^[ \t]*$/.test(src.slice(lineStart(src, p.start), p.start)) ? src.slice(lineStart(src, p.start), p.start) : '  ';
    const valueEnd = p.comma >= 0 ? tokens[p.comma].start : tokens[found.close].start;
    const body = src.slice(p.start, p.end);
    const parts = [src.slice(p.end, valueEnd).trim()];
    const nextSep = p.comma >= 0 ? tokens[p.comma].end : null;
    if (nextSep !== null) {
      const lineEnd = src.indexOf('\n', nextSep);
      parts.push(src.slice(nextSep, lineEnd < 0 ? src.length : lineEnd).trim());
    }
    const trailing = parts.filter(Boolean).map((part) => ` ${part}`).join('');
    let tail = '';
    if (index === props.length - 1 && nextSep !== null) {
      const closeLine = lineStart(src, tokens[found.close].start);
      const lineEnd = src.indexOf('\n', nextSep);
      if (lineEnd >= 0 && lineEnd + 1 < closeLine) tail = src.slice(lineEnd + 1, closeLine).replace(/^(?:[ \t]*\n)+/, '');
    }
    const head = lines.length ? `${lines.join('\n')}\n\n` : '';
    const text = `${head}export const ${kind} = {\n${leading}${indent}${body},${trailing}\n${tail}};\n`;
    files.set(posix.join(HANDLERS, `${p.key}.js`), text);
    ids.push(p.key);
  });
  const lib = libOf(rest, top, path);
  if (lib !== null) files.set(posix.join(LIB, posix.basename(path)), lib);
  return { files, ids };
}

/** The legacy file without its literal, every declaration exported and its imports moved: null when nothing is left. */
function libOf(rest, top, path) {
  if (!top.declared.size && !top.imports.length) return null;
  const edits = [];
  for (const imp of top.imports) {
    const moved = movedSpec(imp.spec, LIB);
    if (moved !== imp.spec) {
      const text = rest.slice(imp.start, imp.end);
      edits.push({ at: imp.start, drop: imp.end - imp.start, put: text.replace(/(['"])[^'"]*\1(\s*;?)$/, `'${moved}'$2`) });
    }
  }
  const starts = new Set();
  for (const d of top.declared.values()) {
    if (!d.exported && !starts.has(d.start)) { starts.add(d.start); edits.push({ at: d.start, drop: 0, put: 'export ' }); }
  }
  edits.sort((a, b) => b.at - a.at);
  let text = rest;
  for (const e of edits) text = text.slice(0, e.at) + e.put + text.slice(e.at + e.drop);
  return `// The helpers of ${path}, split by scripts/split-handlers.mjs: shared by the handler files that import them.\n${text.replace(/\n+$/, '')}\n`;
}

/** Every `.js` file under `dir` (relative to `root`), as repository paths. */
function jsFiles(root, dir) {
  const out = [];
  const walk = (rel) => {
    let names;
    try { names = fs.readdirSync(join(root, rel)).sort(); } catch (e) { return; }
    for (const name of names) {
      const path = posix.join(rel, name);
      if (fs.statSync(join(root, path)).isDirectory()) walk(path); else if (name.endsWith('.js')) out.push(path);
    }
  };
  walk(dir);
  return out;
}

/** The split of the tree at `root`, in memory: `{ files, ids, legacy }`; refuses what it cannot split. */
export function plan(root) {
  const present = LEGACY.filter((l) => fs.existsSync(join(root, l.path)));
  if (!present.length) return { files: new Map(), ids: { reads: [], commands: [] }, legacy: [] };
  for (const path of jsFiles(root, 'mock/src')) {
    if (LEGACY.some((l) => l.path === path)) continue;
    const text = fs.readFileSync(join(root, path), 'utf8');
    if (/\bfrom\s+['"]\.\/(reads|commands)\.js['"]/.test(text)) throw new Refusal(`${path} still imports a legacy handler file: install the harness's mock/src/server.js first (harness init-repo), then split`);
  }
  const files = new Map();
  const ids = { reads: [], commands: [] };
  for (const legacy of present) {
    const one = splitOne(fs.readFileSync(join(root, legacy.path), 'utf8'), legacy);
    for (const [path, text] of one.files) {
      if (files.has(path)) throw new Refusal(`${path} would be written twice: ${posix.basename(path, '.js')} is both a read and a command`);
      files.set(path, text);
    }
    ids[legacy.kind] = one.ids;
  }
  for (const [path, text] of files) {
    const target = join(root, path);
    if (fs.existsSync(target) && fs.readFileSync(target, 'utf8') !== text) throw new Refusal(`${path} is already there, with other content: merge it with the legacy handler by hand, then split again`);
  }
  return { files, ids, legacy: present.map((l) => l.path) };
}

const DRIVER = `
import fs from 'node:fs/promises';
const [root, idsJson] = process.argv.slice(2);
const ids = JSON.parse(idsJson);
globalThis.window = globalThis;
globalThis.location = { href: new URL('mock/pages/', root).href };
globalThis.fetch = async (url) => {
  try { const text = await fs.readFile(new URL(url), 'utf8'); return { ok: true, json: async () => JSON.parse(text) }; }
  catch { return { ok: false }; }
};
const later = globalThis.setTimeout;
globalThis.setTimeout = (fn, _ms, ...rest) => later(fn, 0, ...rest);
let seed = 1;
Math.random = () => { seed = (seed * 16807) % 2147483647; return (seed - 1) / 2147483646; };
let uuid = 0;
Object.defineProperty(globalThis, 'crypto', { value: { randomUUID: () => 'split-' + (uuid += 1) }, configurable: true });
console.error = () => {};
process.on('unhandledRejection', () => {});
const { mockServer } = await import(new URL('mock/src/server.js', root).href);
const s = mockServer();
const out = {};
const said = async (path, body) => {
  try { const r = await s.fetch('http://mock' + path, { method: 'POST', body: JSON.stringify(body) }); return { status: r.status, body: await r.json() }; }
  catch (e) { return { threw: String(e && e.message ? e.message : e) }; }
};
for (const kind of ['reads', 'commands']) {
  for (const id of ids[kind]) {
    const handler = await s.handlerOf(kind, id).catch((e) => e);
    out[kind + ' ' + id + ' source'] = typeof handler === 'function' ? handler.toString() : String(handler);
  }
}
for (const id of ids.reads) out['read ' + id] = await said('/capabilities/' + encodeURIComponent(id), { variables: {} });
for (const id of ids.commands) {
  const r = await said('/commands', { dataCapability: id, payload: {}, target: { id: 'split-check' }, caller: { by: 'split' } });
  out['command ' + id] = r.body && r.body.result ? { status: r.status, result: r.body.result.status, violations: r.body.result.violations } : r;
}
process.stdout.write(JSON.stringify(out));
`;

/** Runs the mock server of the tree at `root` on every id: what it answered, by id. */
function answersOf(root, ids, scratch) {
  const driver = join(scratch, 'driver.mjs');
  fs.writeFileSync(driver, DRIVER);
  const run = spawnSync(process.execPath, [driver, `${pathToFileURL(root).href}/`, JSON.stringify(ids)], { encoding: 'utf8', timeout: 120000, maxBuffer: 64 * 1024 * 1024 });
  if (run.status !== 0) throw new Refusal(`the self-check could not run the mock server in ${root}: ${(run.stderr || run.error || '').toString().trim().split('\n').slice(0, 6).join(' | ')}`);
  return JSON.parse(run.stdout);
}

/**
 * A scratch copy of the mock-up of `root` the self-check runs in: mock/src
 * and mock/handlers copied, the fixtures and the plan linked. The legacy
 * copy leaves out the planned files already there (a merge may have brought
 * them beside the legacy files): it holds the legacy layout only.
 */
function scratchTree(root, at, files, withoutLegacy) {
  fs.mkdirSync(join(at, 'mock'), { recursive: true });
  fs.cpSync(join(root, 'mock/src'), join(at, 'mock/src'), { recursive: true });
  if (fs.existsSync(join(root, HANDLERS))) {
    const planned = new Set([...files.keys()].map((path) => resolve(root, path)));
    fs.cpSync(join(root, HANDLERS), join(at, HANDLERS), { recursive: true, filter: (from) => withoutLegacy || !planned.has(resolve(from)) });
  }
  for (const linked of ['mock/fixtures', 'plan']) {
    if (fs.existsSync(join(root, linked))) fs.symlinkSync(resolve(root, linked), join(at, linked), 'dir');
  }
  if (withoutLegacy) {
    for (const l of LEGACY) if (fs.existsSync(join(at, l.path))) fs.unlinkSync(join(at, l.path));
    for (const [path, text] of files) { fs.mkdirSync(dirname(join(at, path)), { recursive: true }); fs.writeFileSync(join(at, path), text); }
  }
}

/**
 * The self-check: the mock server answers the same on the legacy tree and on
 * the split one — the same handlers, each with its source unchanged, and the
 * same answers on the fixtures. Refuses on the first difference.
 */
export function selfCheck(root, planned) {
  const scratch = fs.mkdtempSync(join(tmpdir(), 'split-handlers-'));
  try {
    scratchTree(root, join(scratch, 'legacy'), planned.files, false);
    scratchTree(root, join(scratch, 'split'), planned.files, true);
    const before = answersOf(join(scratch, 'legacy'), planned.ids, scratch);
    const after = answersOf(join(scratch, 'split'), planned.ids, scratch);
    const differ = Object.keys(before).concat(Object.keys(after)).filter((key, n, all) => all.indexOf(key) === n && JSON.stringify(before[key]) !== JSON.stringify(after[key]));
    if (differ.length) {
      const first = differ[0];
      throw new Refusal(`the split mock server answers otherwise than the legacy one for ${differ.slice(0, 5).join(', ')}${differ.length > 5 ? ', …' : ''} — ${first}: ${JSON.stringify(before[first]).slice(0, 300)} became ${JSON.stringify(after[first]).slice(0, 300)}`);
    }
    return Object.keys(before).length;
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
}

/** Splits the tree at `root`: what it did, in one line; a Refusal when it cannot. */
export function split(root) {
  const planned = plan(root);
  if (!planned.legacy.length) return 'nothing to split: mock/src/reads.js and mock/src/commands.js are not there';
  const checked = selfCheck(root, planned);
  for (const [path, text] of planned.files) {
    fs.mkdirSync(dirname(join(root, path)), { recursive: true });
    fs.writeFileSync(join(root, path), text);
  }
  for (const path of planned.legacy) fs.unlinkSync(join(root, path));
  return `split ${planned.ids.reads.length} read(s) and ${planned.ids.commands.length} command(s) into ${planned.files.size} file(s) under ${HANDLERS}/, ${checked} answers checked; removed ${planned.legacy.join(' and ')} — now run sh mock/build.sh and commit`;
}

function main() {
  if (!nodeIsRecent(process.versions.node)) {
    process.stderr.write(`split-handlers: node ${MIN_NODE} or later is needed, this is ${process.version} — the legacy files are ES modules without a package.json type\n`);
    process.exit(1);
  }
  try {
    process.stdout.write(`split-handlers: ${split(process.cwd())}\n`);
  } catch (e) {
    if (!(e instanceof Refusal)) throw e;
    process.stderr.write(`split-handlers: refused, nothing written — ${e.message}\n`);
    process.exit(1);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main();
