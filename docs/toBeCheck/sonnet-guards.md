# Sonnet on the data layer: one test per acceptance criterion, opus on the second data-layer repair

## Context

The blind review of 2026-10-25
(`harness-dnd2-run/night/blind-review-sonnet-opus.md`, ForgeDesMondes data
layer, 10 bundles, 3 judges) found sonnet after the SK lot level with or
above opus. Sonnet's one real defect before SK was acceptance criteria no
test checked: #510, bundle G, AC3 and AC4 never verified. The owner kept
sonnet on the data layer with guards; two of them ship here:

- (2) one test per acceptance criterion, every code task, both models;
- (3) a data-layer PR still red after its first repair gets its second on
  opus.

## What shipped

Branch `feat/sonnet-guards`, off `main` `d185fc9` (worktree
`harness-guards`). Not committed yet.

1. `crates/workflows/src/dev_loop/data/acceptance.rs`: parses
   `## Acceptance Criteria` (bullets, `1.`, given/when/then over several
   lines), numbers them `- AC1: …`, finds `AC<n>` names (a UUID's `-ac12-`
   or `ac1e` is none), counts only the names a change adds
   (`added(base, head)`), reads `ACk: covered by <path>[::<test>] — <why>`
   and `ACk: not testable here — <reason>` lines. Proptested.
2. `dev_loop::action::actions::Ask`: the code prompt's SCOPE carries the
   criteria numbered; a `--- ACCEPTANCE CRITERIA ---` block asks for
   `// AC2: …` on the test that proves each, the `covered by` pointer for a
   criterion an existing test or a rule under `tests/rules/` proves (that
   test never edited), or the untestable line in the PR body.
3. `dev_loop::checks::criteria::CriteriaTested`, third check of `code`'s
   post gate ("code must keep the architecture"): reads the task's open PR,
   each test file at its base and head, each pointed file at its head; a
   criterion none covers posts a blocking review (`the criteria gate`),
   labels the PR `harness:pr-fix`, the task `harness:review-pending`, and
   halts the round. A failed GitHub read (files, body, one file) warns and
   lets the round through, writing nothing.
4. `pr_review`: the precheck reads the criteria of the task the PR closes
   (`Closes #n`), the brief lists them numbered (`{criteria}`), accepts
   the same three proofs, lists `ACk uncovered — …` under **Gaps with the
   SPEC**, blocking.
5. `pr_fix::data::ladder`: sonnet, sonnet, opus — but a `harness:data-layer`
   task's PR with a red check at its second repair runs it on opus.
   The launcher's journal line:
   `#<pr>: repair <k> on this PR — <model>/<effort> (<why>)`.
   `MODEL`/`EFFORT` do not touch repairs (as before this lot).
6. `.claude/skills/code/SKILL.md` §5 and §7.4 say the same.

## To check on the next live runs

- [ ] Next code PR with criteria: every `ACk` is named in a test
      (`grep -rn "AC[0-9]" <the PR's test files>`) or declared
      pointed `ACk: covered by <path> — <why>` or declared
      `ACk: not testable here — <reason>` in the PR body. The run's journal
      says `criteria: AC1..ACn of #<task> each named by a test of PR #<pr>,
      pointed to one or declared untestable`.
- [ ] A task whose criterion a rule under `tests/rules/` proves (as #510's
      AC4): the PR body points to it, the rule file is not edited, the
      gate passes, `pr_review` does not flag a drift.
- [ ] A PR touching a test file that already names `AC1..ACk` from an
      earlier task does not pass on those old names.
- [ ] A PR that misses one: the PR carries a review "🤖 Review notes — the
      criteria gate" listing `ACk (<criterion>): named by no test this PR
      adds, …`, `harness:pr-fix` is posed, the repair adds the test (or
      the line), and `pr_review`'s next brief is clean on it.
- [ ] The criteria the prompt numbered match the issue's bullets one to
      one (read the session's prompt in
      `.llocal/logs/agent-loop/<run>/`): no intro line counted, a
      given/when/then over three lines counted once.
- [ ] `pr_review`'s brief prompt lists the criteria numbered exactly as the
      code session's SCOPE did (same AC numbers).
- [ ] `pr_review`'s brief on a PR with an uncovered criterion lists
      `ACk uncovered — …` and ends `VERDICT: blocking`.
- [ ] Next `harness:data-layer` PR with a red check after its first
      repair: the pr-fix run's journal says `repair 2 on this PR — opus/high
      (a harness:data-layer task's PR with a check still red after its first
      repair)` and the session ran on opus (`session.log`, spending row).
- [ ] A non-data-layer PR at its second repair still says `sonnet/high
      (the ladder's first rungs)`; a third repair says `opus/high (the 2
      repairs on sonnet did not clear it)`.
- [ ] A gh hiccup during the gate (rate limit, 5xx) leaves a journal warning
      `criteria: … cannot be read — not judged` and no review, no label.
