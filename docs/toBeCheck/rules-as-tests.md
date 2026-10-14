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
- [ ] The `harness:rules` label is created.
- [ ] Initialize opens "Rules: write the rules and their tests" (`harness:rules`
      + `harness:ready`), before the roadmap task.
  - Its body carries a ```json block that validates against
    `contracts/p-rules.schema.json`.
  - A rule whose criterion is not whole is listed as left out.
- [ ] The watch routes `Rules { issue }` right after the planner.
  - `harness:ready` is taken off at once.
  - The dev loop starts no task while the issue is open; the idle line
    says "development waits on the rules' task #n".
- [ ] The write session opens a `harness:rules` PR from `rules/<n>` with:
  - `rules.json` exactly as in the issue;
  - at least one test per rule, in its family's suite;
  - the criterion copied above each test.

  In that PR, `node scripts/rules-gate.mjs coverage` is green, every
  business test is pending (nothing built), and the session does **not**
  merge.
- [ ] The fidelity session (opus) merges the PR when every test is
      faithful, and the issue closes.
  - On a deliberately unfaithful test, it comments on the PR and puts
    `harness:human` on the issue.
  - The view then shows "#n needs you".
- [ ] `gates.yml` on the rules PR:
  - `rule-tests-untouched` is skipped (the label);
  - `rules-hold` is non-strict;
  - the `rules-status` artifact is uploaded.
- [ ] A dev PR touching `tests/rules/` is refused by `rule-tests-untouched`.
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
- [ ] A milestone PR into `main_agent` runs `status --strict`: a red
      conform rule blocks it.
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
- [ ] Unknown, to look at: `scripts/rules-runtime.sh` stays a stub until a
      task makes it serve the product. Every rule stays **pending** until
      then, even once its units are built. Decide which task owns the
      runtime.
