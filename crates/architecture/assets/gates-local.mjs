#!/usr/bin/env node
// Every gate of .github/workflows/gates.yml, run before the pull request on
// a full copy of this checkout's work — installed by `harness init-repo`
// with scripts/gates-local.sh, its entry. No dependency: node, git, and
// docker for a job that declares a Postgres service.
//
//   bash scripts/gates-local.sh [--only a,b] [--failed] [--rules]
//                               [--base <branch>] [--head <branch>]
//                               [--label <name>]...
//                               [--no-gh] [--json] [--list]
//
// What it runs: gates.yml itself, read now from the merged tree it judges —
// the one the CI runs, never a copy that goes stale when the base adds a
// gate. Each job of the workflow runs in parallel with the others, in a
// worktree of its own under .git/gates-local/, its steps in their order with
// the CI's semantics (`!cancelled()`: every gate runs whatever the others
// said). `rules-hold` starts the product and takes minutes: off unless
// `--rules`. A run takes minutes and says each job's gates as the job ends:
// give it a long timeout (10 minutes) or run it in the background.
//
// The tree it judges is the pull request's: the work as it stands —
// committed, staged, unstaged and untracked, everything a sparse checkout
// hides included — merged with the tip of the base, as the CI checks out
// `refs/pull/<n>/merge`. A conflict with the base is a finding of its own.
//
// The PR's context is simulated: a `pull_request` event on the base the
// session facts name ($HARNESS_FACTS, written by the harness), else the
// pull request `gh pr view` finds for this branch, else `--base`
// (`main_agent` by default). Labels come from the same places, or `--label`;
// the PR's head branch from the same places, or `--head` (the rules' task's
// PR is told by its branch, `rules/<n>`).
//
// Expressions: every `${{ … }}` and `if:` of the workflow is looked up in a
// table of the exact strings the harness's gates.yml uses (EXPRESSIONS). A
// string the table does not know makes its step `unreadable`, never skipped
// in silence; a harness test keeps the table in step with the workflow. So
// does a key it does not implement — a job's `needs:`, `if:`, `strategy:`,
// a workflow's `env:`, `defaults:` — which makes the whole job unreadable.
//
// A red gate is judged against the base: from the base's last finished
// `gates` run at the same commit (gh), else by running the red gates of
// the jobs that build nothing (no cargo) on the base itself. It is
// `base-red`, not this change's, only when every error it reports
// (`::error file=…::…`) is one the base reports too.
//
// The closure check — what the harness's loop judges after the session:
// the files changed outside the unit's closure, a unit missing, a skeleton
// hole — is the harness's own code, run through its binary ($HARNESS_BIN,
// set in a harness session); elsewhere it is skipped and said.
//
// `--failed` reruns what the last report left red: its red gates, the jobs
// whose setup failed, and the closure check; after a merge conflict, or
// with no report, everything. Nothing red and the closure ok: nothing runs.
//
// Output: one line per gate as its job ends, `ok · <gate> · 3s`, `red ·
// <gate>` (judged against the base at the end: `FAIL · <gate> · 12s` or
// `base-red · <gate>`), `skip · <gate> · <why>`, `unreadable · <gate> ·
// <why>`, `tolerated · <gate>` (failed under `continue-on-error`); each
// failing gate's log, filtered, with its full path; the closure check's
// findings; then the totals. The report is .git/gates-local/report.json
// (`partial` names the flags of a run that judged a subset). Exit 1 when a
// gate or the closure check fails (or the base conflicts), 2 when a step or
// the closure check could not be judged, 0 otherwise.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const WORKFLOW = '.github/workflows/gates.yml';

// The jobs a local run leaves out unless asked, and the flag that runs them.
const OFF_BY_DEFAULT = { 'rules-hold': '--rules' };

// The actions a step may use: the runner's setup, which a local run already
// has. Anything else is a step this script cannot reproduce.
const KNOWN_USES = [
  'actions/checkout',
  'actions/setup-node',
  'dtolnay/rust-toolchain',
  'actions/upload-artifact',
];

const STEP_KEYS = new Set([
  'id', 'name', 'if', 'env', 'run', 'shell', 'uses', 'with',
  'working-directory', 'continue-on-error', 'timeout-minutes',
]);
const JOB_KEYS = new Set(['runs-on', 'services', 'steps', 'env', 'permissions', 'name']);
// The workflow's own keys this script reads, or that change nothing a local
// run does: anything else applies to every job and makes them unreadable.
const WORKFLOW_KEYS = new Set(['name', 'on', 'jobs', 'permissions', 'concurrency']);

// How long one step may run before it is killed, in seconds.
const STEP_TIMEOUT = Number(process.env.GATES_LOCAL_STEP_TIMEOUT || 1200);

// ---------------------------------------------------------------------------
// The workflow, read: a parser for the YAML the harness writes.
// ---------------------------------------------------------------------------

class Unreadable extends Error {}

const indentOf = (line) => line.length - line.trimStart().length;
const isContent = (line) => {
  const t = line.trim();
  return t !== '' && !t.startsWith('#');
};

function splitKey(text) {
  let key;
  let rest;
  if (text[0] === '"' || text[0] === "'") {
    const end = text.indexOf(text[0], 1);
    if (end < 0) return null;
    key = text.slice(1, end);
    rest = text.slice(end + 1);
    if (!rest.startsWith(':')) return null;
    rest = rest.slice(1);
  } else {
    const at = /:(\s|$)/.exec(text);
    if (!at) return null;
    key = text.slice(0, at.index);
    rest = text.slice(at.index + 1);
  }
  if (rest !== '' && !/^\s/.test(rest)) return null;
  return [key.trim(), rest.trim()];
}

function unquote(text, where) {
  if (text.startsWith("'")) {
    if (!text.endsWith("'") || text.length < 2) throw new Unreadable(`${where}: unterminated quote`);
    return text.slice(1, -1).replace(/''/g, "'");
  }
  if (text.startsWith('"')) {
    if (!text.endsWith('"') || text.length < 2) throw new Unreadable(`${where}: unterminated quote`);
    return text.slice(1, -1).replace(/\\(["\\nt])/g, (_, c) => ({ n: '\n', t: '\t' })[c] ?? c);
  }
  return text;
}

function scalar(text, where) {
  if (/^[&*!]/.test(text)) throw new Unreadable(`${where}: anchors, aliases and tags are not read`);
  if (text.startsWith('{')) {
    if (text === '{}') return {};
    throw new Unreadable(`${where}: a flow mapping is not read`);
  }
  if (text.startsWith('[')) {
    if (!text.endsWith(']')) throw new Unreadable(`${where}: a flow sequence on several lines is not read`);
    const inner = text.slice(1, -1).trim();
    if (inner === '') return [];
    const items = [];
    let current = '';
    let quote = null;
    for (const c of inner) {
      if (quote) {
        current += c;
        if (c === quote) quote = null;
      } else if (c === '"' || c === "'") {
        quote = c;
        current += c;
      } else if (c === ',') {
        items.push(current.trim());
        current = '';
      } else {
        current += c;
      }
    }
    items.push(current.trim());
    return items.map((item) => unquote(item, where));
  }
  if (text.startsWith('"') || text.startsWith("'")) return unquote(text, where);
  // A comment after a plain scalar.
  return text.replace(/\s+#.*$/, '');
}

function parseYaml(text) {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  let i = 0;
  const where = () => `line ${i + 1}`;
  const skip = () => {
    while (i < lines.length && !isContent(lines[i])) i += 1;
  };
  for (const [n, line] of lines.entries()) {
    if (/^\t/.test(line)) throw new Unreadable(`line ${n + 1}: a tab indents it`);
  }

  function block(keyIndent, style) {
    i += 1;
    const body = [];
    while (i < lines.length && (lines[i].trim() === '' || indentOf(lines[i]) > keyIndent)) {
      body.push(lines[i]);
      i += 1;
    }
    while (body.length && body[body.length - 1].trim() === '') body.pop();
    const first = body.find((line) => line.trim() !== '');
    const cut = first === undefined ? 0 : indentOf(first);
    const kept = body.map((line) => (line.trim() === '' ? '' : line.slice(cut)));
    if (style.startsWith('>')) {
      const folded = kept.join('\n').replace(/([^\n])\n(?!\n)/g, '$1 ');
      return style.endsWith('-') ? folded : `${folded}\n`;
    }
    const literal = kept.join('\n');
    return style.endsWith('-') ? literal : `${literal}\n`;
  }

  function value(rest, keyIndent) {
    if (rest === '') {
      i += 1;
      skip();
      if (i >= lines.length) return null;
      const next = indentOf(lines[i]);
      const t = lines[i].trim();
      if (next > keyIndent) return node(next);
      if (next === keyIndent && (t.startsWith('- ') || t === '-')) return sequence(next);
      return null;
    }
    if (/^[|>][-+]?$/.test(rest)) return block(keyIndent, rest);
    const read = scalar(rest, where());
    i += 1;
    return read;
  }

  function mapping(indent) {
    const out = {};
    for (;;) {
      skip();
      if (i >= lines.length) break;
      const line = lines[i];
      const at = indentOf(line);
      if (at < indent) break;
      if (at > indent) throw new Unreadable(`${where()}: unexpected indentation`);
      const t = line.trim();
      if (t.startsWith('- ') || t === '-') break;
      const split = splitKey(t);
      if (!split) throw new Unreadable(`${where()}: not a key`);
      const [key, rest] = split;
      if (key === '<<') throw new Unreadable(`${where()}: merge keys are not read`);
      out[key] = value(rest, indent);
    }
    return out;
  }

  function sequence(indent) {
    const out = [];
    for (;;) {
      skip();
      if (i >= lines.length) break;
      const line = lines[i];
      const at = indentOf(line);
      const t = line.trim();
      if (at !== indent || !(t.startsWith('- ') || t === '-')) {
        if (at > indent) throw new Unreadable(`${where()}: unexpected indentation`);
        break;
      }
      if (t === '-') {
        i += 1;
        out.push(node(indent + 1));
        continue;
      }
      const after = t.slice(2);
      const rest = after.trimStart();
      const column = at + 2 + (after.length - rest.length);
      const split = rest.startsWith('"') || rest.startsWith("'") ? null : splitKey(rest);
      if (split) {
        lines[i] = ' '.repeat(column) + rest;
        out.push(mapping(column));
      } else {
        out.push(scalar(rest, where()));
        i += 1;
      }
    }
    return out;
  }

  function node(indent) {
    skip();
    if (i >= lines.length) return null;
    const t = lines[i].trim();
    return t.startsWith('- ') || t === '-' ? sequence(indent) : mapping(indent);
  }

  const doc = node(0);
  skip();
  if (i < lines.length) throw new Unreadable(`${where()}: left unread`);
  return doc;
}

// The jobs and their steps, each step a gate, a setup step, or unreadable.
function readWorkflow(text) {
  let doc;
  try {
    doc = parseYaml(text);
  } catch (error) {
    if (error instanceof Unreadable) return { error: error.message, jobs: [] };
    throw error;
  }
  const jobs = [];
  const unread = doc && typeof doc === 'object' ? Object.keys(doc).filter((key) => !WORKFLOW_KEYS.has(key)) : [];
  const workflowUnread = unread.length ? `workflow key(s) not read: ${unread.join(', ')}` : null;
  for (const [id, job] of Object.entries(doc?.jobs ?? {})) {
    const read = { id, env: {}, postgres: null, steps: [], unreadable: workflowUnread };
    jobs.push(read);
    if (!job || typeof job !== 'object') {
      read.unreadable = 'not a mapping';
      continue;
    }
    const strange = Object.keys(job).filter((key) => !JOB_KEYS.has(key));
    if (strange.length) read.unreadable ??= `job key(s) not read: ${strange.join(', ')}`;
    read.env = stringMap(job.env);
    const services = job.services ?? {};
    for (const [name, service] of Object.entries(services)) {
      if (name !== 'postgres' || !service?.image) {
        read.unreadable = `service ${name} is not read`;
        continue;
      }
      read.postgres = { image: service.image, env: stringMap(service.env) };
    }
    for (const [index, step] of (job.steps ?? []).entries()) {
      read.steps.push(readStep(step, index));
    }
  }
  return { jobs };
}

function stringMap(value) {
  if (!value || typeof value !== 'object') return {};
  return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, v == null ? '' : String(v)]));
}

function readStep(step, index) {
  const id = step?.id ? String(step.id) : null;
  const label = id ?? step?.name ?? `step ${index + 1}`;
  if (!step || typeof step !== 'object') return { id: label, kind: 'unreadable', why: 'not a mapping' };
  const strange = Object.keys(step).filter((key) => !STEP_KEYS.has(key));
  if (strange.length) return { id: label, kind: 'unreadable', why: `key(s) not read: ${strange.join(', ')}` };
  if (step.uses) {
    const action = String(step.uses).split('@')[0];
    if (KNOWN_USES.includes(action)) return { id: label, kind: 'setup-action', uses: step.uses };
    return { id: label, kind: 'unreadable', why: `uses ${step.uses}: no local equivalent` };
  }
  if (typeof step.run !== 'string') return { id: label, kind: 'unreadable', why: 'no run' };
  const condition = step.if == null ? '' : String(step.if);
  const shell = step.shell == null ? '' : String(step.shell);
  if (!['', 'bash', 'sh'].includes(shell)) return { id: label, kind: 'unreadable', why: `shell ${shell} is not run` };
  const tolerant = step['continue-on-error'] == null ? 'false' : String(step['continue-on-error']);
  if (!['true', 'false'].includes(tolerant)) {
    return { id: label, kind: 'unreadable', why: `continue-on-error: ${tolerant} is not true or false` };
  }
  const minutes = step['timeout-minutes'] == null ? null : String(step['timeout-minutes']);
  if (minutes !== null && !/^\d+(\.\d+)?$/.test(minutes)) {
    return { id: label, kind: 'unreadable', why: `timeout-minutes: ${minutes} is not a number` };
  }
  const read = {
    id: label,
    kind: id ? 'gate' : 'setup',
    if: condition,
    env: stringMap(step.env),
    run: step.run,
    shell,
    directory: step['working-directory'] ? String(step['working-directory']) : '',
    tolerant: tolerant === 'true',
    timeout: minutes === null ? STEP_TIMEOUT : Math.max(1, Math.round(Number(minutes) * 60)),
  };
  // A step that only names the failed ones once a job is red.
  if (!id && /\bfailure\(\)/.test(condition)) read.kind = 'report';
  return read;
}

// ---------------------------------------------------------------------------
// Expressions: the exact strings gates.yml uses, and nothing else.
// ---------------------------------------------------------------------------

// The rules' own change: told by its branch, `rules/<n>`, as gates.yml does
// now; by its label, as a base's older gates.yml still does.
const NOT_RULES_BRANCH = (c) => !String(c.headRef ?? '').startsWith('rules/');
const NOT_RULES = (c) => !c.labels.includes('harness:rules');

// Conditions (`if:`), by their text without `${{ }}`. An absent `if:` is
// `success()`, as on the runner.
const CONDITIONS = new Map([
  ['', (c) => !c.failed],
  ['success()', (c) => !c.failed],
  ['failure()', (c) => c.failed],
  ['always()', () => true],
  ['!cancelled()', () => true],
  ["!cancelled() && github.event_name == 'pull_request'", () => true],
  [
    "!cancelled() && github.event_name == 'pull_request' && !contains(github.event.pull_request.labels.*.name, 'harness:rules')",
    NOT_RULES,
  ],
  [
    "!cancelled() && github.event_name == 'pull_request' && !startsWith(github.head_ref, 'rules/')",
    NOT_RULES_BRANCH,
  ],
]);

const NEEDS_PR = { skip: 'needs the pull request itself (its number)' };
const NEEDS_TOKEN = { skip: "needs the CI's token: run on the pull request only" };

// Values (`${{ … }}` in `run:` and `env:`).
const VALUES = new Map([
  ['github.event_name', () => 'pull_request'],
  ['github.base_ref', (c) => c.baseRef],
  ['github.head_ref', (c) => c.headRef],
  ['github.repository', (c) => c.repository],
  ['github.workspace', (c) => c.workspace],
  ['github.event.pull_request.base.sha', (c) => c.baseSha],
  ['github.event.pull_request.base.sha || github.event.before', (c) => c.baseSha],
  ['github.event.pull_request.head.sha', (c) => c.headSha],
  ['github.event.pull_request.head.repo.full_name', (c) => c.repository],
  ['github.event.pull_request.number', (c) => (c.number ? String(c.number) : NEEDS_PR)],
  ['github.token', () => NEEDS_TOKEN],
  ['toJSON(steps)', (c) => JSON.stringify(c.steps ?? {})],
  [
    "github.event_name == 'pull_request' && github.base_ref == 'main_agent' && !contains(github.event.pull_request.labels.*.name, 'harness:rules') && '--strict' || ''",
    (c) => (c.baseRef === 'main_agent' && NOT_RULES(c) ? '--strict' : ''),
  ],
  [
    "github.event_name == 'pull_request' && github.base_ref == 'main_agent' && !startsWith(github.head_ref, 'rules/') && '--strict' || ''",
    (c) => (c.baseRef === 'main_agent' && NOT_RULES_BRANCH(c) ? '--strict' : ''),
  ],
]);

const normalize = (expression) =>
  expression
    .trim()
    .replace(/^\$\{\{([\s\S]*)\}\}$/, '$1')
    .trim()
    .replace(/\s+/g, ' ');

// A condition's verdict: { value } or { unreadable }.
function condition(text, context) {
  const key = normalize(text);
  const known = CONDITIONS.get(key);
  if (!known) return { unreadable: `if: ${text.trim()} is not an expression this script knows` };
  return { value: Boolean(known(context)) };
}

// Every `${{ … }}` of `text` replaced: { text }, { skip } or { unreadable }.
function substitute(text, context) {
  let skip = null;
  let unreadable = null;
  const out = text.replace(/\$\{\{([\s\S]*?)\}\}/g, (whole, inner) => {
    const known = VALUES.get(normalize(inner));
    if (!known) {
      unreadable ??= `${whole.trim()} is not an expression this script knows`;
      return '';
    }
    const value = known(context);
    if (value && typeof value === 'object') {
      skip ??= value.skip;
      return '';
    }
    return value ?? '';
  });
  if (unreadable) return { unreadable };
  if (skip) return { skip };
  return { text: out };
}

// ---------------------------------------------------------------------------
// Processes.
// ---------------------------------------------------------------------------

function run(command, args, options = {}) {
  const out = spawnSync(command, args, {
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
    timeout: (options.timeout ?? 120) * 1000,
    cwd: options.cwd,
    env: options.env ?? process.env,
    input: options.input,
  });
  return {
    ok: out.status === 0,
    code: out.status,
    stdout: options.raw ? out.stdout ?? '' : (out.stdout ?? '').trim(),
    stderr: (out.stderr ?? '').trim(),
    missing: out.error?.code === 'ENOENT',
  };
}

const IDENTITY = {
  GIT_AUTHOR_NAME: 'gates-local',
  GIT_AUTHOR_EMAIL: 'gates-local@localhost',
  GIT_COMMITTER_NAME: 'gates-local',
  GIT_COMMITTER_EMAIL: 'gates-local@localhost',
};

function git(args, options = {}) {
  return run('git', args, options);
}

function gitOk(args, options = {}) {
  const out = git(args, options);
  if (!out.ok) throw new Error(`git ${args.join(' ')}: ${out.stderr || out.stdout}`);
  return out.stdout;
}

// The environment a git of a gates worktree runs with: sparse checkout off,
// whatever the session's checkout says, and no index or directory inherited.
function gitEnvironment(base = process.env) {
  const env = { ...base };
  delete env.GIT_INDEX_FILE;
  delete env.GIT_DIR;
  delete env.GIT_WORK_TREE;
  const count = Number(env.GIT_CONFIG_COUNT || 0);
  env.GIT_CONFIG_COUNT = String(count + 1);
  env[`GIT_CONFIG_KEY_${count}`] = 'core.sparseCheckout';
  env[`GIT_CONFIG_VALUE_${count}`] = 'false';
  return env;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === 'EPERM';
  }
};

// ---------------------------------------------------------------------------
// The run's state, and what is cleaned on the way out.
// ---------------------------------------------------------------------------

const live = { containers: new Set(), children: new Set(), files: new Set(), lock: null };

function cleanup() {
  for (const child of live.children) {
    try {
      process.kill(-child.pid, 'SIGKILL');
    } catch {
      // Already gone.
    }
  }
  for (const name of live.containers) run(docker(), ['rm', '-f', name], { timeout: 30 });
  live.containers.clear();
  for (const file of live.files) fs.rmSync(file, { force: true, recursive: true });
  if (live.lock) fs.rmSync(live.lock, { force: true });
}

process.on('exit', cleanup);
for (const [signal, code] of [['SIGINT', 130], ['SIGTERM', 143], ['SIGHUP', 129]]) {
  process.on(signal, () => {
    cleanup();
    process.exit(code);
  });
}

const docker = () => process.env.GATES_LOCAL_DOCKER || 'docker';

// The containers and the lock a killed run (SIGKILL: no trap ran) left.
function reap(home) {
  const lock = path.join(home, 'lock');
  if (fs.existsSync(lock)) {
    const pid = Number(fs.readFileSync(lock, 'utf8').trim());
    if (pid && pid !== process.pid && alive(pid)) {
      throw new Error(`another gates-local runs on this checkout (pid ${pid})`);
    }
    fs.rmSync(lock, { force: true });
  }
  fs.writeFileSync(lock, `${process.pid}\n`);
  live.lock = lock;
}

function reapContainers() {
  const listed = run(docker(), ['ps', '-a', '--filter', 'name=gates-local-', '--format', '{{.Names}}'], { timeout: 30 });
  if (!listed.ok) return;
  for (const name of listed.stdout.split('\n').filter(Boolean)) {
    const pid = Number(/^gates-local-(\d+)-/.exec(name)?.[1]);
    if (pid && !alive(pid)) run(docker(), ['rm', '-f', name], { timeout: 30 });
  }
}

// ---------------------------------------------------------------------------
// The pull request's context.
// ---------------------------------------------------------------------------

function readFacts() {
  const file = process.env.HARNESS_FACTS;
  if (!file || !fs.existsSync(file)) return null;
  try {
    return { file, ...JSON.parse(fs.readFileSync(file, 'utf8')) };
  } catch {
    return null;
  }
}

function pullRequest(root, offline) {
  if (offline) return null;
  const out = run('gh', ['pr', 'view', '--json', 'number,baseRefName,headRefName,labels,body,state'], { cwd: root, timeout: 30 });
  if (!out.ok) return null;
  try {
    const pr = JSON.parse(out.stdout);
    if (pr.state && pr.state !== 'OPEN') return null;
    const task = /^\s*(?:closes|fixes|resolves)\s+#(\d+)\s*$/im.exec(pr.body ?? '')?.[1];
    return {
      number: pr.number,
      base: pr.baseRefName,
      head: pr.headRefName,
      labels: (pr.labels ?? []).map((label) => label.name),
      task: task ? Number(task) : null,
    };
  } catch {
    return null;
  }
}

function repositoryOf(url) {
  const match = /[:/]([^/:]+\/[^/]+?)(?:\.git)?\/?$/.exec(url ?? '');
  return match ? match[1] : '';
}

function context(options) {
  const root = gitOk(['rev-parse', '--show-toplevel']);
  const common = path.resolve(root, gitOk(['rev-parse', '--git-common-dir'], { cwd: root }));
  const home = path.join(common, 'gates-local');
  fs.mkdirSync(home, { recursive: true });
  const facts = readFacts();
  const current = git(['branch', '--show-current'], { cwd: root }).stdout;
  const named = (options.base || facts?.base || '').replace(/^origin\//, '');
  // On the base itself, before its `git checkout -b`, the pull request gh
  // would find is the base's own (a milestone's into main_agent), never this
  // work's.
  const pr = named && current === named ? null : pullRequest(root, options.offline);
  const base = (named || pr?.base || 'main_agent').replace(/^origin\//, '');
  const labels = options.labels.length ? options.labels : facts?.labels ?? pr?.labels ?? [];
  const fetched = git(['fetch', '--no-tags', '-q', 'origin', `+refs/heads/${base}:refs/remotes/origin/${base}`], { cwd: root, timeout: 120 });
  const baseTip = git(['rev-parse', '--verify', '-q', `origin/${base}^{commit}`], { cwd: root });
  if (!baseTip.ok) throw new Error(`no origin/${base} to merge with: pass --base <the branch your PR targets>`);
  // Before its `git checkout -b`, a session is on the base: the PR's head is
  // the branch its task declares, never the base (`milestone/*` would make
  // the gates read a milestone's pull request).
  const headRef = options.head || pr?.head || (current && current !== base ? current : facts?.branch || 'gates-local');
  return {
    root,
    home,
    facts,
    pr,
    offline: options.offline,
    baseRef: base,
    baseSha: baseTip.stdout,
    headRef,
    head: gitOk(['rev-parse', 'HEAD'], { cwd: root }),
    labels,
    number: pr?.number ?? null,
    task: facts?.task ?? pr?.task ?? null,
    repository: repositoryOf(git(['remote', 'get-url', 'origin'], { cwd: root }).stdout),
    fetchFailed: !fetched.ok,
    target: process.env.CARGO_TARGET_DIR || path.join(root, 'target'),
  };
}

// ---------------------------------------------------------------------------
// The tree judged: a snapshot of the work, merged with the base's tip.
// ---------------------------------------------------------------------------

function snapshot(ctx) {
  const index = path.join(ctx.home, `index-${process.pid}`);
  live.files.add(index);
  // From the real index, so its skip-worktree bits carry over: the files a
  // sparse checkout hides stay what HEAD has, never "deleted".
  const real = path.resolve(ctx.root, gitOk(['rev-parse', '--git-path', 'index'], { cwd: ctx.root }));
  if (fs.existsSync(real)) fs.copyFileSync(real, index);
  const env = { ...process.env, GIT_INDEX_FILE: index };
  if (!fs.existsSync(real)) gitOk(['read-tree', 'HEAD'], { cwd: ctx.root, env });
  gitOk(['add', '-A'], { cwd: ctx.root, env });
  const tree = gitOk(['write-tree'], { cwd: ctx.root, env });
  fs.rmSync(index, { force: true });
  live.files.delete(index);
  const commit = gitOk(['commit-tree', tree, '-p', 'HEAD', '-m', 'gates-local: the work as it stands'], {
    cwd: ctx.root,
    env: { ...process.env, ...IDENTITY },
  });
  const untracked = git(['ls-files', '--others', '--exclude-standard'], { cwd: ctx.root }).stdout.split('\n').filter(Boolean);
  return { tree, commit, untracked };
}

function mergedWithBase(ctx, snap) {
  if (git(['merge-base', '--is-ancestor', ctx.baseSha, snap.commit], { cwd: ctx.root }).ok) {
    return { commit: snap.commit };
  }
  const out = git(['merge-tree', '--write-tree', '--no-messages', ctx.baseSha, snap.commit], { cwd: ctx.root });
  if (out.code === 0) {
    const tree = out.stdout.split('\n')[0];
    const commit = gitOk(['commit-tree', tree, '-p', ctx.baseSha, '-p', snap.commit, '-m', 'gates-local: merged with the base'], {
      cwd: ctx.root,
      env: { ...process.env, ...IDENTITY },
    });
    return { commit };
  }
  if (out.code === 1) {
    const files = [...new Set(out.stdout.split('\n').slice(1).map((line) => line.split('\t')[1]).filter(Boolean))];
    return { conflict: files };
  }
  return { commit: snap.commit, note: 'this git cannot merge in memory (2.38 or later): the gates read the work unmerged' };
}

// A worktree of `commit` under .git/gates-local/<name>, reused across runs.
function worktree(ctx, name, commit) {
  const dir = path.join(ctx.home, name);
  const env = gitEnvironment();
  if (fs.existsSync(path.join(dir, '.git'))) {
    const moved = git(['checkout', '-q', '-f', '--detach', commit], { cwd: dir, env });
    if (moved.ok) {
      gitOk(['clean', '-ffdxq', '-e', 'node_modules'], { cwd: dir, env });
      return dir;
    }
    fs.rmSync(dir, { recursive: true, force: true });
  }
  git(['worktree', 'prune'], { cwd: ctx.root, env });
  gitOk(['worktree', 'add', '-q', '-f', '--detach', dir, commit], { cwd: ctx.root, env });
  return dir;
}

// ---------------------------------------------------------------------------
// A job's Postgres service: a container of its own, removed on the way out.
// ---------------------------------------------------------------------------

async function postgres(ctx, job, tag, serviceEnv) {
  if (!run(docker(), ['info', '--format', '{{.ServerVersion}}'], { timeout: 20 }).ok) {
    return { missing: 'no docker' };
  }
  reapContainers();
  const name = `gates-local-${process.pid}-${tag}`;
  const env = Object.entries(serviceEnv).flatMap(([k, v]) => ['-e', `${k}=${v}`]);
  const started = run(docker(), ['run', '--rm', '-d', '--name', name, '-p', '127.0.0.1::5432', ...env, job.postgres.image], { timeout: 600 });
  if (!started.ok) return { missing: `docker run failed: ${started.stderr.split('\n').pop()}` };
  live.containers.add(name);
  const port = /:(\d+)\s*$/.exec(run(docker(), ['port', name, '5432/tcp'], { timeout: 30 }).stdout.split('\n')[0] ?? '')?.[1];
  if (!port) return { missing: 'docker gave the service no port' };
  let ready = 0;
  for (let n = 0; n < 120 && ready < 2; n += 1) {
    ready = run(docker(), ['exec', name, 'psql', '-U', 'postgres', '-h', 'localhost', '-c', 'select 1'], { timeout: 20 }).ok ? ready + 1 : 0;
    await sleep(1000);
  }
  if (ready < 2) return { missing: 'the Postgres service never answered' };
  const env2 = { PGPORT: port };
  if (!run('bash', ['-c', 'command -v psql']).ok) {
    const shim = path.join(ctx.home, `shim-${process.pid}-${tag}`);
    fs.mkdirSync(shim, { recursive: true });
    live.files.add(shim);
    fs.writeFileSync(path.join(shim, 'psql'), psqlShim(name), { mode: 0o755 });
    env2.PATH = `${shim}${path.delimiter}${process.env.PATH}`;
  }
  return { env: env2, port };
}

// A psql run inside the container: a `-f <file>` of the host goes in on
// stdin, which the container can read, and the host's port is dropped.
function psqlShim(container) {
  return `#!/usr/bin/env bash
args=(); files=()
while [ $# -gt 0 ]; do
  case "$1" in
    -f) files+=("$2"); shift 2 ;;
    --file=*) files+=("\${1#--file=}"); shift ;;
    -f?*) files+=("\${1#-f}"); shift ;;
    -h|-p) shift 2 ;;
    --host=*|--port=*) shift ;;
    *) args+=("$1"); shift ;;
  esac
done
if [ \${#files[@]} -gt 0 ]; then
  cat "\${files[@]}" | exec ${docker()} exec -i -e PGPASSWORD -e PGOPTIONS ${container} psql -h localhost "\${args[@]}"
fi
exec ${docker()} exec -i -e PGPASSWORD -e PGOPTIONS ${container} psql -h localhost "\${args[@]}"
`;
}

// ---------------------------------------------------------------------------
// A job: its steps in order, in its worktree.
// ---------------------------------------------------------------------------

function spawnLogged(command, args, options, log, timeout = STEP_TIMEOUT) {
  return new Promise((resolve) => {
    const out = fs.openSync(log, 'w');
    const child = spawn(command, args, { ...options, stdio: ['ignore', out, out], detached: true });
    live.children.add(child);
    const timer = setTimeout(() => {
      try {
        process.kill(-child.pid, 'SIGKILL');
      } catch {
        // Gone already.
      }
    }, timeout * 1000);
    const done = (code) => {
      clearTimeout(timer);
      live.children.delete(child);
      fs.closeSync(out);
      resolve(code);
    };
    child.on('error', () => done(127));
    child.on('exit', (code, signal) => done(signal ? 128 : code));
  });
}

function readGithubEnv(file) {
  const out = {};
  if (!fs.existsSync(file)) return out;
  const lines = fs.readFileSync(file, 'utf8').split('\n');
  for (let n = 0; n < lines.length; n += 1) {
    const heredoc = /^([A-Za-z_][A-Za-z0-9_]*)<<(.+)$/.exec(lines[n]);
    if (heredoc) {
      const body = [];
      for (n += 1; n < lines.length && lines[n] !== heredoc[2]; n += 1) body.push(lines[n]);
      out[heredoc[1]] = body.join('\n');
      continue;
    }
    const pair = /^([A-Za-z_][A-Za-z0-9_]*)=(.*)$/.exec(lines[n]);
    if (pair) out[pair[1]] = pair[2];
  }
  return out;
}

const usesPostgres = (step) => /\bpsql\b|\bpg_isready\b/.test(step.run ?? '');

async function runJob(ctx, job, { dir, side, wanted, merge }) {
  const results = [];
  const logs = path.join(ctx.home, 'logs', side);
  fs.mkdirSync(logs, { recursive: true });
  const state = { failed: false, steps: {} };
  const facts = () => ({
    ...ctx,
    workspace: dir,
    headSha: merge,
    failed: state.failed,
    steps: state.steps,
  });
  const steps = job.steps.filter((step) => step.kind !== 'setup-action' && step.kind !== 'report');
  const chosen = steps.filter((step) => step.kind === 'setup' || wanted(step.id, job.id));
  // A job-wide value this script cannot give stops every gate of the job,
  // said: an empty value would run them on something the CI never sees.
  const stopped = (read, where) =>
    chosen
      .filter((step) => step.kind !== 'setup')
      .map((step) => ({
        id: step.id,
        job: job.id,
        kind: step.kind,
        outcome: read.unreadable ? 'unreadable' : 'skip',
        why: `${where}: ${read.unreadable ?? read.skip}`,
      }));
  let jobEnv = {};
  for (const [key, value] of Object.entries(job.env)) {
    const read = substitute(value, facts());
    if (read.text === undefined) return stopped(read, `job env ${key}`);
    jobEnv[key] = read.text;
  }
  let service = null;
  if (job.postgres && chosen.some(usesPostgres)) {
    const serviceEnv = {};
    for (const [key, value] of Object.entries(job.postgres.env)) {
      const read = substitute(value, facts());
      if (read.text === undefined) return stopped(read, `service env ${key}`);
      serviceEnv[key] = read.text;
    }
    service = await postgres(ctx, job, `${side}-${job.id}`, serviceEnv);
    if (service.env?.PGPORT) {
      jobEnv = Object.fromEntries(
        Object.entries(jobEnv).map(([k, v]) => [k, v.replace(/localhost:5432\b/g, `localhost:${service.port}`)]),
      );
    }
  }
  const githubEnv = path.join(logs, `${job.id}.github-env`);
  fs.writeFileSync(githubEnv, '');
  for (const step of chosen) {
    const said = (outcome, extra = {}) => results.push({ id: step.id, job: job.id, kind: step.kind, outcome, ...extra });
    if (step.kind === 'unreadable') {
      said('unreadable', { why: step.why });
      continue;
    }
    const verdict = condition(step.if, facts());
    if (verdict.unreadable) {
      said('unreadable', { why: verdict.unreadable });
      continue;
    }
    if (!verdict.value) {
      said('skip', { why: `if: ${step.if} is false for this pull request` });
      continue;
    }
    const env = {};
    let blocked = null;
    for (const [key, value] of Object.entries(step.env)) {
      const read = substitute(value, facts());
      if (!read.text && read.text !== '') {
        blocked = read;
        break;
      }
      env[key] = read.text;
    }
    const script = blocked ?? substitute(step.run, facts());
    if (script.unreadable) {
      said('unreadable', { why: script.unreadable });
      continue;
    }
    if (script.skip) {
      said('skip', { why: script.skip });
      continue;
    }
    if (usesPostgres(step) && service?.missing) {
      said('skip', { why: `${service.missing}: its Postgres service cannot start` });
      continue;
    }
    const file = path.join(logs, `${step.id.replace(/[^A-Za-z0-9_.-]/g, '_')}.sh`);
    fs.writeFileSync(file, script.text);
    const log = path.join(logs, `${step.id.replace(/[^A-Za-z0-9_.-]/g, '_')}.log`);
    const shell = step.shell === 'sh' ? ['sh', ['-e', file]] : step.shell === 'bash' ? ['bash', ['--noprofile', '--norc', '-eo', 'pipefail', file]] : ['bash', ['-e', file]];
    const started = Date.now();
    const code = await spawnLogged(
      shell[0],
      shell[1],
      {
        cwd: step.directory ? path.join(dir, step.directory) : dir,
        env: gitEnvironment({
          ...process.env,
          ...(service?.env ?? {}),
          ...jobEnv,
          ...env,
          CARGO_TARGET_DIR: ctx.target,
          GITHUB_EVENT_NAME: 'pull_request',
          GITHUB_BASE_REF: ctx.baseRef,
          GITHUB_HEAD_REF: ctx.headRef,
          GITHUB_REPOSITORY: ctx.repository,
          GITHUB_SHA: merge,
          GITHUB_WORKSPACE: dir,
          GITHUB_ENV: githubEnv,
        }),
      },
      log,
      step.timeout,
    );
    const seconds = Math.round((Date.now() - started) / 1000);
    const ok = code === 0;
    // `continue-on-error`: the step fails, the job goes on green, as on the runner.
    if (!ok && !step.tolerant) state.failed = true;
    state.steps[step.id] = { outcome: ok ? 'success' : 'failure', conclusion: ok || step.tolerant ? 'success' : 'failure' };
    jobEnv = { ...jobEnv, ...readGithubEnv(githubEnv) };
    if (!ok && step.tolerant) said('tolerated', { seconds, log, code, why: 'failed under continue-on-error: the job goes on, the CI stays green' });
    else said(ok ? 'ok' : 'FAIL', { seconds, log, code });
  }
  return results;
}

// ---------------------------------------------------------------------------
// Errors, and the base's verdict.
// ---------------------------------------------------------------------------

const decode = (text) => text.replace(/%0D/g, '\r').replace(/%0A/g, '\n').replace(/%3A/g, ':').replace(/%2C/g, ',').replace(/%25/g, '%');

// The `::error …::message` lines of a log, as `file\0message`.
function errorsOf(text) {
  const found = new Set();
  for (const line of text.split('\n')) {
    const match = /^::error(?:\s+([^:]*))?::(.*)$/.exec(line.trim());
    if (!match) continue;
    const file = /(?:^|,)\s*file=([^,]*)/.exec(match[1] ?? '')?.[1] ?? '';
    found.add(`${decode(file).trim()}\u0000${decode(match[2]).trim()}`);
  }
  return found;
}

// Whether a red gate is the base's: the base is red there too, and every
// error the head reports is one the base reports.
function isBaseRed(head, base) {
  if (!base.failed) return false;
  if (head.size === 0) return base.errors.size === 0;
  return [...head].every((error) => base.errors.has(error));
}

function gh(args, ctx) {
  const out = run('gh', args, { cwd: ctx.root, timeout: 60 });
  if (!out.ok) return null;
  try {
    return JSON.parse(out.stdout);
  } catch {
    return null;
  }
}

// The verdict of each gate on the base, from its last finished `gates` run at
// the base's tip, with the errors of the jobs asked for.
function baseRun(ctx) {
  if (ctx.offline) return null;
  const runs = gh(['run', 'list', '--workflow', path.basename(WORKFLOW), '--branch', ctx.baseRef, '--limit', '30', '--json', 'databaseId,headSha,status,conclusion'], ctx);
  const found = (runs ?? []).find((r) => r.headSha === ctx.baseSha && r.status === 'completed');
  if (!found) return null;
  const viewed = gh(['run', 'view', String(found.databaseId), '--json', 'jobs'], ctx);
  if (!viewed?.jobs) return null;
  const steps = new Map();
  for (const job of viewed.jobs) {
    for (const step of job.steps ?? []) steps.set(step.name, { conclusion: step.conclusion, job: job.databaseId });
  }
  const annotations = new Map();
  return {
    id: found.databaseId,
    verdict(gate) {
      const step = steps.get(gate);
      // Only a verdict counts: a gate `skipped` on the base's push run (a
      // pull-request-only gate) or `cancelled` says nothing — rerun it.
      if (!step || !['success', 'failure'].includes(step.conclusion)) return null;
      if (step.conclusion === 'success') return { failed: false, errors: new Set() };
      if (!annotations.has(step.job)) {
        const listed = gh(['api', `repos/${ctx.repository}/check-runs/${step.job}/annotations`, '--paginate'], ctx) ?? [];
        annotations.set(step.job, new Set(listed.map((a) => `${(a.path ?? '').replace(/^\.github$/, '')}\u0000${(a.message ?? '').trim()}`)));
      }
      return { failed: true, errors: annotations.get(step.job) };
    },
  };
}

const buildsNothing = (job) => !job.steps.some((step) => /\bcargo\s/.test(step.run ?? ''));

// ---------------------------------------------------------------------------
// The closure check: the harness's own reading, through its binary.
// ---------------------------------------------------------------------------

async function closureCheck(ctx, snap) {
  const bin = process.env.HARNESS_BIN;
  if (!bin) return { outcome: 'skip', why: 'harness sessions only (HARNESS_BIN unset)' };
  const who = ctx.facts?.file ? ['--facts', ctx.facts.file] : ctx.task ? ['--task', String(ctx.task)] : null;
  if (!who) return { outcome: 'skip', why: 'no task known (no session facts, no pull request naming one)' };
  const dir = worktree(ctx, 'closure', snap.commit);
  const from = git(['merge-base', ctx.baseSha, 'HEAD'], { cwd: ctx.root }).stdout;
  const log = path.join(ctx.home, 'logs', 'closure.log');
  fs.mkdirSync(path.dirname(log), { recursive: true });
  const args = ['architecture-check', ...who, '--tree', dir, '--repo', ctx.root, ...(from ? ['--from', from] : []), '--to', snap.commit];
  const code = await spawnLogged(bin, args, { cwd: ctx.root, env: process.env }, log);
  const said = fs.readFileSync(log, 'utf8').trim();
  if (code === 0) return { outcome: 'ok', said, log };
  if (code === 1) return { outcome: 'FAIL', said, log };
  return { outcome: 'error', said, log, why: `the harness answered ${code}` };
}

// ---------------------------------------------------------------------------
// What is printed.
// ---------------------------------------------------------------------------

const NOISE = [
  /^\s*$/,
  /^\s+(Compiling|Checking|Documenting|Downloaded|Downloading|Updating|Locking|Adding|Fresh|Blocking|Finished|Packaging|Verifying|Archiving) /,
  /^running \d+ tests?$/,
  /^test .* \.\.\. (ok|ignored)$/,
  /^test result: ok\./,
  /^::(group|endgroup)::/,
  / valid$/,
];

function filtered(log) {
  const text = fs.existsSync(log) ? fs.readFileSync(log, 'utf8') : '';
  const kept = text.split('\n').filter((line) => !NOISE.some((noise) => noise.test(line)));
  const lines = kept.length ? kept : text.split('\n');
  const cap = 200;
  return lines.length > cap ? [...lines.slice(0, cap), `… ${lines.length - cap} more line(s)`] : lines;
}

function parseArgs(argv) {
  const options = { only: null, failed: false, rules: false, base: '', head: '', labels: [], offline: false, json: false, list: false, eval: null, number: null };
  for (let n = 0; n < argv.length; n += 1) {
    const arg = argv[n];
    const next = () => {
      n += 1;
      if (n >= argv.length) throw new Error(`${arg} needs a value`);
      return argv[n];
    };
    if (arg === '--only') options.only = next().split(',').map((s) => s.trim()).filter(Boolean);
    else if (arg === '--failed') options.failed = true;
    else if (arg === '--rules') options.rules = true;
    else if (arg === '--base') options.base = next();
    else if (arg === '--head') options.head = next();
    else if (arg === '--label') options.labels.push(next());
    else if (arg === '--no-gh') options.offline = true;
    else if (arg === '--json') options.json = true;
    else if (arg === '--list') options.list = true;
    else if (arg === '--eval') options.eval = next();
    else if (arg === '--number') options.number = Number(next());
    else if (arg === '--workflow') options.workflow = next();
    else if (arg === '-h' || arg === '--help') options.help = true;
    else throw new Error(`unknown argument ${arg}`);
  }
  return options;
}

function list(workflow, options) {
  const rows = [];
  for (const job of workflow.jobs) {
    const off = OFF_BY_DEFAULT[job.id] ?? null;
    const steps = job.steps.filter((step) => step.kind !== 'setup-action' && step.kind !== 'report');
    // A job this script cannot run makes every one of its steps unreadable:
    // a run would report them so, and exit 2.
    for (const step of steps) {
      rows.push({
        job: job.id,
        id: step.id,
        kind: job.unreadable ? 'unreadable' : step.kind,
        why: job.unreadable ?? step.why ?? null,
        off,
      });
    }
    if (job.unreadable && !steps.length) rows.push({ job: job.id, id: job.id, kind: 'unreadable', why: job.unreadable, off });
  }
  if (options.json) console.log(JSON.stringify({ error: workflow.error ?? null, steps: rows }));
  else {
    if (workflow.error) console.log(`unreadable · ${WORKFLOW} · ${workflow.error}`);
    for (const row of rows) console.log(`${row.job} · ${row.id} · ${row.kind}${row.why ? ` · ${row.why}` : ''}${row.off ? ` · off unless ${row.off}` : ''}`);
  }
  return workflow.error || rows.some((row) => row.kind === 'unreadable') ? 2 : 0;
}

function evaluate(options) {
  const ctx = {
    baseRef: (options.base || 'main_agent').replace(/^origin\//, ''),
    headRef: options.head || 'feat/x',
    baseSha: 'base-sha',
    headSha: 'head-sha',
    repository: 'owner/repo',
    workspace: '/work',
    labels: options.labels,
    number: options.number,
    failed: false,
    steps: {},
  };
  const key = normalize(options.eval);
  if (CONDITIONS.has(key)) console.log(JSON.stringify({ kind: 'condition', ...condition(options.eval, ctx) }));
  else {
    const wrapped = options.eval.includes('${{') ? options.eval : `\${{ ${options.eval} }}`;
    console.log(JSON.stringify({ kind: 'value', ...substitute(wrapped, ctx) }));
  }
  return 0;
}

// What `--failed` reruns, from the last report: `null` to run everything,
// `{ nothing }` when nothing is left red, else the red gates and the jobs
// whose setup failed.
function lastRed(reportFile, out) {
  let last = null;
  try {
    last = JSON.parse(fs.readFileSync(reportFile, 'utf8'));
  } catch {
    last = null;
  }
  if (!last) {
    out('gates-local: --failed with no last report — every gate runs');
    return null;
  }
  if (last.conflict) {
    out('gates-local: --failed after a merge conflict — the merge is redone and every gate runs');
    return null;
  }
  const gates = new Set((last.gates ?? []).filter((g) => g.outcome === 'FAIL').map((g) => g.id));
  const jobs = new Set(last.setup_failed ?? []);
  const closure = ['FAIL', 'error'].includes(last.closure);
  if (!gates.size && !jobs.size && !closure) return { nothing: true };
  return { gates, jobs, closure };
}

// A result's line: `ok · gate · 3s`, `skip · gate · why`, … A red gate, until
// the base has judged it, is `red`.
function line(result, ctx) {
  const time = result.seconds !== undefined ? ` · ${result.seconds}s` : '';
  if (result.outcome === 'FAIL' && result.kind === 'gate' && !result.judged) return `red · ${result.id}${time} · judged against the base at the end`;
  if (result.outcome === 'base-red') return `base-red · ${result.id}${time} · red on ${ctx.baseRef} too (${result.base}): not yours to fix`;
  if (result.outcome === 'skip' || result.outcome === 'unreadable') return `${result.outcome} · ${result.id} · ${result.why}`;
  if (result.outcome === 'tolerated') return `tolerated · ${result.id}${time} · ${result.why}`;
  return `${result.outcome} · ${result.id}${time}`;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    const lines = fs.readFileSync(new URL(import.meta.url), 'utf8').split('\n').slice(1);
    const header = lines.slice(0, lines.findIndex((l) => l.startsWith('import ')));
    console.log(header.filter((l) => l.startsWith('//')).map((l) => l.slice(3)).join('\n'));
    return 0;
  }
  if (options.eval !== null) return evaluate(options);
  const root = git(['rev-parse', '--show-toplevel']).stdout || process.cwd();
  const localFile = options.workflow ? path.resolve(options.workflow) : path.join(root, WORKFLOW);
  if (options.list) {
    if (!fs.existsSync(localFile)) {
      console.log(`gates-local: no ${WORKFLOW} in this checkout — nothing to list`);
      return 0;
    }
    return list(readWorkflow(fs.readFileSync(localFile, 'utf8')), options);
  }
  const out = options.json ? (text) => process.stderr.write(`${text}\n`) : (text) => console.log(text);
  const started = Date.now();
  const ctx = context(options);
  reap(ctx.home);
  const reportFile = path.join(ctx.home, 'report.json');

  let wanted = (id) => !options.only || options.only.some((only) => id === only || id.startsWith(`${only}--`));
  let partial = options.only ? `--only ${options.only.join(',')}` : null;
  if (options.failed && !options.only) {
    const red = lastRed(reportFile, out);
    if (red?.nothing) {
      out('gates-local: the last report has no red gate, no failed setup, and its closure check held — nothing to rerun');
      return 0;
    }
    if (red) {
      wanted = (id, job) => red.gates.has(id) || red.jobs.has(job);
      partial = '--failed';
    }
  }

  const snap = snapshot(ctx);
  const merged = mergedWithBase(ctx, snap);
  const shownUntracked = snap.untracked.slice(0, 8).join(', ') + (snap.untracked.length > 8 ? ', …' : '');
  out(
    `gates-local: ${ctx.headRef} -> ${ctx.baseRef} @${ctx.baseSha.slice(0, 7)} · work ${snap.tree.slice(0, 7)}` +
      (snap.untracked.length ? ` · ${snap.untracked.length} untracked file(s) included: ${shownUntracked}` : ''),
  );
  if (ctx.fetchFailed) out(`gates-local: fetching origin/${ctx.baseRef} failed — its tip as last fetched`);
  if (merged.note) out(`gates-local: ${merged.note}`);
  const report = {
    head: ctx.head,
    head_tree: snap.tree,
    base: ctx.baseRef,
    base_sha: ctx.baseSha,
    started: new Date(started).toISOString(),
    partial,
    gates: [],
    setup_failed: [],
    closure: 'skip',
    conflict: null,
  };
  if (merged.conflict) {
    out(`FAIL · merge · your work does not merge into ${ctx.baseRef} (the CI does not run on a conflicting PR): ${merged.conflict.join(', ')}`);
    report.conflict = merged.conflict;
    report.seconds = Math.round((Date.now() - started) / 1000);
    fs.writeFileSync(reportFile, JSON.stringify(report, null, 2));
    return 1;
  }

  // The workflow the CI runs: the merged tree's, never the one the branch
  // was cut with.
  let text;
  if (options.workflow) text = fs.readFileSync(localFile, 'utf8');
  else {
    const shown = git(['show', `${merged.commit}:${WORKFLOW}`], { cwd: ctx.root, raw: true });
    if (!shown.ok) {
      out(`gates-local: no ${WORKFLOW} in the merged tree — nothing to run`);
      return 0;
    }
    text = shown.stdout;
  }
  const workflow = readWorkflow(text);
  if (workflow.error) out(`unreadable · ${WORKFLOW} · ${workflow.error}`);

  const gateSteps = (job) => job.steps.filter((step) => !['setup-action', 'report', 'setup'].includes(step.kind));
  const jobs = workflow.jobs.filter((job) => {
    const flag = OFF_BY_DEFAULT[job.id];
    if (flag === '--rules' && !options.rules) return false;
    return gateSteps(job).some((step) => wanted(step.id, job.id)) || (job.unreadable && !partial);
  });
  const leftOff = workflow.jobs.filter((job) => OFF_BY_DEFAULT[job.id] === '--rules' && !options.rules);
  const closure = !options.only ? closureCheck(ctx, snap) : Promise.resolve({ outcome: 'skip', why: 'not with --only' });
  const runs = await Promise.all(
    jobs.map(async (job) => {
      const begun = Date.now();
      let rows;
      if (job.unreadable) {
        const unread = gateSteps(job).filter((step) => wanted(step.id, job.id) || !partial);
        rows = (unread.length ? unread : [{ id: job.id }]).map((step) => ({ id: step.id, job: job.id, kind: 'gate', outcome: 'unreadable', why: job.unreadable }));
      } else {
        out(`gates-local: job ${job.id} running — ${gateSteps(job).filter((step) => wanted(step.id, job.id)).length} gate(s)…`);
        const dir = worktree(ctx, `head-${job.id}`, merged.commit);
        rows = await runJob(ctx, job, { dir, side: 'head', wanted, merge: merged.commit });
      }
      // Said as the job ends, grouped: a session sees progress, not minutes of silence.
      out(`gates-local: job ${job.id} done · ${Math.round((Date.now() - begun) / 1000)}s`);
      for (const result of rows) if (!(result.kind === 'setup' && result.outcome === 'ok')) out(`  ${line(result, ctx)}`);
      return rows;
    }),
  );
  const results = runs.flat();

  // The base's verdict on what is red here.
  const red = results.filter((r) => r.outcome === 'FAIL' && r.kind === 'gate');
  if (red.length) {
    const fromGithub = baseRun(ctx);
    const rerun = [];
    for (const result of red) {
      const head = errorsOf(fs.readFileSync(result.log, 'utf8'));
      result.errors = head;
      const known = fromGithub?.verdict(result.id);
      if (known) {
        if (isBaseRed(head, known)) result.outcome = 'base-red';
        result.base = `gates run ${fromGithub.id}`;
      } else {
        rerun.push(result);
      }
    }
    const cheap = new Set(jobs.filter(buildsNothing).map((job) => job.id));
    const byJob = new Map();
    for (const result of rerun.filter((r) => cheap.has(r.job))) byJob.set(result.job, [...(byJob.get(result.job) ?? []), result.id]);
    if (byJob.size) {
      const baseRuns = await Promise.all(
        [...byJob.entries()].map(([id, gates]) => {
          const job = jobs.find((j) => j.id === id);
          const dir = worktree(ctx, `base-${id}`, ctx.baseSha);
          const wantedHere = (gate) => gates.includes(gate);
          return runJob({ ...ctx, headRef: ctx.headRef }, job, { dir, side: 'base', wanted: wantedHere, merge: ctx.baseSha });
        }),
      );
      for (const base of baseRuns.flat()) {
        const result = rerun.find((r) => r.id === base.id);
        if (!result || !['ok', 'FAIL'].includes(base.outcome)) continue;
        const verdict = { failed: base.outcome === 'FAIL', errors: base.outcome === 'FAIL' ? errorsOf(fs.readFileSync(base.log, 'utf8')) : new Set() };
        if (isBaseRed(result.errors, verdict)) result.outcome = 'base-red';
        result.base = 'rerun on the base';
      }
    }
    out(`gates-local: the red gate(s) judged against ${ctx.baseRef}:`);
    for (const result of red) {
      result.judged = true;
      out(`  ${line(result, ctx)}`);
    }
  }

  for (const job of leftOff) out(`skip · ${job.id} (job) · off — ${OFF_BY_DEFAULT[job.id]} runs it`);
  for (const result of results.filter((r) => r.outcome === 'FAIL')) {
    out(`\n--- ${result.id} (exit ${result.code}) ---`);
    for (const kept of filtered(result.log)) out(kept);
    out(`--- full log: ${result.log}`);
  }

  const closed = await closure;
  report.closure = closed.outcome;
  if (closed.outcome === 'skip') out(`skip · closure · ${closed.why}`);
  else {
    out(`${closed.outcome} · closure${closed.why ? ` · ${closed.why}` : ''}`);
    if (closed.outcome !== 'ok' && closed.said) for (const said of closed.said.split('\n')) out(`  ${said}`);
  }

  const count = (outcome) => results.filter((r) => r.kind !== 'setup' && r.outcome === outcome).length;
  const setupFailed = [...new Set(results.filter((r) => r.kind === 'setup' && r.outcome === 'FAIL').map((r) => r.job))];
  const seconds = Math.round((Date.now() - started) / 1000);
  report.seconds = seconds;
  report.gates = results.filter((r) => r.kind !== 'setup').map((r) => ({ id: r.id, job: r.job, outcome: r.outcome, seconds: r.seconds ?? null, why: r.why ?? null }));
  report.setup_failed = setupFailed;
  report.counts = {
    ok: count('ok'),
    FAIL: count('FAIL'),
    'base-red': count('base-red'),
    skip: count('skip'),
    unreadable: count('unreadable') + (workflow.error ? 1 : 0),
    tolerated: count('tolerated'),
    'closure-error': closed.outcome === 'error' ? 1 : 0,
  };
  fs.writeFileSync(reportFile, JSON.stringify(report, null, 2));
  out(
    `gates-local: ${report.counts.ok} ok, ${report.counts.FAIL} FAIL, ${report.counts['base-red']} base-red, ${report.counts.skip} skipped, ` +
      `${report.counts.unreadable} unreadable · closure ${closed.outcome}${partial ? ` · partial (${partial})` : ''} · ${seconds}s · ${path.relative(ctx.root, reportFile)}`,
  );
  if (options.json) console.log(JSON.stringify(report));
  if (report.counts.FAIL || setupFailed.length || closed.outcome === 'FAIL') return 1;
  return report.counts.unreadable || closed.outcome === 'error' ? 2 : 0;
}

main().then(
  (code) => process.exit(code),
  (error) => {
    console.error(`gates-local: ${error.message}`);
    process.exit(3);
  },
);
