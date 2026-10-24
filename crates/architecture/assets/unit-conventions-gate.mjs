// The gates `data-capability-conventions` and `capability-conventions` of
// .github/workflows/gates.yml: the conventions a unit drifts from when it
// copies a neighbour, judged statically on the units a pull request adds or
// changes — never on the units it leaves alone, which predate the gate.
//
//   node scripts/unit-conventions-gate.mjs data-capabilities <base sha>
//   node scripts/unit-conventions-gate.mjs capabilities <base sha>
//
// A unit is added when the change adds its manifest, changed when it
// touches any file of its folder (a Capability: or its persisted query).
// Each rule says on which it is judged: the rules every existing unit
// already keeps judge both; the rules some older units break judge the
// added ones only, so editing an old unit never turns red for its past.
//
// DataCapability (crates/<system>/data-capabilities/<name>/):
//   clock (changed)        no SystemTime, Instant::now, chrono, Utc::now,
//                          Local::now in src/ outside tests: time comes
//                          from the engine (context.now_millis, FixedClock)
//   id-constant (changed)  `pub const DATA_CAPABILITY: &str = "<id>";` in
//                          src/lib.rs, the manifest's id
//   ports (changed)        the manifest's `ports` are exactly the ports-*
//                          crates of [dependencies]
//   manifest-planted (added) a test plants the real manifest,
//                          include_str!("../data-capability.json")
//   sql-tested (added)     src/ that reads SQL (`.rows(`) runs it on
//                          PostgreSQL in a test, through the test kit's
//                          Scratch or the guard, named by path
//                          (platform_test_kit::…Scratch,
//                          platform_scratch_db::, ScratchDb::create) in
//                          code, never a comment: over memory
//                          `context.rows` is None, a broken query passes
//   legacy (added)         no `pub fn names(` nor `pub const MANIFEST`
// Capability (crates/<system>/capabilities/<name>/), hostless ones aside:
//   query-file-named (added) reads is ["queries/<folder>.graphql"]
//   no-rust-refilter (added) answer() reads no variable the SQL binds from
//                          its variables argument (`<param>.get("x")`,
//                          `<param>["x"]`; a `_`-named one reads none): the
//                          filter is the SQL's, once. An output key that
//                          shares a variable's name is no read
//   variables-bound (added) VARIABLES is empty only when input is {}
//   one-select (changed)   SQL starts with SELECT or WITH, holds no `;`
//   binds (changed)        SQL binds exactly $1..$n for the n VARIABLES,
//                          and the query file's `$var:` are VARIABLES, in
//                          order — the registry binds them by position
//   parity (changed)       every field the query file selects is a word
//                          of the SQL, camelCase as snake_case
//   text-compare (changed) each $n is compared as text,
//                          `<column>::text = $n` (or `= lower($n)`)
//
// Both kinds:
//   holes (changed)        no `SCAFFOLD:` left in a file of the unit's
//                          folder (a Capability: or its query file) — the
//                          skeleton's holes, every one filled before merge
//
// Reads the files as text, runs none of them. Every finding is reported
// (`::error file=…::<rule>: …`), the verdict comes last. Installed by
// `harness init-repo`; the repository owns it from here.

import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import { posix } from 'node:path';

const [kind, base] = process.argv.slice(2);
const KINDS = {
  'data-capabilities': 'data-capability.json',
  capabilities: 'capability.json',
};
if (!KINDS[kind]) {
  process.stdout.write(`usage: node scripts/unit-conventions-gate.mjs data-capabilities|capabilities <base sha>\n`);
  process.exit(2);
}

let failed = 0;
const error = (file, rule, message) => {
  failed += 1;
  process.stdout.write(`::error file=${file}::${rule}: ${message}\n`);
};

const git = (...args) => execFileSync('git', args, { encoding: 'utf8' });
const reachable = (sha) => {
  try {
    git('cat-file', '-e', sha);
    return true;
  } catch {
    return false;
  }
};
if (!base || !reachable(base)) {
  process.stdout.write('no base to compare with: nothing to judge\n');
  process.exit(0);
}
const diff = (filter) => git('diff', '--name-only', `--diff-filter=${filter}`, `${base}...HEAD`)
  .split('\n')
  .filter(Boolean);

const read = (path) => (fs.existsSync(path) ? fs.readFileSync(path, 'utf8') : null);
const json = (path) => {
  try {
    return JSON.parse(read(path));
  } catch {
    return null;
  }
};

/** The `.rs` files under `dir`, recursively, as paths from the root. */
function rustFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const path = posix.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...rustFiles(path));
    else if (entry.name.endsWith('.rs')) out.push(path);
  }
  return out.sort();
}

/** A Rust file's text before its first `#[cfg(test)]`: the shipped code. */
const shipped = (text) => {
  const at = text.indexOf('#[cfg(test)]');
  return at < 0 ? text : text.slice(0, at);
};

/** Rust text with its `//` comments blanked, line numbers kept. */
const uncommented = (text) => (text || '').replace(/(^|[ \t])\/\/.*$/gm, '$1');

/** A test reaching PostgreSQL through the kit's Scratch or the guard, named by path. */
const SCRATCH = /platform_test_kit::(?:\{[^}]*\b)?Scratch\b|platform_scratch_db::|ScratchDb::create/;

/** `camelCase` as `camel_case`. */
const snake = (name) => name.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();

/** The unit folders of `kind` the paths fall in, as `crates/<system>/<kind>/<name>`. */
function unitsOf(paths) {
  const pattern = new RegExp(`^(crates/[^/]+/${kind}/[^/]+)/`);
  const dirs = new Set();
  for (const path of paths) {
    const found = pattern.exec(path);
    if (found) dirs.add(found[1]);
  }
  return dirs;
}

const added = new Set(diff('A'));
const changed = diff('AMR');
const addedUnits = new Set([...unitsOf(added)].filter((dir) => added.has(`${dir}/${KINDS[kind]}`)));
const touched = unitsOf(changed);
if (kind === 'capabilities') {
  // A persisted query changed is its Capability changed.
  const queries = changed.filter((path) => /^crates\/[^/]+\/queries\/[^/]+$/.test(path));
  for (const query of queries) {
    const system = query.split('/')[1];
    const root = `crates/${system}/capabilities`;
    if (!fs.existsSync(root)) continue;
    for (const name of fs.readdirSync(root)) {
      const manifest = json(`${root}/${name}/capability.json`);
      const reads = Array.isArray(manifest?.reads) ? manifest.reads : [];
      if (reads.some((r) => typeof r === 'string' && query.endsWith(r.replace(/^\.?\//, '')))) {
        touched.add(`${root}/${name}`);
      }
    }
  }
}
const judged = [...new Set([...addedUnits, ...touched])]
  .filter((dir) => fs.existsSync(`${dir}/${KINDS[kind]}`))
  .sort();
if (judged.length === 0) {
  process.stdout.write(`no ${kind} added or changed: nothing to judge\n`);
  process.exit(0);
}

for (const dir of judged) {
  const isAdded = addedUnits.has(dir);
  holes(dir);
  if (kind === 'data-capabilities') dataCapability(dir, isAdded);
  else capability(dir, isAdded);
}

process.stdout.write(`${judged.length} unit(s) judged (${judged.join(', ')}): ${failed === 0 ? 'every convention holds' : `${failed} finding(s)`}\n`);
process.exit(failed === 0 ? 0 : 1);

/** Every file under `dir`, recursively, build output aside. */
function filesUnder(dir) {
  if (!fs.existsSync(dir)) return [];
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const path = posix.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== 'target' && entry.name !== 'node_modules') out.push(...filesUnder(path));
    } else if (entry.isFile()) out.push(path);
  }
  return out.sort();
}

/** A skeleton's hole left in the unit's folder, or in a Capability's query file. */
function holes(dir) {
  const files = filesUnder(dir);
  if (kind === 'capabilities') {
    const system = dir.split('/')[1];
    const manifest = json(`${dir}/capability.json`);
    const reads = Array.isArray(manifest?.reads) ? manifest.reads : [];
    const queries = [`queries/${dir.split('/').pop()}.graphql`, ...reads.filter((r) => typeof r === 'string')]
      .map((r) => `crates/${system}/${r.replace(/^\.?\//, '')}`)
      .filter((path) => fs.existsSync(path) && !files.includes(path));
    files.push(...new Set(queries));
  }
  for (const path of files) {
    const lines = (read(path) || '').split('\n');
    const at = lines.flatMap((line, n) => (line.includes('SCAFFOLD:') ? [n + 1] : []));
    if (at.length > 0) {
      error(path, 'holes', `line(s) ${at.join(', ')} leave a \`SCAFFOLD:\` hole of the skeleton: fill each one, never merge it`);
    }
  }
}

function dataCapability(dir, isAdded) {
  const manifestPath = `${dir}/data-capability.json`;
  const manifest = json(manifestPath);
  if (!manifest) {
    error(manifestPath, 'manifest', 'is not JSON');
    return;
  }
  const src = rustFiles(`${dir}/src`).map((path) => ({ path, code: shipped(read(path)) }));
  const tests = rustFiles(`${dir}/tests`).map((path) => uncommented(read(path))).join('\n');
  for (const { path, code } of src) {
    code.split('\n').forEach((line, n) => {
      if (/SystemTime|Instant::now|chrono::|Utc::now|Local::now/.test(line)) {
        error(path, 'clock', `line ${n + 1} reads a clock — time comes from the engine, context.now_millis (FixedClock in tests)`);
      }
    });
  }
  const lib = read(`${dir}/src/lib.rs`) || '';
  const id = manifest.dataCapability;
  if (!lib.includes(`pub const DATA_CAPABILITY: &str = "${id}";`)) {
    error(`${dir}/src/lib.rs`, 'id-constant', `declares no \`pub const DATA_CAPABILITY: &str = "${id}";\`, the manifest's id its Prepare is registered under`);
  }
  const declared = (Array.isArray(manifest.ports) ? manifest.ports : []).map(String).sort();
  const cargo = read(`${dir}/Cargo.toml`) || '';
  const dependencies = (/^\[dependencies\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(cargo) || [, ''])[1];
  const crates = [...dependencies.matchAll(/^(ports-[a-z0-9-]+)\s*=/gm)].map((m) => m[1].replace(/^ports-/, '')).sort();
  if (JSON.stringify(declared) !== JSON.stringify(crates)) {
    error(`${dir}/Cargo.toml`, 'ports', `the manifest's ports [${declared.join(', ')}] are not the ports-* crates of [dependencies] [${crates.join(', ')}]`);
  }
  if (!isAdded) return;
  if (!/include_str!\(\s*"(?:\.\.\/)+data-capability\.json"\s*\)/.test(tests)) {
    error(`${dir}/tests`, 'manifest-planted', 'no test plants the real manifest, include_str!("../data-capability.json") beside its aggregate (platform_test_kit::Bench)');
  }
  if (src.some(({ code }) => code.includes('.rows(')) && !SCRATCH.test(tests)) {
    error(`${dir}/tests`, 'sql-tested', 'src/ reads SQL (`.rows(`) and no test runs it on PostgreSQL (platform_test_kit::Scratch): over memory `context.rows` is None, a broken query passes green');
  }
  for (const { path, code } of src) {
    if (/pub fn names\(|pub const MANIFEST\b/.test(code)) {
      error(path, 'legacy', 'holds a pre-plugin item, `pub fn names(` or `pub const MANIFEST`: dead code a neighbour carried, never written in a new unit');
    }
  }
}

/** The raw text of `pub const SQL`, or null. */
function sqlOf(code) {
  const raw = /pub const SQL: &str =\s*r(#*)"([\s\S]*?)"\1\s*;/.exec(code);
  if (raw) return raw[2];
  const plain = /pub const SQL: &str =\s*"((?:[^"\\]|\\.)*)"\s*;/.exec(code);
  return plain ? plain[1] : null;
}

/** `pub const VARIABLES`'s names, or null. */
function variablesOf(code) {
  const found = /pub const VARIABLES: &\[&str\] =\s*&\[([\s\S]*?)\]\s*;/.exec(code);
  return found ? [...found[1].matchAll(/"([^"]*)"/g)].map((m) => m[1]) : null;
}

/** The body of `pub fn answer(`, braces included, or null. */
function answerBody(code) {
  const at = code.indexOf('pub fn answer(');
  if (at < 0) return null;
  const open = code.indexOf('{', code.indexOf(')', at));
  if (open < 0) return null;
  let depth = 0;
  for (let i = open; i < code.length; i += 1) {
    const c = code[i];
    if (c === '"' || (c === 'r' && /^r#*"/.test(code.slice(i, i + 8)) && !/\w/.test(code[i - 1] || ''))) {
      const hashes = c === 'r' ? /^r(#*)"/.exec(code.slice(i))[1] : null;
      const start = c === 'r' ? i + hashes.length + 2 : i + 1;
      if (hashes !== null) {
        const end = code.indexOf(`"${hashes}`, start);
        i = end < 0 ? code.length : end + hashes.length;
      } else {
        let end = start;
        while (end < code.length && code[end] !== '"') end += code[end] === '\\' ? 2 : 1;
        i = end;
      }
      continue;
    }
    if (c === '\'' && (code[i + 2] === '\'' || (code[i + 1] === '\\' && code[i + 3] === '\''))) {
      i += code[i + 1] === '\\' ? 3 : 2;
      continue;
    }
    if (c === '{') depth += 1;
    if (c === '}') {
      depth -= 1;
      if (depth === 0) return code.slice(open, i + 1);
    }
  }
  return code.slice(open);
}

/**
 * The request variables `pub fn answer(` reads from its variables argument
 * (its second parameter): `<param>.get("x")`, `<param>["x"]`. None when that
 * parameter is `_`-named, which reads nothing; an output key is no read.
 */
function variableReads(code) {
  const signature = /pub fn answer\s*\(\s*(?:mut\s+)?\w+\s*:[^,]*,\s*(?:mut\s+)?(\w+)\s*:/.exec(code);
  if (!signature || signature[1].startsWith('_')) return [];
  const body = uncommented(answerBody(code));
  const param = signature[1];
  const access = new RegExp(`\\b${param}\\s*(?:\\.\\s*get\\s*\\(\\s*&?\\s*"([^"]*)"|\\[\\s*"([^"]*)"\\s*\\])`, 'g');
  return [...body.matchAll(access)].map((m) => m[1] ?? m[2]);
}

function capability(dir, isAdded) {
  const manifestPath = `${dir}/capability.json`;
  const manifest = json(manifestPath);
  if (!manifest) {
    error(manifestPath, 'manifest', 'is not JSON');
    return;
  }
  if (manifest.host === 'hostless') return;
  const folder = dir.split('/').pop();
  const system = dir.split('/')[1];
  const libPath = `${dir}/src/lib.rs`;
  const code = shipped(read(libPath) || '');
  const sql = sqlOf(code);
  const variables = variablesOf(code);
  if (sql === null || variables === null) {
    error(libPath, 'one-select', 'declares no `pub const SQL: &str` and `pub const VARIABLES: &[&str]` the gate can read');
    return;
  }
  const reads = Array.isArray(manifest.reads) ? manifest.reads : [];
  const queryPath = reads
    .filter((r) => typeof r === 'string')
    .flatMap((r) => [r, `${dir}/${r}`, `crates/${system}/${r}`])
    .find((p) => fs.existsSync(p));
  if (isAdded) {
    const wanted = `queries/${folder}.graphql`;
    if (JSON.stringify(reads) !== JSON.stringify([wanted])) {
      error(manifestPath, 'query-file-named', `reads ${JSON.stringify(reads)}: a Capability reads one query file named after its folder, ["${wanted}"]`);
    }
    const readVariables = variableReads(code);
    for (const variable of variables) {
      if (readVariables.includes(variable)) {
        error(libPath, 'no-rust-refilter', `answer() reads "${variable}", a variable the SQL binds: the filter is the SQL's, once — answer() maps the rows it already filtered`);
      }
    }
    const inputs = manifest.input && typeof manifest.input === 'object' ? Object.keys(manifest.input) : [];
    if (variables.length === 0 && inputs.length > 0) {
      error(libPath, 'variables-bound', `VARIABLES is empty while the input is {${inputs.join(', ')}}: a filtering input is bound in the SQL, never applied in Rust over every row`);
    }
  }
  const upper = sql.trim().toUpperCase();
  if (!(upper.startsWith('SELECT') || upper.startsWith('WITH')) || sql.includes(';')) {
    error(libPath, 'one-select', 'SQL is not one read statement: it starts with SELECT or WITH and holds no `;`');
  }
  const bound = [...new Set([...sql.matchAll(/\$(\d+)/g)].map((m) => Number(m[1])))].sort((a, b) => a - b);
  const expected = variables.map((_, n) => n + 1);
  if (JSON.stringify(bound) !== JSON.stringify(expected)) {
    error(libPath, 'binds', `SQL binds [${bound.map((n) => `$${n}`).join(', ')}] for VARIABLES [${variables.join(', ')}]: exactly $1..$${variables.length}, in their order`);
  }
  for (const n of expected) {
    if (!new RegExp(`::text\\s*=\\s*(?:lower\\(\\s*)?\\$${n}(?!\\d)`).test(sql)) {
      error(libPath, 'text-compare', `$${n} (${variables[n - 1]}) is not compared as text, \`<column>::text = $${n}\`: an id that is no uuid then fails instead of answering empty`);
    }
  }
  if (!queryPath || !queryPath.endsWith('.graphql')) return;
  const query = read(queryPath);
  const lines = query.split('\n');
  const start = lines.findIndex((line) => /^\s*query\b/.test(line));
  if (start < 0) return;
  const declared = [...lines[start].matchAll(/\$(\w+)\s*:/g)].map((m) => m[1]);
  if (JSON.stringify(declared) !== JSON.stringify(variables)) {
    error(queryPath, 'binds', `the query's variables [${declared.join(', ')}] are not VARIABLES [${variables.join(', ')}], in order`);
  }
  const words = new Set(sql.split(/[^A-Za-z0-9_]+/).filter(Boolean));
  for (const line of lines.slice(start + 1)) {
    const field = line.trim();
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(field)) continue;
    if (!words.has(snake(field))) {
      error(queryPath, 'parity', `selects \`${field}\` and the SQL has no \`${snake(field)}\`: the query file declares what the SQL really reads`);
    }
  }
}

