#!/usr/bin/env node
// The ports' gate (contract Q): what `ports.json` says the product reaches
// outside, checked against the code. Installed by `harness init-repo`; the
// repository owns it from here. No dependency: node alone.
//
//   node scripts/ports-gate.mjs declared
//       Every port a manifest names (`ports` in capability.json and
//       data-capability.json) is in ports.json, and every port's `usedBy`
//       is exactly the units that name it. A Capability never names a
//       stateful port: it reads our mirror, a DataCapability writes through it.
//
//   node scripts/ports-gate.mjs contracts
//       Every port but `database` has its crate (`crates/ports/<port>/`), a
//       fake (the `fake` module) and its contract tests (`tests/`), and each
//       adapter it declares exists.
//
//   node scripts/ports-gate.mjs behind
//       No HTTP client, provider SDK or database client outside a port's
//       adapter: a Rust crate's Cargo.toml, an app's package.json, a
//       `fetch(` in apps/ outside the shell. A database client only in the
//       `database` port's adapter (the Data layer) and the DataGuard.
//
//   node scripts/ports-gate.mjs secrets
//       Nothing under secrets/ but sops-encrypted `*.enc.yaml` files and the
//       README; no `.env` file committed but `.env.example`.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';

const root = process.cwd();
const say = (line) => process.stdout.write(line + '\n');
const fail = (line) => { process.stdout.write(`::error::${line}\n`); };
const posix = (p) => p.split(sep).join('/');

// A Rust dependency that reaches outside: HTTP, a cloud or provider SDK, a
// database, a cache, a broker. The `database` port's adapter may name the
// database ones.
const RUST_OUTSIDE = [
  /^reqwest$/, /^hyper$/, /^hyper-util$/, /^ureq$/, /^isahc$/, /^surf$/, /^attohttpc$/, /^curl$/,
  /^aws-sdk-/, /^aws-config$/, /^rusoto_/, /^google-cloud-/, /^gcp_/, /^azure_/,
  /^stripe$/, /^async-stripe$/, /^lettre$/, /^sendgrid$/, /^mailgun/, /^openai/, /^async-openai$/,
  /^anthropic/, /^twilio/, /^octocrab$/, /^redis$/, /^mongodb$/, /^rdkafka$/, /^lapin$/,
];
const RUST_DATABASE = [/^sqlx$/, /^tokio-postgres$/, /^postgres$/, /^deadpool-postgres$/, /^diesel$/, /^sea-orm$/, /^bb8-postgres$/];
const NPM_OUTSIDE = [
  /^axios$/, /^node-fetch$/, /^got$/, /^ky$/, /^undici$/, /^superagent$/, /^stripe$/, /^@stripe\//,
  /^@aws-sdk\//, /^aws-sdk$/, /^@google-cloud\//, /^@azure\//, /^openai$/, /^@anthropic-ai\//,
  /^nodemailer$/, /^@sendgrid\//, /^twilio$/, /^pg$/, /^postgres$/, /^mysql2?$/, /^mongodb$/, /^redis$/, /^ioredis$/,
  /^firebase/, /^@supabase\//,
];

function walk(dir, keep, out = []) {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name === 'target' || name === '.git') continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, keep, out);
    else if (keep(name, path)) out.push(path);
  }
  return out;
}

function ports() {
  const path = join(root, 'ports.json');
  if (!existsSync(path)) return null;
  const read = JSON.parse(readFileSync(path, 'utf8'));
  return Array.isArray(read.ports) ? read.ports : [];
}

function manifests() {
  const found = [];
  for (const path of walk(join(root, 'crates'), (n) => n === 'capability.json' || n === 'data-capability.json')) {
    const m = JSON.parse(readFileSync(path, 'utf8'));
    const write = path.endsWith('data-capability.json');
    const id = write ? m.dataCapability : m.capability;
    if (id) found.push({ path: posix(relative(root, path)), id: String(id).split('@')[0], write, ports: m.ports || [] });
  }
  return found;
}

function declared() {
  const all = ports();
  if (all === null) { say('no ports.json yet: nothing declared'); return 0; }
  const byId = new Map(all.map((p) => [p.id, p]));
  const units = manifests();
  let failed = 0;
  for (const u of units) {
    for (const id of u.ports) {
      const port = byId.get(id);
      if (!port) { fail(`${u.path}: names the port ${id}, which ports.json has not — a new port is a human's to approve`); failed += 1; continue; }
      if (port.stateful && !u.write) { fail(`${u.path}: a Capability reads the stateful port ${id} — read our mirror; only a DataCapability goes through it`); failed += 1; }
    }
  }
  for (const port of all) {
    const users = units.filter((u) => u.ports.includes(port.id)).map((u) => u.id).sort();
    const listed = [...(port.usedBy || [])].sort();
    const missing = users.filter((u) => !listed.includes(u));
    const stale = listed.filter((u) => !users.includes(u));
    if (missing.length) { fail(`ports.json: ${port.id} is used by ${missing.join(', ')}, which its usedBy does not list`); failed += 1; }
    if (stale.length) { fail(`ports.json: ${port.id} lists ${stale.join(', ')} in usedBy, which name it nowhere`); failed += 1; }
    say(`${port.id} (${port.provider}): ${users.length ? users.join(', ') : 'used by no unit yet'}`);
  }
  return failed ? 1 : 0;
}

function contracts() {
  const all = ports();
  if (all === null) { say('no ports.json yet: no port to test'); return 0; }
  let failed = 0;
  for (const port of all.filter((p) => p.id !== 'database')) {
    const dir = join(root, 'crates/ports', port.id);
    if (!existsSync(join(dir, 'Cargo.toml'))) { fail(`ports.json: ${port.id} has no crate at crates/ports/${port.id}/`); failed += 1; continue; }
    const sources = walk(join(dir, 'src'), (n) => n.endsWith('.rs')).map((p) => readFileSync(p, 'utf8')).join('\n');
    if (!/\bmod\s+fake\b/.test(sources) && !existsSync(join(dir, 'src/fake.rs'))) { fail(`crates/ports/${port.id}: no fake — the tests and the mock-up run on it`); failed += 1; }
    if (!walk(join(dir, 'tests'), (n) => n.endsWith('.rs')).length) { fail(`crates/ports/${port.id}: no contract tests under tests/ — every adapter passes the same ones`); failed += 1; }
    if (port.adapter && !existsSync(join(root, port.adapter, 'Cargo.toml'))) { fail(`ports.json: ${port.id}'s adapter ${port.adapter} is not a crate`); failed += 1; }
    if (!failed) say(`${port.id}: fake, contract tests and adapter in place`);
  }
  return failed ? 1 : 0;
}

/// The dependency names a Cargo.toml declares, in any dependencies table.
function cargoDependencies(text) {
  const names = [];
  let inDeps = false;
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    const table = line.match(/^\[(.+)\]$/);
    if (table) {
      inDeps = /(^|\.)(dev-|build-)?dependencies$/.test(table[1]);
      const inline = table[1].match(/(?:^|\.)(?:dev-|build-)?dependencies\.([A-Za-z0-9_-]+)$/);
      if (inline) names.push(inline[1]);
      continue;
    }
    if (!inDeps) continue;
    const m = line.match(/^([A-Za-z0-9_-]+)\s*=/);
    if (m) names.push(m[1]);
  }
  return names;
}

function behind() {
  const all = ports() || [];
  const databaseAdapter = (all.find((p) => p.id === 'database') || {}).adapter || 'crates/data';
  // The Data layer reads PostgreSQL, the DataGuard writes it: the two only.
  const reachesTheStore = (rel) => [databaseAdapter, 'crates/dataguard'].some((a) => rel === a || rel.startsWith(a + '/'));
  const adapters = all.map((p) => p.adapter).filter(Boolean);
  const inAdapter = (path) => path.startsWith('crates/ports/') || adapters.some((a) => path === a || path.startsWith(a + '/'));
  let failed = 0;
  for (const path of walk(join(root, 'crates'), (n) => n === 'Cargo.toml')) {
    const rel = posix(relative(root, path)).replace(/\/Cargo\.toml$/, '');
    for (const dep of cargoDependencies(readFileSync(path, 'utf8'))) {
      if (RUST_DATABASE.some((r) => r.test(dep))) {
        if (!reachesTheStore(rel)) {
          fail(`${rel}: depends on ${dep}, a database client — only the Data layer (${databaseAdapter}) and the DataGuard reach PostgreSQL`); failed += 1;
        }
      } else if (RUST_OUTSIDE.some((r) => r.test(dep)) && !inAdapter(rel)) {
        fail(`${rel}: depends on ${dep}, which reaches outside the product — wrap it in a port's adapter under crates/ports/`); failed += 1;
      }
    }
  }
  for (const path of walk(join(root, 'apps'), (n) => n === 'package.json')) {
    const rel = posix(relative(root, path));
    const pkg = JSON.parse(readFileSync(path, 'utf8'));
    for (const dep of Object.keys({ ...pkg.dependencies, ...pkg.devDependencies })) {
      if (NPM_OUTSIDE.some((r) => r.test(dep))) { fail(`${rel}: depends on ${dep}, which reaches outside the product — the frontend reaches only our server, which reaches a port`); failed += 1; }
    }
  }
  for (const path of walk(join(root, 'apps'), (n) => /\.(ts|tsx|js|mjs)$/.test(n))) {
    const rel = posix(relative(root, path));
    if (rel.startsWith('apps/shell/')) continue;
    if (/\bfetch\s*\(|XMLHttpRequest|new\s+WebSocket\s*\(/.test(readFileSync(path, 'utf8'))) {
      fail(`${rel}: calls the network itself — a Micro-UI reaches Capabilities and DataCapabilities by identifier, through the shell`); failed += 1;
    }
  }
  if (!failed) say('nothing reaches outside the product but through a port');
  return failed ? 1 : 0;
}

function secrets() {
  let failed = 0;
  for (const path of walk(join(root, 'secrets'), () => true)) {
    const rel = posix(relative(root, path));
    if (rel === 'secrets/README.md') continue;
    if (!rel.endsWith('.enc.yaml')) { fail(`${rel}: a secret file in clear — only sops-encrypted secrets/<env>.enc.yaml are committed`); failed += 1; continue; }
    const text = readFileSync(path, 'utf8');
    if (!/^sops:/m.test(text) || !/ENC\[/.test(text)) { fail(`${rel}: not encrypted by sops — run \`sops --encrypt --in-place ${rel}\``); failed += 1; }
  }
  for (const path of walk(root, (n) => /^\.env(\..+)?$/.test(n) && n !== '.env.example')) {
    fail(`${posix(relative(root, path))}: an environment file is committed — its secrets go to secrets/<env>.enc.yaml`); failed += 1;
  }
  if (!failed) say('every committed secret is encrypted');
  return failed ? 1 : 0;
}

const [mode] = process.argv.slice(2);
const run = { declared, contracts, behind, secrets }[mode];
if (!run) { say('usage: ports-gate.mjs declared | contracts | behind | secrets'); process.exit(2); }
process.exit(run());
