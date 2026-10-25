# Re-study after two discoveries, sibling chaining, X

## Context

Owner, 2026-10-25 15:31: "Fais l'ensemble". Live evidence on ForgeDesMondes
(`harness-dnd2-run/DECISIONS.md`, `night/priorities-H.md` V and X, design
`night/restudy-chaining-x-design.md`):

- #219 stopped six times in one day, each stop declaring the next missing
  units: a journey stops at its first failure, which hides the next one.
- #567/#568 came from one stop; #568 needed `session.updateSceneNotes`,
  which #567 produces, was opened unchained, ran first and halted on
  `ArchitectureHolds` until chained by hand.
- X: milestone #27's landing PR #562 past its repairs escalated onto the
  milestone (`harness:needs-decision`), but `MergeMainAgent { 27 }` (free,
  990) and `Decide { 27 }` (paid, 860) share the key 27: the merge won every
  tick and re-escalated ("Escalated { pr: #562, asked: false }" for 75 min).

Green tests only (fakes): no live re-study, chain, not-needed close or
landing decision has run yet.

## What shipped

One commit on branch `feat/restudy-chaining-x` (worktree
`../harness-restudy`, off `d0a741a`), "feat(restudy): a 2nd discovery
triggers a re-study; …" — not merged into `main` yet.

Gates on the commit's tree: fmt, clippy `-D warnings`, workspace tests,
machete, dead-code green; `cargo mutants --in-diff` (diff from `d0a741a`):
298 mutants, 247 caught, 51 unviable, 0 missed, 0 timeout (83 min, `-j 1`).

- X: `routing::in_flight` schedules no `MergeMainAgent { m }` while an open
  `harness:needs-decision` issue is milestone `m`; `decide` on a landing
  PR: `repair` or `wait` only (an `answer` refused: no session reads a
  milestone's `## Decision`).
- Chaining in `common::missing_task::open_or_find` (every opener: a stop,
  `decide`'s `task`, a re-study): `needs` ∩ `produces` between the slices
  of one declaration, then the kin opened for the same waiting task.
- `GitHub::close_issue_not_planned`; `decide`'s `cut` uses it; an opened
  task's brief asks it to check it is still needed, and
  `AGENT_LOOP_STOP: not-needed — <evidence>` on a harness-opened task closes
  it as not planned.
- The streak on `## Decision` (`discovered:`, `restudied:`, `restudies:`),
  counted by stops that opened a task; `Route::ReStudy` (720) once due; a
  due task is not started — the shared picker skips it (`tasks::next_task`
  for a sequential dev loop, `tasks::startable_tasks` for a lane).
- Every opener parks the waiting task on each slice as soon as it is opened
  or found, so a publish that fails halfway leaves it waiting on those.
- An open task the re-study declares again by its branch, with a new id in
  its `needs`, is found and chained on the new slice (the prompt asks it).
- `RESTUDY_CONTEST` may span several lines (pretty-printed JSON).
- The `restudy` workflow (opus, its lane's own checkout of the milestone's
  branch, runtime ports `20000 + (task % 5000) * 2`), `harness route
  restudy <n>`, the view's `restudy` line, event `Restudied`.

## Migration (supervisor, once deployed)

- [ ] #219 waits on #567/#568 under a `kind: wait` written before this lot
      (no streak). Add two lines to its `## Decision`: `discovered: 2` and
      `milestone: #28`. Without them, #219 needs one more stop before its
      first re-study. No other task is due at deploy (all at 0).
- [ ] Tasks `decide` cut before this lot were closed *completed* with no
      merged PR, and `every_task_merged` waits on them. Read-only first:
      `gh issue list --state closed --search "Cut by the harness in:comments"`
      (or the comments starting "Cut by the harness:"); any one under a
      milestone still building is reopened and closed as not planned
      (`gh issue close N --reason "not planned"`).

## Known limit (by design)

The re-study takes its streak (`restudied:`, `restudies+1`) **before** its
paid session. A watch restart or SIGKILL mid-study, a failed gate, or a
GitHub error in `publish` uses that re-study up: the task is no longer due,
resumes as a plain task, and the cap counts it. The next re-study comes
only `EVERY` (3) discoveries later, within `MAX` (2).

## To verify

- [ ] **#219's re-study** runs within one tick of the edit above
      (journal: `tick: ReStudy { task: 219 }`, then `restudy #219 …`,
      `#219: the re-study is taken (restudied: 2, 1/2)`). Its run log
      (`.llocal/logs/restudy/<run>/stream.jsonl`) shows the runtime served
      on 20438/20439 and a curl probe for each read and command of R11, R13
      and R15; `rules-runtime.sh stop` before the answer.
- [ ] It opens N tasks in one publish (journal `opened #… for the missing
      task` ×N), chained by ids — at least one `TaskWaits … needs \`…\`,
      which #… builds` — and #219 is blocked on all of them (`kind: wait`,
      `wait — until #a, #b, … land`, `restudied: 2`, `restudies: 1`).
      Event line `#219 re-studied: waits on #…`.
- [ ] R15's scroll-dependent assertion is in the contest note under #219's
      `## Decision` ("Contest these rule tests with CONTEST.md …").
- [ ] Cost ≤ $5 (`costs.tsv`, stage `study`), time ≤ 40 min.
- [ ] #219 does not stop on a missing unit again before 2 more discoveries
      (next re-study only at `discovered: 5`, then never past
      `restudies: 2`). Its checkout is untouched (no `WorkKept` on the lane
      after it; the gate "study must answer" passes).
- [ ] An opened slice already satisfied closes itself: journal
      `#N: not needed — closed as not planned, #219 no longer waits on it`,
      the evidence commented on #N and #219, #N closed with reason "not
      planned". Declared again, it is refused ("closed as not needed …
      with the probe that shows it missing").
- [ ] The next landing PR past its repairs gets `decide` within one tick
      (no `Escalated { …, asked: false }` line for more than 2 ticks): the
      decision is `repair` or `wait`, at most 2 rounds, then
      `harness:human` on the milestone. Once decided, `MergeMainAgent`
      comes back the next tick.
- [ ] The next multi-slice stop whose slices share an id (needs ∩
      produces) gets its `blocked_by` written: journal `TaskWaits … needs`.
- [ ] When #219's re-study finds a slice an open sibling (#568-like) needs,
      it declares that sibling again by its branch: journal `the missing
      task is #568 already`, then `TaskWaits` on #568 for the new slice; no
      code session of #568 stops to re-declare it.
- [ ] While #219 is due (between the stop that made it due and the
      re-study), no `dev-loop` round picks it (sequential watch) and no
      lane starts it.
- [ ] If a re-study is interrupted (restart mid-study), the journal shows
      no second `ReStudy { task: 219 }` for the same discovery, and #219
      runs again as a plain task (Known limit above).
- [ ] The next `decide` cut closes the task as not planned (GitHub shows
      "Closed as not planned"), and its milestone still lands.
