# The rules are tests, written first (option B)

## Context

Each rule of the creation's Rules step (R<id>, journey or business, its
given/when/then) becomes a test in the product, written before any
development by a task of its own; development conforms to them. Decided in
a business grill (17 answers). Shipped with green tests and gates, never
exercised on a real product.

## What shipped

- `6e5a748`: contract P (`rules.json`), and the kit `init-repo` installs:
  - `tests/rules/business` (Rust, wire protocol, pending until built);
  - `tests/rules/journey` (Playwright);
  - `scripts/rules-gate.mjs` (coverage, status, untouched);
  - `scripts/rules-runtime.sh` (a stub);
  - three jobs in `gates.yml`.
- `aacd421`: the `harness:rules` label and the `rules` workflow:
  - stages: context, write, fidelity, with the fidelity stage merging;
  - routing: a rules issue comes after the planner, and any open one holds
    the dev loop;
  - Initialize opens the first rules task;
  - `CONTEST.md` leads `rules-gate.mjs contest` to open "Rules contested:
    PR #n";
  - a warning notification for an open contest.
- `edaaac1`: "Send the changed rules" in the Rules step, after Initialize.
  - It opens a rules task, with the changed rules marked conform.
  - It opens a conformity task, a sub-issue of the current milestone, or a
    roadmap item when no milestone is open.
- `0f6c38c`: each rule's state on its card and a summary in the Rules
  step.
  - The source is `rules-status.json`, the artifact of `gates.yml` on the
    integration branch, read at most once a minute.

## To check, on a fresh product (new initializations only)

- [ ] `init-repo` installs 47 files, among them:
  - `tests/rules/business/Cargo.toml`
  - `tests/rules/journey/kit.ts`
  - `scripts/rules-gate.mjs`
  - `contracts/p-rules.schema.json`
- [x] The `harness:rules` label is created. (seen 2026-10-16 22:04 Paris: `harness:rules` created on ForgeDesMondes)
- [x] Initialize opens "Rules: write the rules and their tests" (`harness:rules`
      + `harness:ready`), before the roadmap task.
  - Its body carries a ```json block that validates against
    `contracts/p-rules.schema.json`.
  - A rule whose criterion is not whole is listed as left out. (seen 2026-10-16 22:04 Paris: #2 (harness:rules + harness:ready) with a json block, before roadmap #4 (2026-10-17 00:05 Paris); the left-out case not exercised, schema validation not rechecked)
- [x] The watch routes `Rules { issue }` right after the planner.
  - `harness:ready` is taken off at once.
  - The dev loop starts no task while the issue is open; the idle line
    says "development waits on the rules' task #n". (seen 2026-10-24 21:11 Paris: `idle: development waits on the rules' task #517 …`; #2's harness:ready taken off at each attempt (10-16))
- [ ] The write session opens a `harness:rules` PR from `rules/<n>` with:
  - `rules.json` exactly as in the issue;
  - at least one test per rule, in its family's suite;
  - the criterion copied above each test.

  In that PR, `node scripts/rules-gate.mjs coverage` is green, every
  business test is pending (nothing built), and the session does **not**
  merge.
- [x] The fidelity session (opus) merges the PR when every test is
      faithful, and the issue closes.
  - On a deliberately unfaithful test, it comments on the PR and puts
    `harness:human` on the issue.
  - The view then shows "#n needs you". (seen 2026-10-17 02:05 Paris: PR #7 (rules/2) merged by the rules workflow, #2 closed (with 3 red gates: the merge-on-red bug fixed later); the unfaithful branch is now M6's, last box)
- [x] `gates.yml` on the rules PR:
  - `rule-tests-untouched` is skipped (the label);
  - `rules-hold` is non-strict;
  - the `rules-status` artifact is uploaded. (seen 2026-10-24 21:14 Paris: rules PRs #7…#518: `rule-tests-untouched` SKIPPED, `rules-hold` green with pending business tests (PR #7), `rules-status` artifact on the rules/517 and main_agent gates runs)
- [x] A dev PR touching `tests/rules/` is refused by `rule-tests-untouched`. (seen 2026-10-24 16:20 Paris: #501's pull_request run before its harness:rules label failed node-gates on `rule-tests-untouched`)
- [ ] A dev PR with `CONTEST.md` opens "Rules contested: PR #n" once.
  - The job's `issues: write` permission is enough for that.
  - The PR stays red, saying it waits; once the issue is closed, the PR
    says to remove `CONTEST.md` and conform.
  - The view shows a warning "#n: a rule's test is contested" and the
    rule's card says "contested ↗".
  - The rules task settles it: (a) fix the test, (b) change the rule, or
    (c) close with the reason.
  - It comments the decision on the dev PR, and `pr_fix` then removes
    `CONTEST.md`.
- [x] A milestone PR into `main_agent` runs `status --strict`: a red
      conform rule blocks it. (seen 2026-10-24 07:13 Paris: PR #423 (milestone #25) held red on rules-hold (R101) through 21 repairs, merged only once green)
- [ ] After Initialize, edit a rule's criterion in the Rules step, then
      validate.
  - The card "d · Send to the factory" lists "changed: R<id>".
  - Sending opens "Rules changed: R<id>" (R<id> marked conform) and
    "Conform to the changed rules: R<id>" under the open milestone (a
    sub-issue).
  - The card then lists both links, and the button is disabled ("No rule
    changed since").
- [ ] The rule cards show green, red, pending, to conform or untested
      once a `gates.yml` run on `main_agent` uploaded `rules-status`.
  - `gh run download` works with the view's slug-only `GhCli` (`-R`).
  - The summary links the run.
- [x] `scripts/rules-runtime.sh` (complete since the follow-up commit) in
      the CI's `rules-hold`, on the Postgres service:
  - while `crates/server` does not exist it prints nothing, and every
    rule is pending;
  - once it exists, it builds a scratch database from the migrations and
    starts `crates/server` with `FIXTURES_DIR`;
  - it waits on `/health`, then builds and serves `apps/shell`;
  - `tee` does not hang on the background processes;
  - the step "Stop the product" drops the scratch database. (seen 2026-10-24 21:57 Paris: `rules-hold` green on PR #519 in 4 min with crates/server present; 7 rules green end to end on main_agent (night/report-2026-10-24.md); the drop of the scratch database not read in a job log)

  Checked locally on a scratch product: no server, no database, the server
  alone, server and shell, and a server that never answers (exit 1). It
  has never run on a GitHub runner.
- [x] The first development tasks put the server in `crates/server` and
      the frontend in `apps/shell`, as the layout of `CLAUDE.md` says.
  - The server loads `FIXTURES_DIR`, otherwise the givens have no data. (seen 2026-10-17: main_agent has `crates/server` (FIXTURES_DIR and PORTS=fake from rules-runtime.sh) and `apps/shell`, laid by #16 `Product skeleton`)
- [ ] A dev PR touching `scripts/rules-runtime.sh` is refused by
      `rule-tests-untouched`.
- [ ] The plant's prompts know the rules come first (commit "the plant's
      prompts").
  - The planner's first milestone on a fresh product lays `crates/server`
    and `apps/shell`. Each milestone carries a `## Rules` section, and
    every rule of `rules.json` falls in exactly one milestone.
  - Split hands each task its `rules`: a `## Rules` section, kept through
    both refinements.
  - Code sessions run `rules-runtime.sh serve` and the rules' suites, and
    write `CONTEST.md` rather than edit a test.
  - `pr_fix` never edits `tests/rules/`, and removes `CONTEST.md` once a
    contest is settled.
  - The review brief names a task PR touching the rules first.
- [x] Unfaithful rules' tests no longer wait on a human (M6 of the
      autonomy plan).
  - The first time, the fidelity session comments its findings
    (`<!-- rules-fidelity: unfaithful -->`) and poses `harness:ready`; the
    writer answers them on `rules/<n>`.
  - The second time, it poses `harness:needs-decision`: `decide` reads the
    rules' PR, merges it as written or answers. An answer re-poses
    `harness:ready`. (seen 2026-10-17 00:53–01:04 Paris: #2 — `found unfaithful` comment and harness:ready re-posed; the second time harness:needs-decision; decide answered `repair` at 23:03Z)
