// Writes each rule test's outcome, one JSON line, to RULES_STATUS (default
// `target/rules-status.jsonl` at the repository's root) — what
// `scripts/rules-gate.mjs` reads. Installed by `harness init-repo`.
import type { Reporter, TestCase, TestResult } from '@playwright/test/reporter';
import { appendFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { root } from './kit';

export default class StatusReporter implements Reporter {
  onTestEnd(test: TestCase, result: TestResult): void {
    const id = test.annotations.find((a) => a.type === 'rule')?.description;
    if (!id) return;
    const pending = test.annotations.find((a) => a.type === 'pending')?.description;
    const state = pending ? 'pending' : result.status === 'passed' ? 'green' : 'red';
    const why = pending || (result.error?.message ?? '').split('\n')[0];
    const path = process.env.RULES_STATUS || join(root, 'target/rules-status.jsonl');
    mkdirSync(dirname(path), { recursive: true });
    appendFileSync(path, JSON.stringify({ rule: id, family: 'journey', test: test.title, state, why }) + '\n');
  }
}
