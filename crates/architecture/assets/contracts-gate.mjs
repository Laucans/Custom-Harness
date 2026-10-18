#!/usr/bin/env node
// The contracts' gate (contract R): a milestone changes the interfaces it
// produces, and no other's. Installed by `harness init-repo`; the repository
// owns it from here. No dependency: node and git alone.
//
//   node scripts/contracts-gate.mjs untouched <base>
//       A change measured against <base> (`origin/<branch>` as the CI has
//       it) touches a `capability.json` or a `data-capability.json` only
//       when the plan's graph (`plan/graph.json`) says the milestone the
//       change lands in produces that id — read off the branch the pull
//       request targets (`milestone/<n>-…`, in GITHUB_BASE_REF), or off
//       `--milestone <n>`. An id the graph does not know is this change's
//       own: new, and nobody else's. Without a graph, nothing is judged.
//
//       A consumer that needs a contract to change does not change it: it
//       stops with `AGENT_LOOP_STOP: contract <id> — <what it needs>`, and
//       the contract manager evolves the contract for every consumer at
//       once. That is what keeps two milestones building side by side
//       against the same interface.

import { readFileSync, existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const args = process.argv.slice(2);
const mode = args[0];

function fail(file, message) {
  console.log(`::error file=${file}::${message}`);
}

function graph() {
  if (!existsSync('plan/graph.json')) return null;
  try {
    return JSON.parse(readFileSync('plan/graph.json', 'utf8'));
  } catch {
    return null;
  }
}

function milestoneOf(args) {
  const at = args.indexOf('--milestone');
  if (at >= 0 && args[at + 1]) return Number(args[at + 1]);
  const base = process.env.GITHUB_BASE_REF || '';
  const m = /^milestone\/(\d+)(-|$)/.exec(base);
  return m ? Number(m[1]) : null;
}

function idOf(file) {
  try {
    const json = JSON.parse(readFileSync(file, 'utf8'));
    return json.capability || json.dataCapability || null;
  } catch {
    return null;
  }
}

function untouched(base) {
  const g = graph();
  if (!g) {
    console.log('no plan/graph.json: nothing to judge');
    return 0;
  }
  const milestone = milestoneOf(args);
  const changed = execFileSync('git', ['diff', '--name-only', `${base}...HEAD`], { encoding: 'utf8' })
    .split('\n')
    .filter((f) => /(^|\/)(capability|data-capability)\.json$/.test(f));
  let failed = 0;
  for (const file of changed) {
    const id = existsSync(file) ? idOf(file) : null;
    if (!id) continue;
    const entry = g.ids[id];
    if (!entry || entry.producer == null) continue;
    if (milestone != null && entry.producer === milestone) continue;
    fail(
      file,
      `the contract of ${id} belongs to milestone #${entry.producer}` +
        (milestone == null ? '' : `, not #${milestone}`) +
        ' — a consumer that needs it to change stops with `AGENT_LOOP_STOP: contract ' +
        id +
        ' — <what it needs>`; the contract manager evolves it',
    );
    failed = 1;
  }
  if (!failed) console.log(`contracts untouched: ${changed.length} manifest(s) changed, every one its milestone's own`);
  return failed;
}

if (mode === 'untouched' && args[1]) {
  process.exit(untouched(args[1]));
}
console.error('usage: node scripts/contracts-gate.mjs untouched <base> [--milestone <n>]');
process.exit(2);
