#!/usr/bin/env node
// Skill evals: replay real factory prompts against the harness's skills and
// score what the session answers with deterministic checks.
//
//   node scripts/skill-evals.mjs harvest <factory run folder> [--name <n>] [--expect-stop] [--repo <owner/name>]
//   node scripts/skill-evals.mjs run [<skill>...] [--changed] [--reps 3] [--jobs 3]
//                                    [--label <l>] [--skills-from <git ref>] [--mirror <bare clone>]
//   node scripts/skill-evals.mjs observe <factory> [--since <date>] [--until <date>] [--label <l>]
//   node scripts/skill-evals.mjs report <label> [<other label>]
//   node scripts/skill-evals.mjs list
//
// A case is a real prompt the factory sent (`evals/cases/<skill>/<name>/`),
// the product's commit it was sent on, and what the real outcome was. `run`
// replays it in a clone at that commit, with the skills of this working tree
// (or of `--skills-from`) lent the way the launcher lends them, and keeps
// each session's stream under `.llocal/evals/runs/<label>/`. `report` scores
// the streams. Compare a skill change by running twice:
//
//   node scripts/skill-evals.mjs run code --label before --skills-from HEAD
//   node scripts/skill-evals.mjs run code --label after
//   node scripts/skill-evals.mjs report before after
//
// The replay cannot act: its tools are Bash, Read and Skill only, the CLI
// denies any command it does not know to be read-only, `gh` is a stub that
// fails, `origin` points nowhere, and the clone keeps no ref past the case's
// commit — otherwise a session reads the board, finds its task delivered,
// and stops. Every run spends real tokens: `report` shows the cost.
// `observe` needs none of that and spends nothing: it scores the rounds a
// factory has really run, from their streams in `.llocal/logs`, the build
// half of a code round included. It compares windows rather than the same
// task twice, so it needs more rounds to conclude:
//
//   node scripts/skill-evals.mjs observe ~/.harness/factories/<f> --until 2026-10-26 --label before
//   node scripts/skill-evals.mjs observe ~/.harness/factories/<f> --since 2026-10-26 --label after
//   node scripts/skill-evals.mjs report before after
//
// Needs node 22+, git, the `claude` CLI, and `gh` for `harvest` only.

import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CASES = path.join(ROOT, 'evals', 'cases');
const WORK = path.join(ROOT, '.llocal', 'evals');
const SKILLS = '.claude/skills';
const TIMEOUT_MS = 30 * 60 * 1000;

// The launcher's own flags (crates/adapters/src/agent/claude_cli.rs: LEAN,
// NO_SCHEDULING), so a replay is billed and cached like a real session.
const LEAN = ['--strict-mcp-config', '--setting-sources', 'project,local', '--exclude-dynamic-system-prompt-sections'];
const NO_SCHEDULING = ['--disallowedTools', 'ScheduleWakeup,CronCreate,CronDelete,CronList,RemoteTrigger,Monitor,Bash(gh pr merge:*)'];
// Narrower than the launcher's `Bash,Read,Edit,Write,Skill`: a replay never writes.
const READ_ONLY_TOOLS = ['--tools', 'Bash,Read,Skill'];

const UNITS = ['concept', 'aggregate', 'data-capability', 'infrastructure', 'capability', 'persisted-query', 'micro-ui', 'composition', 'tooling'];
const EFFECTS = ['insert', 'update', 'delete', 'upsert'];
const BRANCH = /^(feat|fix|docs|refactor|test|chore|AIchore)\/[a-z0-9][a-z0-9-]*$/;

// ---- small helpers -------------------------------------------------------

function die(message) {
  console.error(`skill-evals: ${message}`);
  process.exit(1);
}

function git(dir, args, opts = {}) {
  return execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...opts }).trim();
}

function parseArgs(argv) {
  const positional = [];
  const flags = {};
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a.startsWith('--')) {
      const key = a.slice(2);
      const next = argv[i + 1];
      if (next === undefined || next.startsWith('--')) flags[key] = true;
      else { flags[key] = next; i += 1; }
    } else positional.push(a);
  }
  return { positional, flags };
}

// A label, a case name or a skill is a plain name: never a path.
export function plainName(value, what) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9._-]+$/.test(value) || value === '.' || value === '..') {
    die(`${what} must be a plain name (letters, digits, . _ -), got ${JSON.stringify(value)}`);
  }
  return value;
}

function fullCommit(value) {
  if (typeof value !== 'string' || !/^[0-9a-f]{40}$/.test(value)) die(`a case commit must be a full sha, got ${JSON.stringify(value)}`);
  return value;
}

// A path this script removes or rewrites: always under `.llocal/evals`.
function inWork(target) {
  const resolved = path.resolve(target);
  if (!resolved.startsWith(`${WORK}${path.sep}`)) die(`refusing to touch ${resolved}, outside ${WORK}`);
  return resolved;
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function slug(text) {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 40);
}

// The first turn of a run's `prompts.md`: its header and its fenced text.
// The prompt is the turn's section up to the next turn, its outer fence
// stripped: a prompt may hold fences of its own (a split ends inside one).
export function firstTurn(markdown) {
  const start = markdown.indexOf('## turn 1 ');
  if (start < 0) return null;
  const next = markdown.indexOf('\n## turn 2 ', start);
  const section = markdown.slice(start, next < 0 ? undefined : next);
  const header = section.slice('## turn 1 '.length, section.indexOf('\n'));
  const body = section.slice(section.indexOf('\n') + 1).replace(/^\s*```[^\n]*\n/, '').replace(/\n?```\s*$/, '');
  if (body.trim().length === 0) return null;
  const model = header.match(/\((\w+)\/(\w+)\)/);
  return { header, prompt: body, model: model?.[1] ?? 'sonnet', effort: model?.[2] ?? 'high' };
}

// ---- cases ---------------------------------------------------------------

function loadCases() {
  const cases = [];
  if (!fs.existsSync(CASES)) return cases;
  for (const skill of fs.readdirSync(CASES)) {
    const dir = path.join(CASES, skill);
    if (!fs.statSync(dir).isDirectory()) continue;
    for (const name of fs.readdirSync(dir)) {
      const at = path.join(dir, name);
      if (!fs.existsSync(path.join(at, 'case.json'))) continue;
      const kase = readJson(path.join(at, 'case.json'));
      cases.push({ ...kase, name, commit: fullCommit(kase.commit), skills: kase.skills ?? [kase.skill], prompt: fs.readFileSync(path.join(at, 'prompt.txt'), 'utf8') });
    }
  }
  return cases.sort((a, b) => `${a.skill}/${a.name}`.localeCompare(`${b.skill}/${b.name}`));
}

// The product a factory builds: the remote of one of its lane clones.
function productRepo(factory) {
  const lanes = path.join(factory, '.llocal', 'agentic_workspaces');
  if (!fs.existsSync(lanes)) return null;
  for (const lane of fs.readdirSync(lanes)) {
    try {
      const url = git(path.join(lanes, lane), ['remote', 'get-url', 'origin']);
      const m = url.match(/github\.com[:/]([^/]+\/[^/.]+)(\.git)?$/);
      if (m) return m[1];
    } catch { /* not a clone */ }
  }
  return null;
}

function factoryMirror(factory) {
  const at = path.join(factory, '.llocal', 'view-mirror');
  if (!fs.existsSync(at)) return null;
  const bare = fs.readdirSync(at).find((n) => n.endsWith('.git'));
  return bare ? path.join(at, bare) : null;
}

function harvest(runDir, flags) {
  const run = path.resolve(runDir);
  const workflow = path.basename(path.dirname(run));
  const factory = run.split(`${path.sep}.llocal${path.sep}`)[0];
  const turn = firstTurn(fs.readFileSync(path.join(run, 'prompts.md'), 'utf8'));
  if (!turn) die(`${run}/prompts.md holds no first turn`);
  const repo = flags.repo || productRepo(factory) || die('no product repository found; pass --repo <owner/name>');
  const mirror = factoryMirror(factory) || die(`no mirror under ${factory}/.llocal/view-mirror to resolve commits`);
  const base = { repo: `https://github.com/${repo}.git`, model: turn.model, effort: turn.effort, source: `${path.basename(factory)}/${workflow}/${path.basename(run)}` };
  let kase;
  let name;
  if (workflow === 'agent-loop') {
    if (!turn.header.includes('/tech-analyst')) die('this round did not open with /tech-analyst, the code stage\'s lead');
    const facts = readJson(path.join(run, 'session-facts.json'));
    if ((facts.skeleton ?? []).length > 0) {
      die('this round started from a skeleton the harness wrote uncommitted: its files are in no commit, so it cannot be replayed');
    }
    const files = JSON.parse(execFileSync('gh', ['pr', 'list', '--repo', repo, '--head', facts.branch, '--state', 'merged', '--json', 'files', '--limit', '1'], { encoding: 'utf8' }));
    const truth = (files[0]?.files ?? []).map((f) => f.path).filter((p) => p !== 'Cargo.lock');
    if (truth.length === 0) die(`no merged pull request for ${facts.branch}: the case needs the files the real round changed`);
    kase = { skill: 'code', skills: ['tech-analyst', 'code', 'create-test'], ...base, task: Number(facts.task), commit: git(mirror, ['rev-parse', facts.round_base]), branch: String(facts.base).replace(/^origin\//, ''), expect_stop: Boolean(flags['expect-stop']), truth: { files: truth } };
    name = `${facts.task}-${slug(facts.title ?? facts.branch)}`;
  } else if (workflow === 'split') {
    const log = fs.readFileSync(path.join(run, 'run.log'), 'utf8');
    const milestone = Number(path.basename(run).split('-').pop());
    const branch = log.match(/back on origin\/([^\s,]+)/)?.[1] ?? 'main_agent';
    const started = log.match(/^\[([0-9TZ:-]+)\]/)?.[1] ?? die('run.log carries no timestamp');
    const commit = git(mirror, ['rev-list', '-1', `--before=${started}`, `refs/heads/${branch}`]) || die(`no commit of ${branch} before ${started}`);
    const slices = (log.match(/opened task #/g) ?? []).length;
    kase = { skill: 'split', skills: ['split'], ...base, milestone, commit, branch, expect_stop: Boolean(flags['expect-stop']), truth: { slices } };
    name = `m${milestone}-${slug(log.match(/splitting #\d+ — (.*)/)?.[1] ?? 'milestone')}`;
  } else {
    die(`no eval for a ${workflow} run yet (agent-loop and split are harvested)`);
  }
  name = plainName(flags.name || name, 'a case name');
  const out = path.join(CASES, kase.skill, name);
  fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(path.join(out, 'case.json'), `${JSON.stringify(kase, null, 2)}\n`);
  fs.writeFileSync(path.join(out, 'prompt.txt'), turn.prompt);
  console.log(`harvested ${path.relative(ROOT, out)} (${kase.skill}, commit ${kase.commit.slice(0, 8)})`);
}

// ---- replay --------------------------------------------------------------

// The skills a run lends: this working tree's, or a git ref's.
function skillsSource(ref, label) {
  if (!ref) return path.join(ROOT, SKILLS);
  if (!/^[\w./~^-]+$/.test(ref) || ref.startsWith('-')) die(`--skills-from takes a git ref, got ${JSON.stringify(ref)}`);
  const at = inWork(path.join(WORK, 'skills', label));
  fs.rmSync(at, { recursive: true, force: true });
  fs.mkdirSync(at, { recursive: true });
  const tar = execFileSync('git', ['-C', ROOT, 'archive', ref, SKILLS]);
  execFileSync('tar', ['-x', '-C', at], { input: tar });
  return path.join(at, SKILLS);
}

function ghStub() {
  const dir = path.join(WORK, 'shim');
  fs.mkdirSync(dir, { recursive: true });
  const gh = path.join(dir, 'gh');
  fs.writeFileSync(gh, '#!/bin/sh\necho "gh: error connecting to api.github.com (no network in this replay)" >&2\nexit 1\n');
  fs.chmodSync(gh, 0o755);
  // A `gh` reached by its full path finds no login here either.
  fs.mkdirSync(path.join(dir, 'gh-config'), { recursive: true });
  return dir;
}

// The replay's environment: the stub first on PATH, no GitHub credential.
function replayEnv(shim) {
  const env = { ...process.env, PATH: `${shim}:${process.env.PATH}`, GH_CONFIG_DIR: path.join(shim, 'gh-config') };
  for (const key of ['GH_TOKEN', 'GITHUB_TOKEN', 'GH_ENTERPRISE_TOKEN', 'GITHUB_ENTERPRISE_TOKEN']) delete env[key];
  return env;
}

// A clone of the product at the case's commit, as a lane holds it then: the
// integration branch at that commit and nothing newer, the skills lent.
// Built whole or not at all: a `ready` marker is written last, and a clone
// without it is rebuilt — a half-made one would keep every ref and its origin.
function prepareClone(kase, label, skills, mirror) {
  const dir = inWork(path.join(WORK, 'clones', label, `${kase.commit.slice(0, 12)}-${slug(kase.branch)}`));
  const ready = path.join(dir, '.git', 'evals-ready');
  if (!fs.existsSync(ready)) {
    fs.rmSync(dir, { recursive: true, force: true });
    fs.mkdirSync(path.dirname(dir), { recursive: true });
    execFileSync('git', ['clone', '--quiet', mirror || kase.repo, dir], { stdio: 'ignore' });
    git(dir, ['checkout', '--quiet', '--detach', kase.commit]);
    for (const ref of git(dir, ['for-each-ref', '--format=%(refname)']).split('\n').filter(Boolean)) {
      git(dir, ['update-ref', '-d', ref]);
    }
    git(dir, ['update-ref', `refs/remotes/origin/${kase.branch}`, kase.commit]);
    git(dir, ['remote', 'set-url', 'origin', 'file:///nonexistent/replay.git']);
    // The reflog still names the newer tip and where it was cloned from.
    git(dir, ['reflog', 'expire', '--expire=now', '--all']);
    git(dir, ['gc', '--quiet', '--prune=now']);
    fs.appendFileSync(path.join(dir, '.git', 'info', 'exclude'), `${SKILLS}/\n`);
    fs.writeFileSync(ready, `${kase.commit}\n`);
  }
  const lent = path.join(dir, SKILLS);
  fs.rmSync(lent, { recursive: true, force: true });
  fs.mkdirSync(path.dirname(lent), { recursive: true });
  fs.cpSync(skills, lent, { recursive: true });
  return dir;
}

// What a code case is told on top of its real prompt: the replay cannot
// build, so it is judged on its plan, and a stop for that reason is no stop.
export const REPLAY_NOTE = `

--- REPLAY (skill evals) ---
This is a read-only replay: nothing can be edited, committed or pushed, and
\`gh\` has no network. Do the plan half only — the pre-flight gate, the ordered
checklist against the real code, the stop line, the risks — and end there.
Being unable to build is expected here, not a reason for AGENT_LOOP_STOP; stop
only for what would stop the real round.
--- END REPLAY ---
`;

function replay(kase, clone, shim, out) {
  const prompt = kase.skill === 'code' ? `${kase.prompt}${REPLAY_NOTE}` : kase.prompt;
  const args = ['-p', '--output-format', 'stream-json', '--verbose', '--model', kase.model, '--effort', kase.effort,
    '--permission-mode', 'default', ...LEAN, ...NO_SCHEDULING, ...READ_ONLY_TOOLS, '--', prompt];
  return new Promise((resolve) => {
    const sink = fs.openSync(out, 'w');
    const child = spawn('claude', args, { cwd: clone, env: replayEnv(shim), stdio: ['ignore', sink, sink], detached: true });
    let killer;
    const timer = setTimeout(() => {
      // The whole process group: the session's own children die with it.
      try { process.kill(-child.pid, 'SIGTERM'); } catch { /* already gone */ }
      killer = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch { /* gone */ } }, 10_000);
    }, TIMEOUT_MS);
    const done = (code) => { clearTimeout(timer); clearTimeout(killer); fs.closeSync(sink); resolve(code); };
    child.on('error', (error) => { console.error(`skill-evals: cannot start claude: ${error.message}`); done(-1); });
    child.on('close', done);
  });
}

// The skills a `git status --porcelain -z` names: a rename counts by its new path.
export function skillsInStatus(text) {
  const entries = text.split('\0');
  const skills = new Set();
  for (let i = 0; i < entries.length; i += 1) {
    const entry = entries[i];
    if (entry.length < 4) continue;
    const file = entry.slice(3);
    if (entry[0] === 'R' || entry[0] === 'C') i += 1;
    const parts = file.split('/');
    if (parts[0] === '.claude' && parts[1] === 'skills' && parts[2]) skills.add(parts[2]);
  }
  return [...skills];
}

function changedSkills() {
  return skillsInStatus(execFileSync('git', ['-C', ROOT, 'status', '--porcelain', '-z', '--', SKILLS], { encoding: 'utf8' }));
}

async function run(positional, flags) {
  const label = plainName(flags.label || 'working-tree', 'a label');
  const reps = Number(flags.reps || 3);
  const jobs = Number(flags.jobs || 3);
  if (!(Number.isInteger(reps) && reps >= 1 && Number.isInteger(jobs) && jobs >= 1)) die('--reps and --jobs take a whole number of at least 1');
  const wanted = flags.changed ? changedSkills() : positional;
  const cases = loadCases().filter((c) => wanted.length === 0 || wanted.includes(c.skill) || c.skills.some((s) => wanted.includes(s)));
  if (cases.length === 0) die(flags.changed ? 'no changed skill has a case' : 'no case matches');
  const skills = skillsSource(flags['skills-from'], label);
  const shim = ghStub();
  const outDir = path.join(WORK, 'runs', label);
  fs.mkdirSync(outDir, { recursive: true });
  const queue = [];
  for (const kase of cases) {
    const clone = prepareClone(kase, label, skills, flags.mirror || process.env.SKILL_EVALS_MIRROR);
    for (let rep = 1; rep <= reps; rep += 1) {
      const out = path.join(outDir, `${kase.skill}--${kase.name}--${rep}.jsonl`);
      if (fs.existsSync(out) && fs.readFileSync(out, 'utf8').includes('"type":"result"')) continue;
      queue.push({ kase, clone, out, rep });
    }
  }
  console.log(`skill-evals: ${queue.length} session(s) to replay under "${label}", ${jobs} at a time`);
  const worker = async () => {
    for (let job = queue.shift(); job; job = queue.shift()) {
      const started = Date.now();
      const code = await replay(job.kase, job.clone, shim, job.out);
      console.log(`  ${job.kase.skill}/${job.kase.name} #${job.rep}: exit ${code}, ${Math.round((Date.now() - started) / 1000)}s`);
    }
  };
  await Promise.all(Array.from({ length: jobs }, worker));
  report([label]);
}

// ---- scoring -------------------------------------------------------------

export function parseStream(text) {
  const out = { result: null, answer: '', final: '', denied: [], commands: [] };
  const commands = new Map();
  const texts = [];
  for (const line of text.split('\n')) {
    let e;
    try { e = JSON.parse(line); } catch { continue; }
    if (!e || typeof e !== 'object') continue;
    if (e.type === 'result') out.result = e;
    const content = Array.isArray(e.message?.content) ? e.message.content : [];
    for (const b of content) {
      if (e.type === 'assistant' && b.type === 'text') texts.push(b.text);
      if (b.type === 'tool_use') {
        commands.set(b.id, String(b.input?.command ?? b.input?.file_path ?? b.name));
        if (b.name === 'Bash') out.commands.push(String(b.input?.command ?? ''));
      }
      if (b.type === 'tool_result' && b.is_error && /requires approval/i.test(JSON.stringify(b.content ?? ''))) {
        out.denied.push(commands.get(b.tool_use_id) ?? '?');
      }
    }
  }
  out.final = String(out.result?.result ?? '');
  // Everything the session said: a plan may sit in an earlier message than the last.
  out.answer = [...texts, out.final].join('\n');
  return out;
}

// A file is found when the answer names its path, or at least `parent/name`.
export function found(answer, file) {
  if (answer.includes(file)) return true;
  const parts = file.split('/');
  return parts.length > 1 && answer.includes(parts.slice(-2).join('/'));
}

// The simple commands a shell line runs: quoted text dropped, split on
// `&&`, `||`, `;`, `|` and newlines — so a grep for a pattern is no use of it.
export function simpleCommands(line) {
  const unquoted = line.replace(/'[^']*'/g, "''").replace(/"(?:\\.|[^"\\])*"/g, '""');
  // A leading `VAR=value`, `env`, `time` or `timeout N` is no command of its own.
  const prefix = /^(?:\w+=\S*\s+|env\s+|time\s+|timeout\s+\S+\s+)+/;
  return unquoted.split(/&&|\|\||;|\||\n/).map((c) => c.trim().replace(prefix, '')).filter(Boolean);
}

const runs = (commands, pattern) => commands.flatMap(simpleCommands).filter((c) => pattern.test(c));
const BULK_ADD = /^git(\s+-C\s+\S+)?\s+(add\s+(-A|--all|\.)(\s|$)|commit\s+(-[A-Za-z]*a[A-Za-z]*)(\s|$))/;
const SELF_MERGE = /^gh\s+pr\s+merge(\s|$)/;

// The JSON array a split answers: its last fenced json block, else its outermost brackets.
export function slicesOf(answer) {
  const fenced = [...answer.matchAll(/```json\s*\n([\s\S]*?)```/g)].at(-1)?.[1];
  const text = fenced ?? answer.slice(answer.indexOf('['), answer.lastIndexOf(']') + 1);
  try {
    const value = JSON.parse(text);
    // A slice that is not an object is kept as an empty one: it fails the shape checks.
    return Array.isArray(value) ? value.map((s) => (s && typeof s === 'object' && !Array.isArray(s) ? s : {})) : null;
  } catch { return null; }
}

const dependsOn = (s) => (Array.isArray(s.depends_on) ? s.depends_on : s.depends_on == null ? [] : [Number.NaN]);

function acyclic(slices) {
  const state = new Array(slices.length).fill(0);
  const visit = (i) => {
    if (state[i] === 1) return false;
    if (state[i] === 2) return true;
    state[i] = 1;
    const ok = dependsOn(slices[i]).every((j) => visit(j));
    state[i] = 2;
    return ok;
  };
  return slices.every((_, i) => visit(i));
}

const filled = (v) => typeof v === 'string' && v.trim().length > 0;

// Each check answers [passed, detail]; the detail says why when it failed.
export const CHECKS = {
  code: {
    'pre-flight reported': (r) => {
      const topics = [/block/i, /working tree|git status/i, /assumption|stale/i, /missing input/i];
      const hit = topics.filter((t) => t.test(r.answer)).length;
      return [hit >= 3, `${hit}/4 findings`];
    },
    'files found': (r, k) => {
      const files = k.truth.files ?? [];
      const hit = files.filter((f) => found(r.answer, f)).length;
      return [files.length > 0 && hit / files.length >= 0.8, `${hit}/${files.length}`];
    },
    'commands that prove': (r) => {
      const n = (r.answer.match(/`(cargo|npm|npx|node|bash|pnpm|git) [^`]+`/g) ?? []).length;
      return [n >= 2, `${n} command(s)`];
    },
    'stop line restated': (r) => [/stop line/i.test(r.answer), ''],
    'stops only when expected': (r, k) => [/AGENT_LOOP_STOP:/.test(r.answer) === Boolean(k.expect_stop), k.expect_stop ? 'expected a stop' : 'unexpected stop'],
  },
  // A code round the factory really ran: the plan half, then the build.
  'code (observed)': {
    'pre-flight reported': (r) => CHECKS.code['pre-flight reported'](r),
    'stop line restated': (r) => CHECKS.code['stop line restated'](r),
    'staged by name': (r) => {
      const bulk = runs(r.commands, BULK_ADD);
      return [bulk.length === 0, bulk.slice(0, 2).join(' | ')];
    },
    'verification run': (r) => {
      // A round that stopped before building had nothing to verify.
      if (!/AGENT_LOOP_OK:/.test(r.final)) return [true, 'stopped before building'];
      const ran = runs(r.commands, /^(cargo\s+(test|nextest)|npm\s+(run\s+)?test|npx\s+(vitest|playwright)|node\s+--test|bash\s+\S*gates-local)/);
      return [ran.length > 0, `${ran.length} run(s)`];
    },
    'never merges itself': (r) => {
      const merges = runs(r.commands, SELF_MERGE);
      return [merges.length === 0, merges.slice(0, 1).join('')];
    },
    'ends on a marker': (r) => [/AGENT_LOOP_(OK|STOP):/.test(r.final), ''],
  },
  split: {
    'stays read-only': (r) => [r.denied.length === 0, r.denied.slice(0, 3).join(' | ')],
    'answer parses': (r) => [Boolean(r.slices?.length), r.slices ? `${r.slices.length} slice(s)` : 'no JSON array'],
    'slices well formed': (r) => {
      const bad = (r.slices ?? []).filter((s) => !(filled(s.title) && filled(s.brief) && s.brief.length >= 40 && BRANCH.test(s.branch ?? '')
        && typeof s.needs_human === 'boolean' && UNITS.includes(s.unit) && filled(s.system) && Array.isArray(s.produces ?? [])));
      return [Boolean(r.slices?.length) && bad.length === 0, bad.map((s) => s.branch ?? s.title).slice(0, 3).join(', ')];
    },
    'data layer complete': (r) => {
      const bad = (r.slices ?? []).filter((s) => (s.unit === 'data-capability' && !(EFFECTS.includes(s.effect) && filled(s.aggregate)))
        || (s.unit === 'aggregate' && !filled(s.aggregate)));
      return [Boolean(r.slices?.length) && bad.length === 0, bad.map((s) => s.branch).join(', ')];
    },
    'carries a spec': (r) => {
      const bad = (r.slices ?? []).filter((s) => !(filled(s.goal) && Array.isArray(s.acceptance) && s.acceptance.length > 0 && filled(s.technical)));
      return [Boolean(r.slices?.length) && bad.length === 0, bad.map((s) => s.branch).join(', ')];
    },
    'dependencies acyclic': (r) => {
      const s = r.slices ?? [];
      const inRange = s.every((x, i) => dependsOn(x).every((j) => Number.isInteger(j) && j >= 0 && j < s.length && j !== i));
      return [s.length > 0 && inRange && acyclic(s), inRange ? '' : 'an index out of range'];
    },
    'unique branches': (r) => {
      const branches = (r.slices ?? []).map((s) => s.branch);
      return [branches.length > 0 && new Set(branches).size === branches.length, ''];
    },
    'slice count close': (r, k) => {
      const n = r.slices?.length ?? 0;
      const t = k.truth.slices ?? 0;
      return [t > 0 && Math.abs(n - t) <= Math.max(1, Math.round(t * 0.4)), `${n} vs ${t} in the factory`];
    },
  },
};

export function score(kase, text) {
  const r = parseStream(text);
  r.slices = kase.skill === 'split' ? slicesOf(r.final) : null;
  const finished = Boolean(r.result) && !r.result.is_error;
  const checks = { finished: [finished, finished ? '' : 'no result'] };
  for (const [name, check] of Object.entries(CHECKS[kase.skill] ?? {})) {
    let verdict;
    try { verdict = finished ? check(r, kase) : [false, 'not finished']; } catch (error) { verdict = [false, `the check threw: ${error.message}`]; }
    checks[name] = verdict;
  }
  return { checks, cost: Number(r.result?.total_cost_usd ?? 0), turns: Number(r.result?.num_turns ?? 0) };
}

// One factory run folder's id as a time, for the --since/--until windows.
function runTime(folder) {
  const m = folder.match(/^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})/);
  // The factory names its run folders in UTC, as its run.log stamps them.
  return m ? Date.parse(`${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:${m[6]}Z`) : Number.NaN;
}

function observe(factory, flags) {
  const logs = path.join(path.resolve(factory), '.llocal', 'logs');
  if (!fs.existsSync(logs)) die(`no ${logs}`);
  const bound = (value, name) => {
    if (value === undefined) return undefined;
    const t = typeof value === 'string' ? Date.parse(value) : Number.NaN;
    if (Number.isNaN(t)) die(`--${name} takes a date (2026-10-26 or 2026-10-26T12:00Z), got ${JSON.stringify(value)}`);
    return t;
  };
  const since = bound(flags.since, 'since') ?? -Infinity;
  const until = bound(flags.until, 'until') ?? Infinity;
  const label = plainName(flags.label || 'observed', 'a label');
  const rows = [];
  for (const [workflow, skill] of [['agent-loop', 'code (observed)'], ['split', 'split']]) {
    const dir = path.join(logs, workflow);
    if (!fs.existsSync(dir)) continue;
    for (const run of fs.readdirSync(dir).sort()) {
      const t = runTime(run);
      const at = path.join(dir, run);
      if (!(t >= since && t < until) || !fs.existsSync(path.join(at, 'stream.jsonl'))) continue;
      const turn = fs.existsSync(path.join(at, 'prompts.md')) ? firstTurn(fs.readFileSync(path.join(at, 'prompts.md'), 'utf8')) : null;
      if (skill !== 'split' && !turn?.header.includes('/tech-analyst')) continue;
      const kase = { skill, truth: {} };
      if (skill === 'split') {
        const log = fs.existsSync(path.join(at, 'run.log')) ? fs.readFileSync(path.join(at, 'run.log'), 'utf8') : '';
        kase.truth.slices = (log.match(/opened task #/g) ?? []).length;
      }
      rows.push({ skill, id: `${workflow}/${run}`, ...score(kase, fs.readFileSync(path.join(at, 'stream.jsonl'), 'utf8')) });
    }
  }
  const out = path.join(WORK, 'runs', label);
  fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(path.join(out, 'observed.json'), `${JSON.stringify(rows)}\n`);
  console.log(`skill-evals: ${rows.length} round(s) observed under "${label}"`);
  report([label]);
}

function tally(rows) {
  const bySkill = {};
  for (const row of rows) {
    const skill = (bySkill[row.skill] ??= { runs: 0, cost: 0, turns: 0, checks: {}, failures: [] });
    skill.runs += 1;
    skill.cost += row.cost;
    skill.turns += row.turns;
    for (const [name, [ok, detail]] of Object.entries(row.checks)) {
      skill.checks[name] = (skill.checks[name] ?? 0) + (ok ? 1 : 0);
      if (!ok) skill.failures.push(`${row.id}: ${name}${detail ? ` (${detail})` : ''}`);
    }
  }
  return bySkill;
}

function scoreLabel(label) {
  const dir = path.join(WORK, 'runs', plainName(label, 'a label'));
  if (!fs.existsSync(dir)) die(`no run under the label "${label}"`);
  const observed = path.join(dir, 'observed.json');
  if (fs.existsSync(observed)) {
    if (fs.readdirSync(dir).some((f) => f.endsWith('.jsonl'))) console.warn(`skill-evals: "${label}" holds observed rounds and replays; only the observed ones are scored`);
    return tally(readJson(observed));
  }
  const cases = new Map(loadCases().map((c) => [`${c.skill}--${c.name}`, c]));
  const rows = [];
  for (const file of fs.readdirSync(dir).filter((f) => f.endsWith('.jsonl')).sort()) {
    const key = file.split('--').slice(0, 2).join('--');
    const kase = cases.get(key);
    if (!kase) continue;
    rows.push({ skill: kase.skill, id: file.replace(/\.jsonl$/, ''), ...score(kase, fs.readFileSync(path.join(dir, file), 'utf8')) });
  }
  return tally(rows);
}

function report(labels) {
  const scored = labels.map((l) => scoreLabel(l));
  const skills = [...new Set(scored.flatMap((s) => Object.keys(s)))].sort();
  for (const skill of skills) {
    console.log(`\n${skill}`);
    const names = [...new Set(scored.flatMap((s) => Object.keys(s[skill]?.checks ?? {})))];
    const width = Math.max(...names.map((n) => n.length), 10);
    console.log(`  ${''.padEnd(width)}  ${labels.map((l) => l.padStart(14)).join('')}`);
    for (const name of names) {
      const cells = scored.map((s) => {
        const k = s[skill];
        return k ? `${k.checks[name] ?? 0}/${k.runs}`.padStart(14) : '-'.padStart(14);
      });
      console.log(`  ${name.padEnd(width)}  ${cells.join('')}`);
    }
    const cost = scored.map((s) => (s[skill] ? `$${(s[skill].cost / s[skill].runs).toFixed(2)}/run`.padStart(14) : '-'.padStart(14)));
    const turns = scored.map((s) => (s[skill] ? `${(s[skill].turns / s[skill].runs).toFixed(1)} turns`.padStart(14) : '-'.padStart(14)));
    console.log(`  ${'cost'.padEnd(width)}  ${cost.join('')}`);
    console.log(`  ${'turns'.padEnd(width)}  ${turns.join('')}`);
    scored.forEach((s, i) => {
      for (const failure of s[skill]?.failures ?? []) console.log(`    ${labels[i]}: ${failure}`);
    });
  }
}

function list() {
  for (const c of loadCases()) {
    console.log(`${c.skill}/${c.name}  commit ${c.commit.slice(0, 8)}  ${c.model}/${c.effort}  from ${c.source}`);
  }
}

// ---- entry ---------------------------------------------------------------

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, ...rest] = process.argv.slice(2);
  const { positional, flags } = parseArgs(rest);
  if (command === 'harvest') harvest(positional[0] ?? die('harvest needs a factory run folder'), flags);
  else if (command === 'run') await run(positional, flags);
  else if (command === 'observe') observe(positional[0] ?? die('observe needs a factory folder'), flags);
  else if (command === 'report') report(positional.length ? positional : die('report needs a label'));
  else if (command === 'list') list();
  else die('usage: skill-evals.mjs harvest|run|observe|report|list (see the header of this file)');
}
