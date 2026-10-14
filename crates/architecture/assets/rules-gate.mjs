#!/usr/bin/env node
// The rules' gate (contract P): what `rules.json` says the product keeps,
// checked against the tests that prove it. Installed by `harness init-repo`;
// the repository owns it from here. No dependency: node alone.
//
//   node scripts/rules-gate.mjs coverage
//       Every rule has at least one test of its family — business under
//       tests/rules/business/tests/*.rs, journey under
//       tests/rules/journey/**/*.spec.ts — naming it (`rule("R3"` or
//       `rule('R3'`), and every test that names it copies its criterion as
//       it is: the three lines `given: …`, `when: …`, `then: …`. A test
//       naming a rule rules.json has not fails too.
//
//   node scripts/rules-gate.mjs status [--strict] [<status.jsonl>…]
//       Reads what the rules' tests wrote (RULES_STATUS, by default
//       target/rules-status.jsonl) and says each rule's state: pending (a
//       unit it needs is not built), green, red. Writes rules-status.json.
//       Fails on a red rule — but a rule marked `conform` (it changed, its
//       tests ahead of the code) only with --strict: where a milestone lands.
//
//   node scripts/rules-gate.mjs untouched <base>
//       A change measured against <base> leaves tests/rules/ and rules.json
//       alone: only the rules' task writes them.
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const root = process.cwd();
const RULES = join(root, 'rules.json');
const BUSINESS = join(root, 'tests/rules/business/tests');
const JOURNEY = join(root, 'tests/rules/journey');

const say = (line) => process.stdout.write(line + '\n');
const fail = (line) => { process.stdout.write(`::error::${line}\n`); };
const squash = (text) => String(text).replace(/\s+/g, ' ').trim();

function rules() {
  if (!existsSync(RULES)) return null;
  const read = JSON.parse(readFileSync(RULES, 'utf8'));
  return Array.isArray(read.rules) ? read.rules : [];
}

function files(dir, keep, out = []) {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name === 'target') continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) files(path, keep, out);
    else if (keep(name)) out.push(path);
  }
  return out;
}

// The rules a test file names, by their id.
function named(text) {
  return [...text.matchAll(/\brule\(\s*["'](R[1-9][0-9]*)["']/g)].map((m) => m[1]);
}

function coverage() {
  const all = rules();
  if (all === null) { say('no rules.json yet: nothing to cover'); return 0; }
  const tests = [
    ...files(BUSINESS, (n) => n.endsWith('.rs')).map((path) => ({ path, family: 'business' })),
    ...files(JOURNEY, (n) => n.endsWith('.spec.ts')).map((path) => ({ path, family: 'journey' })),
  ].map((t) => ({ ...t, text: readFileSync(t.path, 'utf8') }));
  let failed = 0;
  const known = new Set(all.map((r) => r.id));
  for (const t of tests) {
    for (const id of named(t.text)) {
      if (!known.has(id)) { fail(`${t.path}: names ${id}, which rules.json has not`); failed += 1; }
    }
  }
  for (const r of all) {
    const own = tests.filter((t) => named(t.text).includes(r.id));
    const right = own.filter((t) => t.family === r.family);
    if (!right.length) {
      fail(`${r.id} (${r.family}) has no test${own.length ? ` of its family: ${own.map((t) => t.path).join(', ')} is not` : ''}`);
      failed += 1;
      continue;
    }
    for (const t of own) {
      const text = squash(t.text);
      for (const part of ['given', 'when', 'then']) {
        const line = squash(`${part}: ${r.criterion?.[part] ?? ''}`);
        if (!text.includes(line)) { fail(`${t.path}: ${r.id}'s criterion is not copied as it is — "${line}"`); failed += 1; }
      }
    }
    say(`${r.id} covered by ${right.map((t) => t.path.slice(root.length + 1)).join(', ')}`);
  }
  return failed ? 1 : 0;
}

function status(args) {
  const strict = args.includes('--strict');
  const paths = args.filter((a) => a !== '--strict');
  if (!paths.length) paths.push(process.env.RULES_STATUS || join(root, 'target/rules-status.jsonl'));
  const outcomes = paths.filter(existsSync).flatMap((p) => readFileSync(p, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)));
  const all = rules() || [];
  const told = all.map((r) => {
    const own = outcomes.filter((o) => o.rule === r.id);
    const state = !own.length ? 'untested' : own.some((o) => o.state === 'red') ? 'red' : own.some((o) => o.state === 'green') ? 'green' : 'pending';
    return { id: r.id, title: r.title, family: r.family, conform: r.state === 'conform', state, tests: own.map((o) => ({ test: o.test, state: o.state, why: o.why || '' })) };
  });
  writeFileSync(join(root, 'rules-status.json'), JSON.stringify({ at: new Date().toISOString(), rules: told }, null, 2) + '\n');
  let failed = 0;
  for (const r of told) {
    const why = r.tests.map((t) => t.why).filter(Boolean)[0] || '';
    say(`${r.id} ${r.state}${r.conform ? ' (to conform)' : ''}${why ? ` — ${why}` : ''}`);
    if (r.state === 'red' && (!r.conform || strict)) { fail(`${r.id} does not hold${why ? `: ${why}` : ''}`); failed += 1; }
  }
  return failed ? 1 : 0;
}

function untouched(base) {
  if (!base) { fail('untouched needs the base the change is measured against'); return 1; }
  const changed = execFileSync('git', ['diff', '--name-only', `${base}...HEAD`], { encoding: 'utf8' }).split('\n').filter(Boolean);
  const touched = changed.filter((p) => p === 'rules.json' || p.startsWith('tests/rules/'));
  for (const p of touched) fail(`${p}: only the rules' task writes the rules and their tests — contest a rule's test instead`);
  return touched.length ? 1 : 0;
}

const [mode, ...rest] = process.argv.slice(2);
const run = { coverage: () => coverage(), status: () => status(rest), untouched: () => untouched(rest[0]) }[mode];
if (!run) { say('usage: rules-gate.mjs coverage | status [--strict] [<status.jsonl>…] | untouched <base>'); process.exit(2); }
process.exit(run());
