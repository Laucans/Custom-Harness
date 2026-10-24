// The mock server of the mock-up, in the page: the shell's wire protocol
// (`POST /capabilities/<id>`, `POST /commands`, `GET /commands/<id>`,
// `POST /commands/<id>/{confirm,cancel,requeue}`, the `dataVersion`
// stream) answered in memory, so the real Micro-UIs run on the real shell
// client with mock data. It knows no product, and it behaves as the
// DataGuard engine does (contracts G and H), so a Micro-UI built on it
// meets no surprise on the real server:
//
// - a read of `<identifier>` is the product's handler for it when there is
//   one (below), else the fixture `fixtures/<identifier>.json` as it is — a
//   plain value, or `{ "$match": [{ "variables": {…}, "data": … }],
//   "data": <default> }` to answer by variables (`not-found` when a
//   `$match` list has no match and no default);
// - a command of `<dataCapability>` is the product's handler for it, called
//   `(server, { payload, targetId, version, basedOn })` — it changes the
//   fixtures it loaded and answers the violation ids the DataGuard would,
//   or `'not-found'`. A DataCapability with no handler is **rejected** with
//   `unhandled-data-capability`: the real server refuses what no manifest
//   registers, so does the mock;
// - the aggregate's own invariants judge too, when the page ships
//   `fixtures/aggregates/<Aggregate>.json` (an aggregate.json, contract I)
//   and the command names its `target.aggregate`: every `rule` of the shape
//   `<field> <op> <value>` is checked on the payload, `reject` rejects,
//   `review` answers `awaiting_review`;
// - one queue per partition, as the engine's: a command whose `basedOn`
//   names a value another pending command declares ahead on the same field
//   is `awaiting_confirmation` with a `ValueDeclaredAhead` message; its
//   author confirms (`POST /commands/<id>/confirm`) or cancels; one that
//   reaches the head unconfirmed is `parked`, and `requeue` brings it back
//   at the end. Applied commands bump `dataVersion` and every micro-frontend
//   on the page refetches, as the product would. The messages (contract L)
//   are kept in `server.messages`, oldest first.
//
// The handlers are the product's, one file per unit, never bundled: the
// first read or command of `<id>` imports `mock/handlers/<id>.js`, which
// exports `reads` and/or `commands`, each an object keyed by that id only —
//
//   export const reads = { '<Capability>': async (server, variables) => data };
//   export const commands = { '<DataCapability>': async (server, { payload }) => violations };
//
// — the browser loads it as a module beside the page, so its imports are
// relative: helpers several files share live in `mock/handlers/_lib/`. The
// legacy `mock/src/reads.js` (`READS`) and `mock/src/commands.js`
// (`COMMANDS`) are still loaded when they exist, until
// `scripts/split-handlers.mjs` turns them into handler files. An id defined
// twice — in its file and a legacy file, or in another id's file — is an
// error naming both files, never one definition silently winning (the gate
// `mock-handlers` refuses it before a merge). It is found at startup for
// every id the server can know of without listing a folder — each legacy
// key, each `needs` and `actions` of `mock/pages.json` — and then the
// server is broken: every call answers 500 with the problems, kept in
// `server.problems` (`await server.ready`) and logged. An id nobody
// declared is checked on its first use, answered 500 alone.
//
// One server per page, shared by every bundle. Installed by `harness
// init-repo`; a product teaches it through `mock/handlers/`.

const LATENCY_MS = 120;
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const uuid = () => (crypto.randomUUID ? crypto.randomUUID() : 'xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx'.replace(/x/g, () => Math.floor(Math.random() * 16).toString(16)));
const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const notFound = () => json({ error: 'not-found' }, 404);
const refused = (why) => json({ error: why }, 409);
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const FINAL = new Set(['applied', 'rejected', 'cancelled', 'expired']);
/** The legacy files, by kind: where they sit relative to the page, and the object they export. */
const LEGACY = { reads: ['../src/reads.js', 'READS'], commands: ['../src/commands.js', 'COMMANDS'] };

/** A handler defined twice, or a handler file that breaks the convention: never a silent override. */
class HandlerError extends Error {}

/** A path relative to the page, as the repository names it. */
const shown = (path) => `mock/${path.replace(/^\.\.\//, '')}`;

/** The module at `path` (relative to the page), `null` when no such file is served. */
async function moduleAt(path) {
  const url = new URL(path, location.href).href;
  const served = await fetch(url).then((r) => r.ok, () => false);
  if (!served) return null;
  try {
    return await import(url);
  } catch (e) {
    throw new HandlerError(`${shown(path)} does not load: ${e && e.message ? e.message : e}`);
  }
}

/** A handler file checked: it exports `reads` and/or `commands`, keyed by its own id only. */
function ownFile(id, mod) {
  if (!mod) return null;
  const file = `mock/handlers/${id}.js`;
  const kinds = Object.keys(LEGACY).filter((kind) => mod[kind] !== undefined);
  if (!kinds.length) throw new HandlerError(`${file} exports neither reads nor commands`);
  for (const kind of kinds) {
    if (!mod[kind] || typeof mod[kind] !== 'object') throw new HandlerError(`${file}: ${kind} is not an object`);
    for (const key of Object.keys(mod[kind])) {
      if (key !== id) throw new HandlerError(`${file} defines ${key}: a handler file defines its own id only — ${key} belongs in mock/handlers/${key}.js`);
    }
  }
  return mod;
}

/** `<field> <op> <value>` on a payload: true when the rule holds or does not apply (field absent). */
export function ruleHolds(rule, payload) {
  const m = /^\s*([A-Za-z_][\w.]*)\s*(<=|>=|==|!=|<|>|=)\s*(.+?)\s*$/.exec(String(rule || ''));
  if (!m) return true;
  const [, field, op, raw] = m;
  if (!(field in payload)) return true;
  const left = payload[field];
  let right = raw;
  if (/^-?\d+(\.\d+)?$/.test(raw)) right = Number(raw);
  else if (raw === 'true' || raw === 'false') right = raw === 'true';
  else right = raw.replace(/^['"]|['"]$/g, '');
  switch (op) {
    case '<=': return left <= right;
    case '>=': return left >= right;
    case '<': return left < right;
    case '>': return left > right;
    case '!=': return left !== right;
    default: return left === right;
  }
}

/** The page's mock server, made on first use. */
export function mockServer() {
  if (window.__mockServer) return window.__mockServer;
  const s = { dataVersion: 1, fixtures: new Map(), listeners: new Set(), commands: new Map(), queue: new Map(), messages: [], tick: 1000, parkAfterMs: 30000 };

  /** A fixture's data by file name (`campagnes` → `fixtures/campagnes.json`), loaded once, mutable; `undefined` when absent. */
  s.load = async (name) => {
    if (!s.fixtures.has(name)) {
      s.fixtures.set(name, fetch(new URL(`../fixtures/${name}.json`, location.href)).then((r) => (r.ok ? r.json() : undefined)).catch(() => undefined));
    }
    return s.fixtures.get(name);
  };
  /** The legacy files' handlers, by kind, loaded once: `{}` for a file that is not there. */
  s.legacy = () => {
    if (!s.legacyHandlers) {
      s.legacyHandlers = Promise.all(Object.entries(LEGACY).map(async ([kind, [path, name]]) => [kind, ((await moduleAt(path)) || {})[name] || {}])).then(Object.fromEntries);
    }
    return s.legacyHandlers;
  };
  s.files = new Map();
  /** The handler of `id` for `kind` (`reads` or `commands`), `undefined` when the product wrote none. */
  s.handlerOf = async (kind, id) => {
    if (!s.files.has(id)) s.files.set(id, moduleAt(`../handlers/${encodeURIComponent(id)}.js`).then((mod) => ownFile(id, mod)));
    const [mod, legacy] = await Promise.all([s.files.get(id), s.legacy()]);
    const own = mod && mod[kind] && Object.hasOwn(mod[kind], id) ? mod[kind][id] : undefined;
    const kept = Object.hasOwn(legacy[kind], id) ? legacy[kind][id] : undefined;
    if (own && kept) throw new HandlerError(`${id} is defined twice: mock/handlers/${id}.js and ${shown(LEGACY[kind][0])} — keep one`);
    return own || kept;
  };
  /**
   * The startup check: every id the legacy files define or `mock/pages.json`
   * declares gets its handler looked up, so an id defined twice is found
   * before any call. Its problems, each once, in `s.problems`.
   */
  s.problems = [];
  s.check = async () => {
    const said = new Set();
    const fail = (e) => {
      const message = e && e.message ? e.message : String(e);
      if (said.has(message)) return;
      said.add(message);
      s.problems.push(message);
      console.error(message);
    };
    const ids = new Set();
    try {
      const legacy = await s.legacy();
      for (const kind of Object.keys(LEGACY)) for (const id of Object.keys(legacy[kind])) ids.add(id);
    } catch (e) {
      fail(e);
      return;
    }
    const pages = await fetch(new URL('../pages.json', location.href)).then((r) => (r.ok ? r.json() : undefined)).catch(() => undefined);
    for (const component of (pages && pages.components) || []) {
      for (const id of [].concat(component.needs || [], component.actions || [])) if (typeof id === 'string') ids.add(id);
    }
    for (const id of [...ids].sort()) {
      for (const kind of Object.keys(LEGACY)) await s.handlerOf(kind, id).catch(fail);
    }
  };
  s.ready = s.check();
  /** Tells every micro-frontend the data changed. */
  s.bump = () => {
    s.dataVersion += 1;
    for (const listener of s.listeners) listener({ data: JSON.stringify({ dataVersion: s.dataVersion }) });
  };
  s.uuid = uuid;
  s.next = () => s.tick++;
  /** Whether the plan's graph (`plan/graph.json`, contract R) names `id` as a write a milestone produces: mocked until built. */
  s.planned = async (id) => {
    if (!s.graph) s.graph = fetch(new URL('../../plan/graph.json', location.href)).then((r) => (r.ok ? r.json() : undefined)).catch(() => undefined);
    const graph = await s.graph;
    const entry = graph && graph.ids && graph.ids[id];
    return Boolean(entry && entry.kind === 'data-capability' && entry.producer != null);
  };
  /** A message of contract L, kept for the page and the tests. */
  s.tell = (message, to, commandId, extra = {}) => {
    const m = { message, to: Array.isArray(to) ? to : [to], commandId, at: new Date().toISOString(), ...extra };
    s.messages.push(m);
    return m;
  };

  const read = async (identifier, variables) => {
    const handler = await s.handlerOf('reads', identifier);
    if (handler) return handler(s, variables);
    const fixture = await s.load(identifier);
    if (fixture === undefined) return undefined;
    if (fixture && typeof fixture === 'object' && Array.isArray(fixture.$match)) {
      const hit = fixture.$match.find((m) => Object.entries(m.variables || {}).every(([k, v]) => same(variables[k], v)));
      return hit ? hit.data : fixture.data;
    }
    return fixture;
  };

  /** The invariants of `aggregate` on `payload`: the ids violated, split by what they do. */
  const invariants = async (aggregate, payload) => {
    const out = { reject: [], review: [] };
    if (!aggregate) return out;
    const manifest = await s.load(`aggregates/${aggregate}`);
    for (const inv of (manifest && manifest.invariants) || []) {
      if (!ruleHolds(inv.rule, payload)) (inv.onViolation === 'review' ? out.review : out.reject).push(inv.id);
    }
    return out;
  };

  /** The pending entries of a partition, by position. */
  const pendingIn = (partition) => [...s.queue.values()].filter((e) => e.partition === partition && !FINAL.has(e.state)).sort((a, b) => a.position - b.position);
  /** What `basedOn` read that moved, or that a pending command ahead declares: `{ by, command, value }` or null. */
  // `merging`: the fields whose aggregate says `merge` — never asked about.
  const declaredAhead = (entry, merging = new Set()) => {
    const fields = Object.keys(entry.payload).filter((f) => !merging.has(f));
    const ahead = pendingIn(entry.partition).find((e) => e.position < entry.position && e.state !== 'parked' && fields.some((f) => f in e.payload));
    if (ahead) { const f = fields.find((x) => x in ahead.payload); return { by: ahead.by, command: ahead.commandId, value: ahead.payload[f], field: f }; }
    const read = (entry.basedOn && entry.basedOn.values) || {};
    for (const f of fields) {
      if (f in read && s.confirmed.has(`${entry.partition}.${f}`) && !same(s.confirmed.get(`${entry.partition}.${f}`), read[f])) {
        return { by: '(applied)', command: '(applied)', value: s.confirmed.get(`${entry.partition}.${f}`), field: f };
      }
    }
    return null;
  };
  s.confirmed = new Map();
  const resultOf = (entry) => ({ commandId: entry.commandId, status: entry.state, dataVersion: entry.dataVersion || null, violations: entry.violations || [], reviewId: entry.reviewId || null });
  const answer = (entry, replayed = false) => json({ commandId: entry.commandId, partition: entry.partition, replayed, warnings: entry.warnings || [], result: resultOf(entry) }, 202);

  /** Applies every head that is ready, partition by partition; a head unconfirmed for `parkAfterMs` at the head is parked (the engine's grace, 30 s). */
  const drive = async () => {
    for (const partition of new Set([...s.queue.values()].filter((e) => !FINAL.has(e.state)).map((e) => e.partition))) {
      for (;;) {
        const head = pendingIn(partition).find((e) => e.state !== 'parked');
        if (!head) break;
        if (head.state === 'awaiting_review') break;
        if (head.state === 'awaiting_confirmation') {
          if (head.atHeadSince === undefined) head.atHeadSince = Date.now();
          if (Date.now() - head.atHeadSince < s.parkAfterMs) break;
          head.state = 'parked';
          s.tell('Parked', head.by, head.commandId, { reason: 'reached the head unconfirmed', actions: ['requeue', 'cancel'] });
          continue;
        }
        const verdict = await invariants(head.aggregate, head.payload);
        if (verdict.reject.length) {
          head.state = 'rejected';
          head.violations = verdict.reject;
          s.tell('CommandRejected', head.by, head.commandId, { violations: verdict.reject });
          continue;
        }
        if (head.handler) {
          const said = await head.handler(s, { payload: head.payload, targetId: head.targetId, version: head.version, basedOn: head.basedOn });
          const violations = Array.isArray(said) ? said : (said && said.violations) || [];
          if (violations.length) {
            head.state = 'rejected';
            head.violations = violations;
            s.tell('CommandRejected', head.by, head.commandId, { violations });
            continue;
          }
        }
        head.state = 'applied';
        s.bump();
        head.dataVersion = s.dataVersion;
        for (const [f, v] of Object.entries(head.payload)) s.confirmed.set(`${partition}.${f}`, v);
        s.tell('CommandApplied', [head.by, 'infrastructure'], head.commandId, { dataVersion: s.dataVersion });
        for (const behind of pendingIn(partition)) {
          if (behind.aheadOf === head.commandId) s.tell('AheadResolved', behind.by, behind.commandId, { reason: `${head.commandId} was resolved` });
        }
      }
    }
  };

  const command = async (body) => {
    const [dataCapability, version] = String(body.dataCapability || '').split('@');
    const payload = body.payload || {};
    const target = body.target || {};
    const targetId = target.id || body.targetId;
    const by = (body.caller && body.caller.by) || body.by || 'anonymous';
    const handler = await s.handlerOf('commands', dataCapability);
    if (body.idempotencyKey) {
      const seen = [...s.queue.values()].find((e) => e.idempotencyKey === body.idempotencyKey && e.dataCapability === dataCapability);
      if (seen) return answer(seen, true);
    }
    const aggregate = target.aggregate || null;
    const declared = ((aggregate && (await s.load(`aggregates/${aggregate}`))) || { fields: {} }).fields || {};
    // A derived field is recomputed by the aggregate: no command writes it.
    const derived = Object.keys(payload).filter((f) => declared[f] && declared[f].kind === 'derived');
    if (derived.length) return json({ error: `${dataCapability} writes the derived field(s) ${derived.join(', ')}: ${aggregate} recomputes them` }, 403);
    const commandId = uuid();
    const partition = `${aggregate || dataCapability}/${targetId || commandId}`;
    const entry = { commandId, dataCapability, version: Number(version) || 1, aggregate, partition, targetId, by, payload, basedOn: body.basedOn, idempotencyKey: body.idempotencyKey, handler, position: s.next(), state: 'queued', warnings: [] };
    s.queue.set(commandId, entry);
    s.commands.set(commandId, entry);
    if (!handler && (await s.planned(dataCapability))) {
      // Mocked until its milestone builds it: accepted, applied, no effect.
      entry.state = 'applied';
      entry.warnings.push(`mocked-data-capability: ${dataCapability} is planned, not built`);
      s.tell('CommandApplied', by, commandId, { mocked: true });
      s.bump();
      return answer(entry);
    }
    if (!handler) {
      entry.state = 'rejected';
      entry.violations = ['unhandled-data-capability'];
      s.tell('CommandRejected', by, commandId, { violations: entry.violations, reason: `${dataCapability} has no handler: write mock/handlers/${dataCapability}.js` });
      return answer(entry);
    }
    const verdict = await invariants(aggregate, payload);
    if (verdict.reject.length || verdict.review.length) {
      entry.warnings.push(`invariant(s) at risk: ${verdict.reject.concat(verdict.review).join(', ')}`);
      s.tell('InvariantAtRisk', by, commandId, { violations: verdict.reject.concat(verdict.review) });
    }
    const needsReview = verdict.review.length > 0 || Object.keys(payload).some((f) => declared[f] && declared[f].criticality === 'critical');
    const merging = new Set(Object.keys(declared).filter((f) => declared[f].merge));
    const ahead = declaredAhead(entry, merging);
    if (needsReview) {
      entry.state = 'awaiting_review';
      entry.reviewId = `r_${s.next()}`;
      entry.warnings.push(`review ${entry.reviewId} required`);
      s.tell('ReviewRequired', 'role:approver', commandId, { actions: ['approve', 'reject'] });
    } else if (ahead && (body.mode || 'confirm_on_stale') === 'confirm_on_stale') {
      entry.state = 'awaiting_confirmation';
      entry.aheadOf = ahead.command;
      entry.warnings.push('a value is declared ahead: confirm or cancel');
      s.tell('ValueDeclaredAhead', by, commandId, { field: `${partition}.${ahead.field}`, yourValue: payload[ahead.field], declaredAhead: { value: ahead.value, by: ahead.by, command: ahead.command }, actions: ['confirm_overwrite', 'cancel', 'edit'] });
    }
    s.tell('CommandQueued', by, commandId);
    await drive();
    return answer(entry);
  };

  const gesture = async (id, verb, body) => {
    const entry = s.queue.get(id);
    if (!entry) return notFound();
    const by = (body && body.by) || entry.by;
    if (by !== entry.by) return refused(`${id} is ${entry.by}'s to ${verb}`);
    if (verb === 'confirm') {
      if (entry.state !== 'awaiting_confirmation') return refused(`${id} is ${entry.state}, not awaiting a confirmation`);
      entry.state = 'confirmed';
    } else if (verb === 'cancel') {
      if (FINAL.has(entry.state)) return refused(`${id} is ${entry.state}`);
      entry.state = 'cancelled';
      s.tell('CommandCancelled', entry.by, id);
    } else if (verb === 'requeue') {
      if (entry.state !== 'parked') return refused(`${id} is ${entry.state}, not parked`);
      entry.state = 'cancelled';
      const fresh = { ...entry, commandId: `${id}-r`, position: s.next(), state: 'queued', warnings: [], requeuedFrom: id, aheadOf: undefined, atHeadSince: undefined };
      s.queue.set(fresh.commandId, fresh);
      s.commands.set(fresh.commandId, fresh);
      const ahead = declaredAhead(fresh);
      if (ahead) {
        fresh.state = 'awaiting_confirmation';
        fresh.aheadOf = ahead.command;
        fresh.warnings.push('a value is declared ahead: confirm or cancel');
        s.tell('ValueDeclaredAhead', fresh.by, fresh.commandId, { field: `${fresh.partition}.${ahead.field}`, yourValue: fresh.payload[ahead.field], declaredAhead: { value: ahead.value, by: ahead.by, command: ahead.command }, actions: ['confirm_overwrite', 'cancel', 'edit'] });
      }
      s.tell('CommandQueued', fresh.by, fresh.commandId);
      await drive();
      return answer(fresh);
    } else if (verb === 'approve' || verb === 'reject') {
      if (entry.state !== 'awaiting_review') return refused(`${id} is ${entry.state}, not awaiting a review`);
      entry.state = verb === 'approve' ? 'confirmed' : 'rejected';
      if (verb === 'reject') entry.violations = [`review ${entry.reviewId} rejected`];
    } else return notFound();
    await drive();
    return json({ result: resultOf(entry) });
  };

  s.fetch = async (input, init) => {
    try {
      await s.ready;
      if (s.problems.length) return json({ error: `the mock server did not start: ${s.problems.join('; ')}` }, 500);
      return await respond(input, init);
    } catch (e) {
      if (!(e instanceof HandlerError)) throw e;
      console.error(e.message);
      return json({ error: e.message }, 500);
    }
  };
  const respond = async (input, init) => {
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
      const entry = s.commands.get(decodeURIComponent(lookup[1]));
      return entry ? json({ result: resultOf(entry) }) : notFound();
    }
    const move = /^\/commands\/([^/]+)\/(confirm|cancel|requeue|approve|reject)$/.exec(url.pathname);
    if (move && method === 'POST') return gesture(decodeURIComponent(move[1]), move[2], body);
    return notFound();
  };
  /** Drives the queue now: what the real engine's applier does on its own clock. */
  s.drive = drive;
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
