---
name: code
description: Implement the task whose SPEC is the body of its harness:agent issue, end to end — pre-flight gate (open harness:human blockers, dirty working tree, stale assumptions, missing inputs), a checklist from the spec's Approach, the build, every Verification bullet run with real output, /code-review, then branch → PR carrying `Closes #N` → gh pr merge --rebase. Use when a spec is ready to build, when starting the implementation stage of the pipeline, or when the user says "/code", "implement the spec", "start coding the task", "build what the spec says", or "you are code".
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
  have offered you this task. The SPEC's `Human actions` lines for *while
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

## 6. Review

Run `/code-review` before calling the work done (`CLAUDE.md`, Workflow;
adversarial review, `Human_guidelines.md` §6). Point it at the branch's
full diff with the SPEC as the checklist: every requirement implemented,
edge cases covered, nothing outside scope changed. Its findings are
addressed before the PR merges, or explicitly declined with the reason
stated — quietly ignoring one is what this step exists to catch.

## 7. Land it

Branch → PR → merge, every task, no exceptions (`CLAUDE.md`, Repository
etiquette). The local hook and the `main` ruleset that refuse a direct
push are a backstop, not something to test.

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
   stops rather than replay a task that still looks open.
5. Wait for `ci` with `gh pr checks`, then `gh pr merge --rebase` — the
   only method enabled; the branch auto-deletes.
6. Confirm GitHub did close it: `gh issue view <n> --json state`. Nobody
   closes it by hand, so a PR merged without the `Closes` line leaves the
   task open and the round unfinished.

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
  missing and the task is not done).

This skill never closes an issue by hand and never touches a
`harness:` label.
