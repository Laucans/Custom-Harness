// Unit tests of the skill evals' scoring: `node --test scripts/skill-evals.test.mjs`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CHECKS, firstTurn, found, parseStream, plainName, score, simpleCommands, skillsInStatus, slicesOf } from './skill-evals.mjs';

const line = (o) => JSON.stringify(o);
const assistant = (content) => line({ type: 'assistant', message: { content } });
const result = (text, extra = {}) => line({ type: 'result', result: text, total_cost_usd: 0.5, num_turns: 4, ...extra });

test('the first turn of a prompts.md carries its prompt, model and effort', () => {
  const md = '# run\n\n## turn 1 — /tech-analyst (opus/max) ×2\n\n```\n/tech-analyst\n\nbody\n```\n\n## turn 2 — /code\n';
  assert.deepEqual(firstTurn(md), { header: '— /tech-analyst (opus/max) ×2', prompt: '/tech-analyst\n\nbody', model: 'opus', effort: 'max' });
  assert.equal(firstTurn('no turn here'), null);
});

test('a stream keeps every said text, the Bash commands and the denied calls', () => {
  const text = [
    assistant([{ type: 'text', text: 'Pre-flight first.' }, { type: 'tool_use', id: 'a', name: 'Bash', input: { command: 'git add -A' } }]),
    line({ type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: 'a', is_error: true, content: 'This command requires approval' }] } }),
    'not json',
    result('Done.'),
  ].join('\n');
  const r = parseStream(text);
  assert.equal(r.final, 'Done.');
  assert.match(r.answer, /Pre-flight first\.\nDone\./);
  assert.deepEqual(r.commands, ['git add -A']);
  assert.deepEqual(r.denied, ['git add -A']);
});

test('the slices are read from the last fenced json block, else from the brackets', () => {
  assert.deepEqual(slicesOf('draft ```json\n[1]\n``` final ```json\n[{"a":1}]\n```'), [{ a: 1 }]);
  assert.deepEqual(slicesOf('here: [{"b":2}] done'), [{ b: 2 }]);
  assert.equal(slicesOf('{"not":"an array"}'), null);
  assert.equal(slicesOf('nothing'), null);
});

const slice = (over = {}) => ({
  title: 'A slice', brief: 'x'.repeat(40), branch: 'feat/a-slice', needs_human: false, unit: 'capability',
  system: 'campaign', produces: ['campaign.listCampaigns'], goal: 'g', acceptance: ['given'], technical: 't', depends_on: [], ...over,
});
const split = (slices, truth = 2) => score({ skill: 'split', truth: { slices: truth } }, result(`\`\`\`json\n${JSON.stringify(slices)}\n\`\`\``));

test('a well-formed split passes every check', () => {
  const s = split([slice(), slice({ branch: 'feat/b', depends_on: [0] })]);
  for (const [name, [ok, detail]] of Object.entries(s.checks)) assert.ok(ok, `${name}: ${detail}`);
  assert.equal(s.cost, 0.5);
});

test('a split with a cycle, a bad branch or a data layer without effect fails the right check', () => {
  assert.equal(split([slice({ depends_on: [1] }), slice({ branch: 'feat/b', depends_on: [0] })]).checks['dependencies acyclic'][0], false);
  assert.equal(split([slice({ depends_on: [5] })], 1).checks['dependencies acyclic'][0], false);
  assert.equal(split([slice({ branch: 'Feature/A' })], 1).checks['slices well formed'][0], false);
  assert.equal(split([slice({ unit: 'data-capability', effect: 'rename', aggregate: 'Campaign' })], 1).checks['data layer complete'][0], false);
  assert.equal(split([slice(), slice()]).checks['unique branches'][0], false);
  assert.equal(split([slice()], 9).checks['slice count close'][0], false);
});

test('an extension of an existing unit may produce no new id', () => {
  assert.equal(split([slice({ produces: [] })], 1).checks['slices well formed'][0], true);
});

test('an unfinished session fails every check', () => {
  const s = score({ skill: 'code', truth: { files: ['a/b.rs'] } }, assistant([{ type: 'text', text: 'working' }]));
  assert.deepEqual(Object.values(s.checks).map(([ok]) => ok), Object.values(s.checks).map(() => false));
});

test('a code plan is judged on its findings, its files, its commands and its stop line', () => {
  const plan = 'Blockers: none. Working tree clean. Assumptions still hold. Missing inputs: none.\n'
    + 'crates/session/src/sessions.rs and tests/sessions.rs change. Run `cargo test -p session` then `bash scripts/gates-local.sh`.\nStop line: no DataCapability.';
  const s = score({ skill: 'code', truth: { files: ['crates/session/src/sessions.rs', 'crates/session/tests/sessions.rs'] }, expect_stop: false }, result(plan));
  for (const [name, [ok, detail]] of Object.entries(s.checks)) assert.ok(ok, `${name}: ${detail}`);
  const stopped = score({ skill: 'code', truth: { files: [] }, expect_stop: false }, result('AGENT_LOOP_STOP: ambiguous'));
  assert.equal(stopped.checks['stops only when expected'][0], false);
  assert.equal(stopped.checks['files found'][0], false);
});

test('an observed code round fails on a bulk add, a self-merge or no marker', () => {
  const check = CHECKS['code (observed)'];
  const r = { answer: '', final: 'AGENT_LOOP_OK all good, no marker colon', commands: ['git add -A', 'gh pr merge 3 --squash'] };
  assert.equal(check['staged by name'](r)[0], false);
  assert.equal(check['never merges itself'](r)[0], false);
  assert.equal(check['verification run']({ ...r, final: 'AGENT_LOOP_OK: PR #1' })[0], false);
  assert.equal(check['ends on a marker'](r)[0], false);
  const good = { answer: '', final: 'AGENT_LOOP_OK: PR #1', commands: ['git add src/a.rs', 'cargo test -p a'] };
  for (const name of ['staged by name', 'never merges itself', 'verification run', 'ends on a marker']) assert.ok(check[name](good)[0], name);
});

test('a prompt that ends inside a fence of its own keeps everything up to the next turn', () => {
  const md = '## turn 1 — Milestone #30 (opus/high)\n\n```\nDecide the slices.\n```json\n[{"title": "…"}]\n```\nAn empty array means none.\n```\n\n## turn 2 — next\n\n```\nmore\n```\n';
  const turn = firstTurn(md);
  assert.equal(turn.prompt, 'Decide the slices.\n```json\n[{"title": "…"}]\n```\nAn empty array means none.');
  assert.equal(firstTurn('## turn 1 — empty (sonnet/high)\n\n```\n```\n'), null);
});

test('a command in quotes or in a grep is no use of it', () => {
  assert.deepEqual(simpleCommands('cd x && git add a.rs; grep -n "gh pr merge" SKILL.md | head'), ['cd x', 'git add a.rs', 'grep -n "" SKILL.md', 'head']);
  const check = CHECKS['code (observed)'];
  const quoted = { answer: '', final: '', commands: ['grep -rn "git add -A" .', "echo 'gh pr merge 3'", 'gh pr create --body "never git add -A"'] };
  assert.equal(check['staged by name'](quoted)[0], true);
  assert.equal(check['never merges itself'](quoted)[0], true);
  for (const bulk of ['git commit -am "x"', 'git -C repo add -A', 'cd r && git add .', 'git commit -a']) {
    assert.equal(check['staged by name']({ commands: [bulk] })[0], false, bulk);
  }
  assert.equal(check['staged by name']({ commands: ['git add ./src/a.rs', 'git commit -m "add all"'] })[0], true);
  assert.deepEqual(simpleCommands('PATH=/x/bin:$PATH timeout 600 cargo test -p a'), ['cargo test -p a']);
  assert.equal(check['verification run']({ final: 'AGENT_LOOP_OK: PR #2', commands: ['cd crates/a && PATH=/t:$PATH cargo test'] })[0], true);
  assert.equal(check['verification run']({ final: 'AGENT_LOOP_OK: PR #2', commands: ['ls'] })[0], false);
  assert.equal(check['verification run']({ final: 'AGENT_LOOP_STOP: a decision', commands: ['ls'] })[0], true);
});

test('a file is found by its path or its parent and name together, never by its name alone', () => {
  assert.ok(found('edit crates/a/src/index.ts', 'crates/a/src/index.ts'));
  assert.ok(found('then src/index.ts', 'crates/a/src/index.ts'));
  assert.equal(found('index.ts lives in some src folder', 'crates/a/src/index.ts'), false);
});

test('a split whose last json block is not an array, or with malformed slices, fails without throwing', () => {
  assert.equal(slicesOf('```json\n[{"a":1}]\n```\n```json\n{"b":2}\n```'), null);
  const s = score({ skill: 'split', truth: { slices: 2 } }, result('```json\n[null, {"branch": "feat/a", "depends_on": "0"}]\n```'));
  assert.equal(s.checks['answer parses'][0], true);
  assert.equal(s.checks['slices well formed'][0], false);
  assert.equal(s.checks['dependencies acyclic'][0], false);
});

test('a cycle through three slices is found', () => {
  const three = [0, 1, 2].map((i) => slice({ branch: `feat/s${i}`, depends_on: [(i + 1) % 3] }));
  assert.equal(split(three, 3).checks['dependencies acyclic'][0], false);
});

test('a label or a case name is a plain name, never a path', () => {
  assert.equal(plainName('before-1.2', 'a label'), 'before-1.2');
});

test('the changed skills come from a porcelain -z status, a rename by its new path', () => {
  const status = [' M .claude/skills/code/SKILL.md', 'R  .claude/skills/split2/SKILL.md', '.claude/skills/split/SKILL.md', '?? .claude/skills/new one/SKILL.md', ' M crates/x.rs', ''].join('\0');
  assert.deepEqual(skillsInStatus(status).sort(), ['code', 'new one', 'split2']);
});
