// Builds the mock-up's micro-frontends from the real Micro-UIs' code: for
// every entry of `components` in `mock/pages.json` that names a `source`
// (the Micro-UI's folder under `apps/`), bundles `mock/src/<Name>.js` into
// `mock/components/<Name>.js` with esbuild, and records in `builtFrom` a
// digest of what it was built from — the entry, the shared files under
// `mock/src/`, every file under `<source>/src/` — so the gates know a
// bundle is stale. Run from the repository root: `node mock/build.mjs`.
// Installed by `harness init-repo`. The digest is FNV-1a 64 over
// `path\0content\0` for every input sorted by path, the same the harness
// computes.

import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';

const ESBUILD = 'esbuild@0.25.0';
const SHARED = ['server.js', 'screen.js', 'reads.js', 'commands.js'];

function walk(dir, out) {
  let entries = [];
  try { entries = readdirSync(dir); } catch (e) { return out; }
  for (const name of entries.sort()) {
    if (name === 'node_modules' || name === 'dist' || name.startsWith('.')) continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, out); else out.push(path);
  }
  return out;
}

export function digest(files) {
  let h = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = (1n << 64n) - 1n;
  const feed = (bytes) => { for (const b of bytes) { h ^= BigInt(b); h = (h * prime) & mask; } };
  const enc = new TextEncoder();
  for (const [path, content] of [...files].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))) {
    feed(enc.encode(path)); feed([0]); feed(enc.encode(content)); feed([0]);
  }
  return h.toString(16).padStart(16, '0');
}

export function inputsOf(name, source) {
  const files = [];
  const add = (path) => { try { files.push([path.split('\\').join('/'), readFileSync(path, 'utf8')]); } catch (e) { /* absent: not an input */ } };
  add(join('mock/src', `${name}.js`));
  for (const shared of SHARED) add(join('mock/src', shared));
  for (const path of walk(join(source, 'src'), [])) add(relative('.', path));
  return files;
}

const manifestPath = 'mock/pages.json';
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
let built = 0;
for (const component of manifest.components || []) {
  if (!component.source) continue;
  const entry = `mock/src/${component.name}.js`;
  execFileSync('npx', ['--yes', ESBUILD, entry, '--bundle', '--format=iife', '--target=es2022', '--log-level=warning', `--outfile=mock/components/${component.name}.js`], { stdio: 'inherit' });
  component.builtFrom = digest(inputsOf(component.name, component.source));
  built += 1;
}
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`built ${built} micro-frontend(s)`);
