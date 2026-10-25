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
//
//   import { rule, screen } from './kit';
//
//   rule('R7', ['screen:campaign', 'micro-ui:CampaignList'], 'a campaign opens from the list', async ({ page }) => {
//     // given: a campaign in the list
//     // when: the DM clicks it
//     // then: its page opens with its sessions
//     await screen(page, 'campaigns');
//     ...
//   });
import { test, type APIResponse, type Browser, type BrowserContext, type Page, type Route } from '@playwright/test';
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
    async ({ context, browser, page: _page }, use) => {
      const found = new Set<string>();
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
        await ctx.route(isCommandUrl, handler);
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
          await ctx.unroute(isCommandUrl, handler).catch(() => undefined);
        }
        for (const [p, close] of pages) p.close = close;
      }
      if (found.size) throw new Error(offManifest([...found]));
    },
    { auto: true },
  ],
});

/** Proves rule `id`: pending while a unit it `needs` is not built or nothing serves the product (decided as the test is declared: a pending test opens no browser); else `body` runs, and must hold — and sends only declared payload keys. */
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
  journey(`${id} ${title}`, { annotation }, body);
}
