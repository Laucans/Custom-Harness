// The rules' kit, for the journey rules' tests (`*.spec.ts`): a test proves
// one rule of `rules.json` (contract P), written before the screens it walks
// through, and so it reaches them by identifier: a screen by its name
// (contract E), opened at RULES_SCREEN_ROUTE (`/screens/<screen>` unless
// the runtime says otherwise), on the product RULES_APP_URL serves.
//
// A rule's test is **pending** while a unit it needs is not built yet (no
// manifest names it) or nothing serves the product: it is skipped, and says
// so. Once everything it needs exists it runs, and must hold. The status
// reporter writes each outcome where `scripts/rules-gate.mjs` reads it.
//
// A running test also fails when a screen sent a payload its manifest does
// not declare: the real server answers such a `POST /commands` with the
// DataGuard engine's warning `payload key(s) <keys> not declared by
// <dataCapability>` (accepted, never rejected), and the journey fails on it
// once its body ran, naming the keys and the DataCapability. That is a
// payload to fix in the screen, never a rule to contest. The kit answers
// every page's `POST /commands` itself — the default context's and any
// context the test opens — reading the answer before the page sees it, and
// a close waits for the commands in flight: nothing is lost when the body
// ends on a command nobody awaits, or closes a context on its way out.
// It fails the same way, "fix the read, this is not a rule contest", when a
// page reads a Capability with a variable its capability.json `input` does
// not take: the real server ignores it and answers null.
//
// A running test is also checked for how it is mounted: a screen it opens
// (at RULES_SCREEN_ROUTE) that its needs do not name, and a Capability or a
// DataCapability its pages reach that none of its needs covers — named, or
// in the `needs`/`actions` of a Micro-UI it needs or one its screens lay
// out. Either makes the rule **mis-mounted** (the status reporter writes
// it): neither red nor contested, the test is the rules' task's to fix.
// A screen counts as opened only by a valid name (contract E) the shell
// answers: a link to `/screens/x.html`, or one answered 404, is the
// product's link, and fails the test red.
// Needs are ids, as the manifests and `plan/graph.json` give them:
// `capability:<system>.<name>`, `data-capability:<system>.<name>`,
// `micro-ui:<Name>`, `screen:<name>`, `aggregate:<Name>`.
//
//   import { rule, screen } from './kit';
//
//   rule('R7', ['screen:campagne', 'micro-ui:CampaignList'], 'a campaign opens from the list', async ({ page }) => {
//     // given: a campaign in the list
//     // when: the DM clicks it
//     // then: its page opens with its sessions
//     await screen(page, 'campaigns');
//     ...
//   });
import { test, type APIResponse, type Browser, type BrowserContext, type Page, type Response as PageResponse, type Route } from '@playwright/test';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The repository's root: this folder is `tests/rules/journey`. */
export const root = join(dirname(fileURLToPath(import.meta.url)), '../../..');

const KINDS: Record<string, [string, string]> = {
  capability: ['capability.json', 'capability'],
  'data-capability': ['data-capability.json', 'dataCapability'],
  'micro-ui': ['micro-ui.json', 'microUi'],
  aggregate: ['aggregate.json', 'aggregate'],
  screen: ['', 'screen'],
};

function manifests(dir: string, out: string[]): void {
  if (!existsSync(dir)) return;
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name === 'target') continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) manifests(path, out);
    else if (name.endsWith('.json')) out.push(path);
  }
}

/** A unit the product has built: a manifest names it (`kind:name`). */
export function built(need: string): boolean {
  const at = need.indexOf(':');
  const kind = KINDS[need.slice(0, at)];
  if (at < 0 || !kind) return false;
  const [file, field] = kind;
  const found: string[] = [];
  manifests(join(root, 'crates'), found);
  manifests(join(root, 'apps'), found);
  return found.some((path) => {
    const named = file ? path.endsWith(`/${file}`) : dirname(path).endsWith('apps/screens');
    if (!named) return false;
    try { return JSON.parse(readFileSync(path, 'utf8'))[field] === need.slice(at + 1); } catch { return false; }
  });
}

/** A manifest the product holds, read once a running test first asks: where it is (from the root), and what it says. */
type Manifest = { path: string; manifest: Record<string, unknown> };
let index: Map<string, Manifest> | null = null;

/** The manifest that names `kind:id`, if the product has built it. */
export function manifestOf(kind: string, id: string): Manifest | undefined {
  if (!index) {
    const read = new Map<string, Manifest>();
    const found: string[] = [];
    manifests(join(root, 'crates'), found);
    manifests(join(root, 'apps'), found);
    for (const path of found) {
      for (const [k, [file, field]] of Object.entries(KINDS)) {
        const named = file ? path.endsWith(`/${file}`) : dirname(path).endsWith('apps/screens');
        if (!named) continue;
        try {
          const manifest = JSON.parse(readFileSync(path, 'utf8'));
          const name = manifest && manifest[field];
          if (typeof name === 'string' && !read.has(`${k}:${name}`)) read.set(`${k}:${name}`, { path: path.slice(root.length + 1), manifest });
        } catch {
          // Not JSON: names nothing.
        }
      }
    }
    index = read;
  }
  return index.get(`${kind}:${id}`);
}

/** An id without its version: `campaign.saveEntity@2` → `campaign.saveEntity`. */
const unversioned = (id: string): string => id.split('@')[0];
const strings = (list: unknown): string[] => (Array.isArray(list) ? list.filter((x): x is string => typeof x === 'string') : []);

/**
 * What a test's needs cover, as `kind:id`: each need, and for a Micro-UI it
 * needs — or one a screen it needs lays out — the Micro-UI and the ids of
 * its manifest's `needs` and `actions` (either kind: an id is one name).
 */
export function covered(needs: string[]): Set<string> {
  const ids = new Set<string>();
  const microUi = (name: string): void => {
    ids.add(`micro-ui:${name}`);
    const m = manifestOf('micro-ui', name)?.manifest;
    for (const id of [...strings(m?.needs), ...strings(m?.actions)]) {
      ids.add(`capability:${unversioned(id)}`);
      ids.add(`data-capability:${unversioned(id)}`);
    }
  };
  for (const need of needs) {
    const at = need.indexOf(':');
    if (at < 0) continue;
    const kind = need.slice(0, at);
    const id = unversioned(need.slice(at + 1));
    ids.add(`${kind}:${id}`);
    if (kind === 'micro-ui') microUi(id);
    if (kind === 'screen') {
      const layout = manifestOf('screen', id)?.manifest.layout;
      for (const entry of Array.isArray(layout) ? layout : []) if (entry && typeof entry === 'object') Object.keys(entry).forEach(microUi);
    }
  }
  return ids;
}

/** Whether `declared` takes the wire key `key`: by name, or through a dotted path under it (`entity.id` declares `entity`). */
const declares = (declared: Record<string, unknown>, key: string): boolean => key in declared || Object.keys(declared).some((name) => name.startsWith(`${key}.`));

/** What is wrong when a read of `capability` sends a variable its manifest's `input` does not take; nothing when it gives no `input` object. */
export function offInput(capability: string, variables: unknown, manifest: Manifest): string | undefined {
  const input = manifest.manifest.input;
  if (!input || typeof input !== 'object' || Array.isArray(input)) return undefined;
  if (!variables || typeof variables !== 'object' || Array.isArray(variables)) return undefined;
  const declared = input as Record<string, unknown>;
  const off = Object.keys(variables).filter((key) => !declares(declared, key));
  if (!off.length) return undefined;
  const takes = Object.keys(declared).length ? `takes ${Object.keys(declared).join(', ')}` : 'takes no input';
  return `${capability} has no input ${off.map((k) => `"${k}"`).join(', ')}; its capability.json ${takes} (${manifest.path})`;
}

/** Why a journey fails on its pages' off-manifest reads: the first line names the first one, then each, then what to do. */
export function offRead(found: string[]): string {
  return [
    `variables off the manifest — fix the read, this is not a rule contest: ${found[0]}`,
    ...found.map((f) => `  - ${f}`),
    'The real server ignores a variable its Capability does not take and answers null. A Micro-UI reads with the variables its manifest\'s input declares; a missing one is a change to the manifest, by its owner.',
  ].join('\n');
}

/** The screen a URL opens, where RULES_SCREEN_ROUTE serves them (`/screens/<screen>` unless the runtime says otherwise). */
export function screenOf(url: URL): string | undefined {
  const template = process.env.RULES_SCREEN_ROUTE || '/screens/<screen>';
  const [before, after = ''] = template.split('<screen>');
  const escape = (text: string): string => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const found = new RegExp(`^${escape(before)}([^/?#&]+)${escape(after)}(?=$|[/?#&])`).exec(url.pathname + url.search);
  if (!found) return undefined;
  try {
    return decodeURIComponent(found[1]);
  } catch {
    return found[1];
  }
}

/** A fixture of the mock-up (`mock/fixtures/<name>.json`): the data the human validated with it. */
export function fixture(name: string): unknown {
  return JSON.parse(readFileSync(join(root, 'mock/fixtures', `${name}.json`), 'utf8'));
}

/** Opens a screen by its name, where the runtime serves it. */
export async function screen(page: Page, name: string): Promise<void> {
  const route = (process.env.RULES_SCREEN_ROUTE || '/screens/<screen>').replace('<screen>', encodeURIComponent(name));
  await page.goto(route);
}

/**
 * The DataGuard engine's warning for a payload key the DataCapability's
 * manifest does not declare (`crates/dataguard`, `undeclared_keys`): the
 * keys, comma-separated, then the DataCapability.
 */
export const UNDECLARED = /^payload key\(s\) (.+) not declared by (\S+)$/;

/** The engine's undeclared-key warnings in what a `POST /commands` answered: its own `warnings`, never a follow-up's. */
export function undeclared(answer: unknown): string[] {
  const warnings = answer && typeof answer === 'object' ? (answer as { warnings?: unknown }).warnings : undefined;
  return Array.isArray(warnings) ? warnings.filter((w): w is string => typeof w === 'string' && UNDECLARED.test(w)) : [];
}

/** Why a journey fails on `warnings`: the first line names the first one (what the rules' gate shows), then each, then what to do. */
export function offManifest(warnings: string[]): string {
  const lines = warnings.map((w) => {
    const [, keys, id] = UNDECLARED.exec(w) || [];
    return `  - ${keys} sent to ${id}: its data-capability.json (crates/*/data-capabilities/*/, the one whose dataCapability is ${id}) does not declare it`;
  });
  return [
    `payload off the manifest — fix the payload, this is not a rule contest: ${warnings[0]}`,
    ...lines,
    'The real server accepted it with this warning. A screen sends only the keys its manifest\'s payload declares; a missing key is a change to the manifest, by its owner.',
  ].join('\n');
}

/** Whether `url` is a command's: `/commands`, wherever the runtime mounts it. */
function isCommandUrl(url: URL): boolean {
  return /\/commands$/.test(url.pathname);
}

/** The Capability a URL reads (`/capabilities/<id>`, wherever the runtime mounts it). */
function capabilityOf(url: URL): string | undefined {
  const found = /\/capabilities\/([^/]+)$/.exec(url.pathname);
  if (!found) return undefined;
  try {
    return decodeURIComponent(found[1]);
  } catch {
    return found[1];
  }
}

const isCapabilityUrl = (url: URL): boolean => capabilityOf(url) !== undefined;
const isScreenUrl = (url: URL): boolean => screenOf(url) !== undefined;
/** A screen's name, as contract E and the shell's `isScreenName` take it. */
const SCREEN_NAME = /^[a-z][a-z0-9-]*$/;

/** A link the shell does not serve: the product's to fix, not the test's. */
function unservedLinks(urls: string[]): string {
  return [
    ...urls.map((url) => `a link to ${url}, which the shell does not serve`),
    'A screen is opened by its name (contract E: lowercase letters, digits and dashes) at the route the shell serves; a link named like a mock-up file is the product\'s to fix, not the rule\'s.',
  ].join('\n');
}

/** A request's JSON body, or nothing. */
function sent(route: Route): Record<string, unknown> | undefined {
  try {
    const body: unknown = route.request().postDataJSON();
    return body && typeof body === 'object' ? (body as Record<string, unknown>) : undefined;
  } catch {
    return undefined;
  }
}

/** The annotation a running rule's test carries its needs in. */
const NEEDS = 'needs';
/** The annotation a mis-mounted test carries, for the status reporter. */
export const MIS_MOUNTED = 'mis-mounted';

/**
 * Counts, in the page, the `POST /commands` its `fetch` and `XMLHttpRequest`
 * have in flight (`window.__kitCommandsInFlight`): a close waits for them,
 * so a command sent just before it reaches the server and is read.
 */
export const IN_FLIGHT = `(() => {
  if (typeof window.__kitCommandsInFlight === 'number') return;
  window.__kitCommandsInFlight = 0;
  const isCommand = (method, url) => {
    try {
      return String(method || 'GET').toUpperCase() === 'POST' && /\\/commands$/.test(new URL(String(url), location.href).pathname);
    } catch {
      return false;
    }
  };
  const track = (sent) => {
    window.__kitCommandsInFlight++;
    const done = () => { window.__kitCommandsInFlight--; };
    sent.then(done, done);
    return sent;
  };
  const fetch0 = window.fetch;
  if (typeof fetch0 === 'function') {
    window.fetch = function (input, init) {
      const request = typeof Request === 'function' && input instanceof Request ? input : null;
      const method = (init && init.method) || (request ? request.method : 'GET');
      const sent = fetch0.apply(this, arguments);
      return isCommand(method, request ? request.url : input) ? track(sent) : sent;
    };
  }
  const Xhr = window.XMLHttpRequest;
  if (typeof Xhr === 'function') {
    const open = Xhr.prototype.open;
    const send = Xhr.prototype.send;
    Xhr.prototype.open = function (method, url) {
      this.__kitCommand = isCommand(method, url);
      return open.apply(this, arguments);
    };
    Xhr.prototype.send = function () {
      if (this.__kitCommand) track(new Promise((resolve) => this.addEventListener('loadend', resolve, { once: true })));
      return send.apply(this, arguments);
    };
  }
})();`;

/**
 * The running rules' test: every `POST /commands` a page of the test sends
 * — in the default context or in one the test opens with
 * `browser.newContext()` (or `browser.newPage()`) — is answered through the
 * kit, which reads the answer before the page sees it. An undeclared-key
 * warning fails the test once its body ran. Nothing is lost on the way: a
 * page or a context the test closes first waits for the commands its pages
 * have in flight, and the kit, which takes `page`, tears down before the
 * page closes, waiting the same way for the body's last commands.
 */
const journey = test.extend<{ payloadsDeclared: void }>({
  payloadsDeclared: [
    // `page` is taken for its order: this teardown runs while it is open.
    async ({ context, browser, page: _page }, use, testInfo) => {
      const found = new Set<string>();
      // What the pages reached, for the mounting: screens opened, units reached, reads off their input.
      const screens = new Set<string>();
      const unserved = new Set<string>();
      const reached = new Set<string>();
      const reads = new Set<string>();
      const needsSaid = testInfo?.annotations?.find((a) => a.type === NEEDS)?.description;
      const needs: string[] | undefined = needsSaid ? JSON.parse(needsSaid) : undefined;
      const pending = new Set<Promise<void>>();
      const drain = async (): Promise<void> => {
        while (pending.size) await Promise.allSettled([...pending]);
      };
      const settle = async (pages: Page[]): Promise<void> => {
        await Promise.all(
          pages.map((p) => p.waitForFunction('!window.__kitCommandsInFlight', undefined, { timeout: 5_000, polling: 10 }).catch(() => undefined)),
        );
        await drain();
      };
      const answer = async (route: Route): Promise<void> => {
        try {
          if (route.request().method() !== 'POST') return await route.fallback();
          const command = sent(route)?.dataCapability;
          if (typeof command === 'string') reached.add(`data-capability:${unversioned(command)}`);
          let response: APIResponse;
          try {
            response = await route.fetch();
          } catch {
            // The server never answered: the page sees the request fail, as it would have.
            return await route.abort('failed');
          }
          const body: unknown = await response.json().catch(() => null);
          for (const w of undeclared(body)) found.add(w);
          await route.fulfill({ response });
        } catch {
          // The page or its context closed meanwhile: what was read is kept.
        }
      };
      const handler = (route: Route): Promise<void> => {
        const run = answer(route);
        pending.add(run);
        void run.finally(() => pending.delete(run));
        return run;
      };
      // A read or a screen is only looked at: it goes on its way untouched.
      const read = async (route: Route): Promise<void> => {
        try {
          const capability = capabilityOf(new URL(route.request().url()));
          if (capability && route.request().method() === 'POST') {
            reached.add(`capability:${capability}`);
            const manifest = manifestOf('capability', capability);
            const off = manifest && offInput(capability, sent(route)?.variables, manifest);
            if (off) reads.add(off);
          }
          await route.fallback();
        } catch {
          // The page or its context closed meanwhile: what was read is kept.
        }
      };
      const opened = async (route: Route): Promise<void> => {
        try {
          const url = new URL(route.request().url());
          const name = screenOf(url);
          if (name && SCREEN_NAME.test(name)) screens.add(name);
          else if (name) unserved.add(`${url.pathname}${url.search}`);
          await route.fallback();
        } catch {
          // The page or its context closed meanwhile: what was read is kept.
        }
      };
      // A screen the shell answers 404 was never opened: the link is the product's.
      const answered = (response: PageResponse): void => {
        try {
          const url = new URL(response.url());
          const name = screenOf(url);
          if (!name || response.status() !== 404) return;
          screens.delete(name);
          unserved.add(`${url.pathname}${url.search}`);
        } catch {
          // A response without a URL to read says nothing of a screen.
        }
      };
      const pages: [Page, Page['close']][] = [];
      const watchPage = (p: Page): void => {
        if (pages.some(([seen]) => seen === p)) return;
        const close = p.close;
        p.close = async (...args: Parameters<Page['close']>) => {
          await settle([p]);
          return close.apply(p, args);
        };
        pages.push([p, close]);
      };
      const contexts: [BrowserContext, BrowserContext['close']][] = [];
      const watch = async (ctx: BrowserContext): Promise<BrowserContext> => {
        await ctx.addInitScript(IN_FLIGHT);
        await ctx.route(isScreenUrl, opened);
        await ctx.route(isCapabilityUrl, read);
        await ctx.route(isCommandUrl, handler);
        ctx.on('response', answered);
        ctx.pages().forEach(watchPage);
        ctx.on('page', watchPage);
        const close = ctx.close;
        ctx.close = async (...args: Parameters<BrowserContext['close']>) => {
          await settle(ctx.pages());
          return close.apply(ctx, args);
        };
        contexts.push([ctx, close]);
        return ctx;
      };
      await watch(context);
      const newContext = browser.newContext;
      browser.newContext = async (...args: Parameters<Browser['newContext']>) => watch(await newContext.apply(browser, args));
      try {
        await use();
      } finally {
        browser.newContext = newContext;
        await settle(contexts.flatMap(([ctx]) => ctx.pages()));
        for (const [ctx, close] of contexts) {
          ctx.close = close;
          ctx.off('page', watchPage);
          ctx.off('response', answered);
          await ctx.unroute(isCommandUrl, handler).catch(() => undefined);
          await ctx.unroute(isCapabilityUrl, read).catch(() => undefined);
          await ctx.unroute(isScreenUrl, opened).catch(() => undefined);
        }
        for (const [p, close] of pages) p.close = close;
      }
      if (needs) {
        const covers = covered(needs);
        const misMounted = [
          ...[...screens].filter((name) => !covers.has(`screen:${name}`)).map((name) => `opens screen:${name}, which its needs do not name: a journey lists every screen it walks through`),
          ...[...reached]
            .filter((unit) => !covers.has(unit))
            .map((unit) => `reaches ${unit}, which none of its needs covers (named, or in the needs or actions of a Micro-UI it needs or one its screens lay out): a journey lists every unit it uses`),
        ];
        if (misMounted.length) {
          const why = misMounted.join(' · ');
          testInfo.annotations.push({ type: MIS_MOUNTED, description: why });
          process.stderr.write(`mis-mounted: ${why} — the test is mounted wrong, not the product: the rules' task fixes it\n`);
        }
      }
      const failures = [
        ...(found.size ? [offManifest([...found])] : []),
        ...(reads.size ? [offRead([...reads])] : []),
        ...(unserved.size ? [unservedLinks([...unserved])] : []),
      ];
      if (failures.length) throw new Error(failures.join('\n'));
    },
    { auto: true },
  ],
});

/** Proves rule `id`: pending while a unit it `needs` is not built or nothing serves the product (decided as the test is declared: a pending test opens no browser); else `body` runs, and must hold — its pages sending only declared payload keys and variables, and reaching only what its needs cover (else mis-mounted). */
export function rule(id: string, needs: string[], title: string, body: Parameters<typeof test>[2]): void {
  const missing = needs.filter((n) => !built(n));
  const why = missing.length ? `waiting for ${missing.join(', ')}` : process.env.RULES_APP_URL ? '' : 'no app: apps/shell is not built, or the runtime serves none';
  const annotation = [{ type: 'rule', description: id }];
  if (why) {
    // eslint-disable-next-line no-empty-pattern
    test(`${id} ${title}`, { annotation: [...annotation, { type: 'pending', description: why }] }, async ({}) => {
      test.skip(true, `${id} pending: ${why}`);
    });
    return;
  }
  journey(`${id} ${title}`, { annotation: [...annotation, { type: NEEDS, description: JSON.stringify(needs) }] }, body);
}
