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
//   import { rule, screen } from './kit';
//
//   rule('R7', ['screen:campaign', 'micro-ui:CampaignList'], 'a campaign opens from the list', async ({ page }) => {
//     // given: a campaign in the list
//     // when: the DM clicks it
//     // then: its page opens with its sessions
//     await screen(page, 'campaigns');
//     ...
//   });
import { test, type Page } from '@playwright/test';
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

/** Proves rule `id`: pending while a unit it `needs` is not built or nothing serves the product (decided as the test is declared: a pending test opens no browser); else `body` runs, and must hold. */
export function rule(id: string, needs: string[], title: string, body: Parameters<typeof test>[2]): void {
  const missing = needs.filter((n) => !built(n));
  const why = missing.length ? `waiting for ${missing.join(', ')}` : process.env.RULES_APP_URL ? '' : 'no app: scripts/rules-runtime.sh serves none yet';
  const annotation = [{ type: 'rule', description: id }];
  if (why) {
    // eslint-disable-next-line no-empty-pattern
    test(`${id} ${title}`, { annotation: [...annotation, { type: 'pending', description: why }] }, async ({}) => {
      test.skip(true, `${id} pending: ${why}`);
    });
    return;
  }
  test(`${id} ${title}`, { annotation }, body);
}
