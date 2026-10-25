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
//       unit it needs is not built), green, red, or mis-mounted. Writes
//       rules-status.json. Fails on a red rule — but a rule marked `conform`
//       (it changed, its tests ahead of the code) only with --strict: where
//       a milestone lands.
//       Mis-mounted, apart: a test that ran reached the product off its
//       mounting — a unit its needs do not name, a read's variable its
//       capability.json `input` does not take, a payload key its
//       data-capability.json does not declare (the kits check it, once the
//       units are built). Its red or green proves nothing: the test is
//       mounted wrong, not the product. It fails always, `conform` or not,
//       said apart: the rules' task fixes the test before anything is
//       contested, and a development only copies the line into CONTEST.md.
//       The rule's other red tests are listed under it: those are the
//       product's, fixed in the same pass.
//
//   node scripts/rules-gate.mjs untouched <base>
//       A change measured against <base> leaves tests/rules/ and rules.json
//       alone — only the rules' task writes them — and scripts/rules-runtime.sh
//       alone: nobody does.
//
//   node scripts/rules-gate.mjs contest <pr>
//       A development that cannot meet a rule's test writes CONTEST.md at the
//       root instead of changing it. Its pull request opens, once, the issue
//       `Rules contested: PR #<pr>` (harness:rules, harness:ready) the rules'
//       task settles, and fails while CONTEST.md is there: waiting while the
//       issue is open, pointing to the decision once it is closed. Needs `gh`
//       and GH_TOKEN.
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
    const state = !own.length
      ? 'untested'
      : own.some((o) => o.state === 'mis-mounted')
        ? 'mis-mounted'
        : own.some((o) => o.state === 'red')
          ? 'red'
          : own.some((o) => o.state === 'green')
            ? 'green'
            : 'pending';
    return { id: r.id, title: r.title, family: r.family, conform: r.state === 'conform', state, tests: own.map((o) => ({ test: o.test, state: o.state, why: o.why || '' })) };
  });
  writeFileSync(join(root, 'rules-status.json'), JSON.stringify({ at: new Date().toISOString(), rules: told }, null, 2) + '\n');
  let failed = 0;
  const blocks = (r) => !r.conform || strict;
  for (const r of told.filter((r) => r.state !== 'mis-mounted')) {
    const why = r.tests.map((t) => t.why).filter(Boolean)[0] || '';
    say(`${r.id} ${r.state}${r.conform ? ' (to conform)' : ''}${why ? ` — ${why}` : ''}`);
    if (r.state === 'red' && blocks(r)) { fail(`${r.id} does not hold${why ? `: ${why}` : ''}`); failed += 1; }
  }
  const misMounted = told.filter((r) => r.state === 'mis-mounted');
  if (misMounted.length) {
    say('');
    say(`mis-mounted: ${misMounted.length} rule(s) whose test is mounted wrong, not the product — the rules' task fixes the test before anything is contested; a development copies the line into CONTEST.md and changes nothing for it`);
  }
  // A mounting defect is never legitimate: it fails whatever the rule's
  // `conform` flag, so it shows on the first pull request, not at landing.
  for (const r of misMounted) {
    const why = r.tests.filter((t) => t.state === 'mis-mounted').map((t) => t.why).filter(Boolean)[0] || '';
    fail(`${r.id} mis-mounted${r.conform ? ' (to conform)' : ''}${why ? ` — ${why}` : ''}`);
    failed += 1;
    // Its red tests are the product's: a development fixes them in the same pass.
    for (const t of r.tests.filter((t) => t.state === 'red')) say(`  ${r.id} also red: ${t.test}${t.why ? ` — ${t.why}` : ''} (the product's, fix it now)`);
  }
  return failed ? 1 : 0;
}

function untouched(base) {
  if (!base) { fail('untouched needs the base the change is measured against'); return 1; }
  const changed = execFileSync('git', ['diff', '--name-only', `${base}...HEAD`], { encoding: 'utf8' }).split('\n').filter(Boolean);
  const touched = changed.filter((p) => p === 'rules.json' || p.startsWith('tests/rules/'));
  for (const p of touched) fail(`${p}: only the rules' task writes the rules and their tests — contest a rule's test instead`);
  // The runtime is installed whole and changed by nobody: a product it
  // cannot start contests it too.
  const runtime = changed.filter((p) => p === 'scripts/rules-runtime.sh');
  for (const p of runtime) fail(`${p}: the rules' runtime starts the product where the layout puts it — build crates/server and apps/shell there, or contest it`);
  return touched.length + runtime.length ? 1 : 0;
}

function contest(pr) {
  const path = join(root, 'CONTEST.md');
  if (!existsSync(path)) { say('no CONTEST.md: no rule contested'); return 0; }
  if (!/^[1-9][0-9]*$/.test(pr || '')) { fail('contest needs the pull request number'); return 1; }
  const title = `Rules contested: PR #${pr}`;
  const gh = (args) => execFileSync('gh', args, { encoding: 'utf8' });
  const found = JSON.parse(gh(['issue', 'list', '--state', 'all', '--label', 'harness:rules', '--search', `"${title}" in:title`, '--json', 'number,state,title']));
  const mine = found.find((i) => i.title === title);
  if (!mine) {
    const said = readFileSync(path, 'utf8').trim();
    const body = `${said}\n\n— \`CONTEST.md\` of #${pr}, copied by the rules' gate.`;
    const url = gh(['issue', 'create', '--label', 'harness:rules', '--label', 'harness:ready', '--title', title, '--body', body]).trim();
    fail(`a rule's test is contested: the rules' task settles it (${url}); this change waits`);
  } else if (mine.state === 'OPEN') {
    fail(`contested in #${mine.number}: this change waits for the rules' task`);
  } else {
    fail(`the contest was settled in #${mine.number}: read its decision, remove CONTEST.md and conform`);
  }
  return 1;
}

const [mode, ...rest] = process.argv.slice(2);
const run = { coverage: () => coverage(), status: () => status(rest), untouched: () => untouched(rest[0]), contest: () => contest(rest[0]) }[mode];
if (!run) { say('usage: rules-gate.mjs coverage | status [--strict] [<status.jsonl>…] | untouched <base> | contest <pr>'); process.exit(2); }
process.exit(run());
