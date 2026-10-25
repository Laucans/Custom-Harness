// Writes each rule test's outcome, one JSON line, to RULES_STATUS (default
// `target/rules-status.jsonl` at the repository's root) — what
// `scripts/rules-gate.mjs` reads. A test the kit found mis-mounted is
// written so, whatever it answered: its red or green proves nothing.
// Installed by `harness init-repo`.
import type { Reporter, TestCase, TestResult } from '@playwright/test/reporter';
import { appendFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { MIS_MOUNTED, root } from './kit';

export default class StatusReporter implements Reporter {
  onTestEnd(test: TestCase, result: TestResult): void {
    const id = test.annotations.find((a) => a.type === 'rule')?.description;
    if (!id) return;
    // What the test pushed while it ran: on the result, or (older Playwright) on the test.
    const ran = (result as { annotations?: TestCase['annotations'] }).annotations ?? [];
    const pending = test.annotations.find((a) => a.type === 'pending')?.description;
    const misMounted = [...ran, ...test.annotations].find((a) => a.type === MIS_MOUNTED)?.description;
    const state = pending ? 'pending' : misMounted ? 'mis-mounted' : result.status === 'passed' ? 'green' : 'red';
    const why = pending || misMounted || (result.error?.message ?? '').split('\n')[0];
    const path = process.env.RULES_STATUS || join(root, 'target/rules-status.jsonl');
    mkdirSync(dirname(path), { recursive: true });
    appendFileSync(path, JSON.stringify({ rule: id, family: 'journey', test: test.title, state, why }) + '\n');
  }
}
