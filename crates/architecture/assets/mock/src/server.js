// The mock server of the mock-up, in the page: the shell's wire protocol
// (`POST /capabilities/<id>`, `POST /commands`, `GET /commands/<id>`, the
// `dataVersion` stream) answered in memory, so the real Micro-UIs run on
// the real shell client with mock data. It knows no product:
//
// - a read of `<identifier>` is `READS[identifier](server, variables)` from
//   `reads.js` when the product wrote one, else the fixture
//   `fixtures/<identifier>.json` as it is — a plain value, or
//   `{ "$match": [{ "variables": {…}, "data": … }], "data": <default> }` to
//   answer by variables (`not-found` when a `$match` list has no match and
//   no default);
// - a command of `<dataCapability>` is `COMMANDS[dataCapability](server,
//   { payload, targetId, version })` from `commands.js` when the product
//   wrote one — it changes the fixtures it loaded and answers the violation
//   ids the DataGuard would, or `'not-found'` — else applied as is;
// - every applied command bumps `dataVersion` and every micro-frontend on
//   the page refetches, as the product would.
//
// One server per page, shared by every bundle. Installed by `harness
// init-repo`; a product teaches it through `reads.js` and `commands.js`.

import { COMMANDS } from './commands.js';
import { READS } from './reads.js';

const LATENCY_MS = 120;
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const uuid = () => (crypto.randomUUID ? crypto.randomUUID() : 'xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx'.replace(/x/g, () => Math.floor(Math.random() * 16).toString(16)));
const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const notFound = () => json({ error: 'not-found' }, 404);
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/** The page's mock server, made on first use. */
export function mockServer() {
  if (window.__mockServer) return window.__mockServer;
  const s = { dataVersion: 1, fixtures: new Map(), listeners: new Set(), commands: new Map(), tick: 1000 };

  /** A fixture's data by file name (`campagnes` → `fixtures/campagnes.json`), loaded once, mutable; `undefined` when absent. */
  s.load = async (name) => {
    if (!s.fixtures.has(name)) {
      s.fixtures.set(name, fetch(new URL(`../fixtures/${name}.json`, location.href)).then((r) => (r.ok ? r.json() : undefined)).catch(() => undefined));
    }
    return s.fixtures.get(name);
  };
  /** Tells every micro-frontend the data changed. */
  s.bump = () => {
    s.dataVersion += 1;
    for (const listener of s.listeners) listener({ data: JSON.stringify({ dataVersion: s.dataVersion }) });
  };
  s.uuid = uuid;
  s.next = () => s.tick++;

  const read = async (identifier, variables) => {
    const handler = READS[identifier];
    if (handler) return handler(s, variables);
    const fixture = await s.load(identifier);
    if (fixture === undefined) return undefined;
    if (fixture && typeof fixture === 'object' && Array.isArray(fixture.$match)) {
      const hit = fixture.$match.find((m) => Object.entries(m.variables || {}).every(([k, v]) => same(variables[k], v)));
      return hit ? hit.data : fixture.data;
    }
    return fixture;
  };

  const command = async (body) => {
    const [dataCapability, version] = String(body.dataCapability || '').split('@');
    const payload = body.payload || {};
    const targetId = body.target && body.target.id;
    const handler = COMMANDS[dataCapability];
    let violations = [];
    if (handler) {
      const verdict = await handler(s, { payload, targetId, version: Number(version) || 1, basedOn: body.basedOn });
      if (verdict === 'not-found') return notFound();
      violations = Array.isArray(verdict) ? verdict : (verdict && verdict.violations) || [];
    }
    const commandId = uuid();
    const status = violations.length ? 'rejected' : 'applied';
    if (status === 'applied') s.bump();
    // Contract H: the five fields, nothing else.
    const result = { commandId, status, dataVersion: status === 'applied' ? s.dataVersion : null, violations, reviewId: null };
    s.commands.set(commandId, result);
    return json({ commandId, partition: `${dataCapability}/${targetId || commandId}`, replayed: false, warnings: [], result }, 202);
  };

  s.fetch = async (input, init) => {
    await delay(LATENCY_MS);
    const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url);
    const method = (init && init.method) || 'GET';
    const body = init && typeof init.body === 'string' ? JSON.parse(init.body) : {};
    if (method === 'POST' && url.pathname.startsWith('/capabilities/')) {
      const data = await read(decodeURIComponent(url.pathname.slice('/capabilities/'.length)), body.variables || {});
      return data === undefined ? notFound() : json({ asOf: s.dataVersion, data });
    }
    if (method === 'POST' && url.pathname === '/commands') return command(body);
    const lookup = /^\/commands\/([^/]+)$/.exec(url.pathname);
    if (lookup && method === 'GET') {
      const result = s.commands.get(decodeURIComponent(lookup[1]));
      return result ? json({ result }) : notFound();
    }
    return notFound();
  };
  s.eventSource = () => {
    const source = { readyState: 1, addEventListener(type, listener) { if (type === 'dataVersion') s.listeners.add(listener); }, close() { source.readyState = 2; } };
    return source;
  };
  window.__mockServer = s;
  return s;
}

/** The page's one shell client over the mock server: `create(server)` makes it the first time, with the product's own shell package. */
export function shellOnce(create) {
  if (!window.__mockShell) window.__mockShell = create(mockServer());
  return window.__mockShell;
}
