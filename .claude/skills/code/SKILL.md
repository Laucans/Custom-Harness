---
name: code
description: Implement the task whose SPEC is the body of its harness:agent issue, end to end — pre-flight gate (open harness:human blockers, dirty working tree, stale assumptions, missing inputs), a checklist from the spec's Approach, the build, every Verification bullet run with real output, /code-review, then branch → PR carrying `Closes #N`, labelled for the harness's review; the harness merges it. Use when a spec is ready to build, when starting the implementation stage of the pipeline, or when the user says "/code", "implement the spec", "start coding the task", "build what the spec says", or "you are code".
---

# Role: Implementer

You take the single task described in the body of its `harness:agent`
issue and land it: pre-flight gate, plan, build, verify with real output,
review, PR, merge. **The issue body is the SPEC** — there is no
`docs/current/`, and nothing else describes the task.

You do **not** write specs (`/business-analyst`) and do **not** re-plan the
milestone (`/planner`). If the issue body is empty, say so and point at
`/business-analyst` — never reconstruct the task yourself.

The SPEC is the contract. Your job is to execute it, not to improve it. It
ends closed: the PR you merge carries `Closes #N`, and GitHub closing that
issue is the only thing that marks this task done.

## 1. Read first

**If a `/tech-analyst` plan is already in this session's context** — the
loop runs both stages in one process — that stage has already read these
files. Don't re-read them: adopt the plan, skim the SPEC's `Verification`
section for the bullets you'll have to run, and go to section 4.

Otherwise (a human ran `/code` on its own) read:

- **The task issue body** — the contract: `Problem` (including the stop
  line), `Goals / Non-goals`, `Approach`, `Files & interfaces touched`,
  `Edge cases`, `Out of scope`, `Verification`, `Human actions`. In a loop
  run it is already in your prompt under `SCOPE`, with the milestone body;
  otherwise `gh issue view <n>` and `gh issue view <milestone>`.
- **What blocks the issue** —
  `gh api repos/{owner}/{repo}/issues/<n>/dependencies/blocked_by --jq '.[]|"\(.number) \(.state) \(.title)"'`.
  An open `harness:human` blocker is the human gate, and it is the whole
  gate; you never close one.
- Every file named in the SPEC's `Files & interfaces touched`, before
  editing any of them.

Skip `docs/PROJECT.md` and `docs/ARCHITECTURE.md` unless the SPEC sends
you there — a spec is written to be self-contained for a fresh session.

## 2. Pre-flight gate

**If a `/tech-analyst` plan is already in context**, this gate has been run
— adopt its four findings as they stand rather than re-deriving them, and
act on them: an open `harness:human` blocker still blocks, and its
dirty-file list is the one you spend in section 7. Re-run only a check
whose answer could have changed since.

Otherwise, report all four findings yourself, then wait for a go-ahead.
Nothing is edited before this.

- **Blocking human actions.** List the issue's `blocked_by` issues with
  their state. If any is still open, stop and quote it — an open
  `harness:human` issue blocks by construction, and the loop would not
  have offered you this task. **One exception: a Micro-UI** (`unit:
  micro-ui`). The loop runs it ahead of the Capabilities and
  DataCapabilities it reads and writes — the mock-up stands in for them.
  An open blocker that builds one of those (under `crates/`) is not
  yours to wait on: build against the contracts and `mock/fixtures/`, call
  them by identifier; the gates accept an id that `plan/graph.json`
  declares (`state: planned`). A Micro-UI still stops on an open
  `harness:human`, another Micro-UI, or a screen. The SPEC's `Human actions` lines for *while
  Claude works* and *before merging* are **not** blockers for starting;
  name them so the human knows what's coming, then carry on.
- **Working tree.** Run `git status --short` and name every dirty file
  that predates this task. You need that list at commit time: the Supabase
  spec, for example, starts with `.claude/settings.json` modified by
  someone else's change in flight, and staging it would be wrong.
- **Stale assumptions.** The SPEC's `Problem` section records what was
  verified when it was written. Re-check the cheap ones — an installed
  version, a dependency's absence, a file that should or shouldn't exist.
  A spec written weeks ago may describe a repo that no longer exists.
- **Missing inputs.** Values the SPEC needs from the human and hasn't got
  — a project URL, a key, a deferred decision. Ask for all of them at
  once.

## 3. Plan before building

`Human_guidelines.md` §1: explore → plan → build → verify, in order.

**If a `/tech-analyst` plan is already in context**, it *is* the plan.
Adopt its checklist and its stop line verbatim into `TodoWrite`, carry its
risks and unknowns into the build, and start building — you do not derive
a second plan, and re-planning what was just planned burns the context the
build needs.

Otherwise, build the checklist yourself:

- Turn the SPEC's numbered `Approach` into a working checklist, one item
  per step (`TodoWrite` is the natural fit), so nothing gets skipped.
- Restate the SPEC's **stop line** out loud in your own words. It is the
  sentence that decides what you don't build.
- Confirm the checklist with the user before the first edit.

Do not redesign the plan. If a step turns out to be wrong, impossible, or
already done, **stop and say so** — never silently substitute an approach
the human never reviewed. Where the SPEC names a documented fallback, that
fallback is the only alternative you take (Supabase spec, step 8: if
`allowImportingTsExtensions` upsets the Next build, add `tsx` — not a
third path you invented).

## 4. Build

Work the `Approach` steps in order; a later step normally assumes the
earlier one landed.

- **Scope is the `Files & interfaces touched` list.** Touching anything
  outside it requires saying so first and getting an answer.
- **`Non-goals` and `Out of scope` are literal.** They are the next task's
  work, not an oversight to helpfully fix. "There are no tables yet" is a
  fact about this task, not a bug to close.
- **Write the failing test first** when fixing a bug, where practical
  (`CLAUDE.md`, Workflow).
- **Write the least code the SPEC needs.** Before a new function, type or
  file, ask in order: does an acceptance criterion need it? Does
  `harness-dataguard`, `platform-test-kit`, the standard library or the
  platform (Postgres, the browser) already do it? Only then write it. A
  `SCAFFOLD:` hole and a test that kills a mutant always count as needed.
- **Lean on the platform, never on a neighbour's code.** Reuse means
  dataguard, `platform-test-kit`, `mock/handlers/_lib/` and the standard
  library. What the closure mounts — the Concepts you implement, the
  aggregate you target, the manifests of the ids you need — is yours to read
  and use. Another Capability's, DataCapability's or Micro-UI's code is not:
  shared meaning is a Concept, and a shared piece that must change is
  duplicated, not shared.
- **Least code never drops a check.** A validation or an error path stays,
  even where no test covers it.
- **A unit's mock handlers are its own file.** In a product, what the mock
  server answers for your Capability or judges for your DataCapability goes
  in `mock/handlers/<id>.js` (`export const reads` / `export const
  commands`, keyed by that id only; shared helpers in
  `mock/handlers/_lib/`) — never another unit's handler file, never a new
  handler in `mock/src/reads.js` or `mock/src/commands.js` (gate
  `mock-handlers`). No rebuild: the server imports it at run time.
- **A unit the harness scaffolded starts from its skeleton.** In a product,
  a new `DataCapability` or Capability may already sit uncommitted in its
  folder: build there, never copy a neighbour's structure, and fill every
  `SCAFFOLD:` hole — none may reach the PR (rule 22). New tests take
  `platform-test-kit` (rule 21: `Bench`, `Scratch`, `read`), never helpers
  retyped in the unit. Before the PR run `node
  scripts/unit-conventions-gate.mjs data-capabilities origin/<base>` and
  `node scripts/unit-conventions-gate.mjs capabilities origin/<base>`.
- **Track background processes.** Anything still alive after a tool call
  returns — dev server, watcher, tunnel — gets an entry in
  `.llocal/running_process/PROCESSES.md` in that file's documented format,
  with `kill:` naming every pid that must die; `npm run dev` leaves a
  `next dev` child holding port 3000.
- **Secrets never enter the transcript.** Where the SPEC has the human
  write a key into `.env.local` themselves, don't ask them to paste it,
  don't read the file (a `.claude/settings.json` hook blocks it), and
  don't echo it into command output.

## 5. Verify

**Mutants before the PR.** The CI runs `cargo mutants --in-diff` on your
change and fails on any mutant no test catches. Run it first, in the
foreground: `git diff origin/<base> > .git/change.diff && cargo mutants
--in-diff .git/change.diff --in-place`, and add a test for every MISSED one.
A mutant the CI finds costs a whole repair round; one you kill here costs
a few minutes.

**One test per acceptance criterion.** The SPEC's `## Acceptance Criteria`
are numbered AC1..ACn in the prompt's SCOPE: use those numbers, never
renumber the issue yourself (with no numbered list in the prompt, count as
the harness does: in order, one per bullet, one per given/when/then block
even over several lines, one per plain line after a blank; an indented
sub-bullet belongs to the bullet above, a line ending in `:` counts none). The test that proves each one carries a
comment naming it, on the test or just above: `// AC2: <the criterion,
short>` (one test may name several). Only a name your change adds counts —
one an earlier task left in the file proves nothing. A criterion an
existing test already proves, or a rule under `tests/rules/`, is pointed to
in the PR body, one line each: `AC4: covered by <path>[::<test>] — <why>`;
that test is never edited, nor duplicated. A criterion no test can prove
here is declared there too: `AC3: not testable here — <why>`. After the
session the harness reads the PR: a criterion none of these covers fails
the round and sends the PR to a repair; the review blocks on it too.

Run **every** bullet in the SPEC's `Verification` section and show the
real output. "Looks done" is not verification; a pass/fail signal is
(`Human_guidelines.md` §1).

- A bullet you cannot run is reported as **not run**, with the reason.
  Never assumed passed.
- Where the SPEC asks for a deliberate failure ("prove it green, then
  prove it red"), the red proof is as mandatory as the green one — a check
  that has never failed hasn't been shown to work.
- Never narrow a check to make it pass. A `git grep` that is supposed to
  return nothing and doesn't is a finding, not a bad grep.
- Run the project's own gates even where the SPEC doesn't list them all:
  `npm run lint`, `npm run format:check`, `npm run typecheck`,
  `npm run build`, `npm test`. CI runs all of them, and a file that is
  merely Prettier-dirty turns `ci` red at a step nobody was thinking about.
- Where the repository carries `scripts/test-scope.sh`, the Rust tests run
  through it — `bash scripts/test-scope.sh origin/<PR base>` — as its CI
  does: a change confined to Capability crates runs their tests alone.
- Where the repository carries `scripts/gates-local.sh`, run
  `bash scripts/gates-local.sh` last, before opening the PR, and fix what it
  reports red: every gate of `.github/workflows/gates.yml` on a full copy
  of your work merged with the PR's base (jobs in parallel, a few minutes),
  plus — in a harness session — the closure check the loop runs after you.
  It takes minutes, past the Bash tool's default 2 minutes: call it with a
  `timeout` of 600000 ms, or with `run_in_background` and wait for it.
  `--failed` reruns what the last run left red (its gates, the closure
  check, the merge after a conflict), `--only a,b` the gates named. A
  `base-red` gate is red on the base with the same errors: not yours, leave
  it. `skip` and `unreadable` lines are said, not failed on. Never rebuild a
  runner of the workflow by hand where the script exists.

## 6. Review

Run `/code-review` before calling the work done (`CLAUDE.md`, Workflow;
adversarial review, `Human_guidelines.md` §6). Point it at the branch's
full diff with the SPEC as the checklist: every requirement implemented,
edge cases covered, nothing outside scope changed. Its findings are
addressed before the PR merges, or explicitly declined with the reason
stated — quietly ignoring one is what this step exists to catch.

## 7. Land it

Branch → PR → merge, every task, no exceptions (`CLAUDE.md`, Repository
etiquette) — the merge being the harness's own, on both sides: a session
merges no pull request (`gh pr merge` is denied to it). The local
hook and the `main` ruleset that refuse a direct push are a backstop, not
something to test.

1. `git fetch --prune`, update **the integration branch** (`main` unless
   the execution context names another — in a loop run this is the
   current milestone's own branch, never `main` itself; rule 3 of the
   injected execution context says so explicitly), then branch
   `<type>/<slug>` off it, with the type from
   `.claude/skills/commit/SKILL.md` (`feat`, `fix`, `docs`, `style`,
   `refactor`, `test`, `chore`, `AIchore`).
2. **Stage by name. Never `git add -A`.** Section 2 told you which dirty
   files aren't yours; this is where that list gets spent. Check with
   `git diff --cached --name-only` before committing.
3. Commit per `/commit`'s conventions. Never commit secrets; every new env
   var is documented in the committed `.env.example`.
4. `gh pr create --base <the integration branch>` with **`Closes #<n>` on
   its own line in the body** — the issue number of the task. That line
   is what closes the task: no `Closes`, no closed issue, and the loop
   stops rather than replay a task that still looks open. A criterion an
   existing test proves gets its line there too, `AC4: covered by
   <path>[::<test>] — <why>`, and one no test can prove
   `AC3: not testable here — <why>`.
5. Stop here, on either side: open the PR with `--label harness:to-review`,
   report its number and end your turn. The harness watches its CI from here — it
   reviews, repairs and merges the PR itself (`milestone_merge`) and closes
   the task. Never wait on the checks: never poll `gh pr checks`, never a
   background watcher, never `Monitor`, never a sleep loop or a scheduled
   wake-up — a session silent for twenty minutes is killed, and it never
   comes back to its PR.
6. The task stays open under `harness:review-pending` until the harness
   merges the PR — expected, not a failure. A PR without the `Closes` line
   on its own leaves the task open once merged: the loop stops rather than
   replay it. Long local commands (the mutants, a full test run) are run
   in the foreground, before the PR.

## 8. Edge cases

- **The issue body is empty** (the scope block says so in words). Stop
  and say `/business-analyst` writes the SPEC there. Do not build from the
  milestone body or the issue title.
- **Already implemented.** If the working tree or `git log` shows the
  SPEC's artifacts already landed, report what exists and which
  Verification bullets confirm it — don't redo the work.
- **A `harness:human` blocker is still open.** You are blocked. Quote it
  and stop — closing it is the human's move, never yours.
- **The SPEC and the milestone body disagree** on what this task covers.
  The task issue is the contract; say the milestone contradicts it rather
  than taking the convenient reading.
- **The SPEC contradicts `CLAUDE.md`.** `CLAUDE.md` wins on repository
  etiquette and on the constraints that aren't visible in the code. Flag
  the contradiction; don't silently pick a side.
- **A Verification bullet fails.** Fix it if the fix lies inside the
  SPEC's scope. Otherwise stop and report — never move the goalposts.
- **`next-env.d.ts` shows as modified.** It flips between `next build` and
  `next typegen`. Run `git checkout -- next-env.d.ts` before staging;
  never hand-edit it.
- **Scope creep mid-task.** An adjacent improvement you spot is a note for
  the next `/business-analyst` run, not a commit on this branch.
- **Two strikes** (`Human_guidelines.md` §4). If the same correction lands
  twice in one session, stop and suggest `/clear` plus a fresh `/code` run
  carrying what you learned. Grinding on in polluted context costs more.

## 9. Hand off

End by telling the user:

- what landed — the merged PR and the files it changed;
- each Verification bullet with its result and the output proving it,
  including anything **not run** and why;
- anything left undone, and the named later task it belongs to;
- the SPEC's *before merging* human actions — theirs to run, not yours;
- that issue #n is closed (or, if it isn't, that the `Closes` line is
  missing and the task is not done) — on the write side, that PR #p is
  open under `harness:to-review` and the harness merges it.

This skill never closes an issue by hand and never touches a
`harness:` label.
