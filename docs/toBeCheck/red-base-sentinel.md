# Nothing merges on red, and a red integration branch holds the plant

## Context

ForgeDesMondes, 2026-10-18 (journal `../harness-dnd2-run/forgedesmondes-journal.md`,
DECISIONS.md "Bugs ouverts" 2 and 4):

- `decide`'s `merge` called `milestone_merge::run` as soon as the PR was
  mergeable, never looking at its checks: PR #231 (a rules contest) merged
  at 14:37Z with 7 red checks.
- `main_agent`'s own CI was red from 14:24Z for about two hours (an orphan
  manifest pushed on it); the watch kept starting tasks whose milestone
  branch takes `main_agent` first (`branching::catch_up`), and opening PRs
  that inherited the red; nobody was told.

Out of scope, still: protecting `main_agent` (the owner's decision), repairing
the base itself, the view beyond compiling.

## What shipped

On the branch `worktree-wf_b56293a2-7f9-2`, to merge into `main` locally:

- `4c20cd8` — **`GitHub::branch_checks(branch) -> Option<bool>`**:
  `gh run list --branch <b> --limit 20 --json headSha,status,conclusion,name`,
  the runs of the newest head sha only (the first row's — `gh` lists the
  newest run first); `Some(false)` on any `failure` / `timed_out` /
  `startup_failure` / `action_required`, `None` while one runs or none ran
  (or all cancelled / skipped), `Some(true)` on at least one `success` and
  nothing but `success` / `skipped` / `neutral`. `FakeGitHub.branch_checks`
  (absent branch: `None`); every other fake answers `None`.
- `59ab778` — **`Event::BaseRed { branch, sha }`** (warning) and
  **`Event::BaseGreen { branch }`**, their journal lines and serde
  round-trip; `notify` tells `base-red:<b>` ("Integration branch <b> is red")
  until `BaseGreen` clears it.
- `09f8a15` — **the merge itself looks at the checks**:
  - `milestone_merge::run` answers `Attempt::{Merged, Pending, Red(checks)}`
    and writes nothing on `Pending` or `Red`, whoever calls it; the watch
    says which (`waits: a check … is still running` / `refused: n failing
    check(s)`).
  - `decide`'s `merge`: a conflict or a red check is a repair (comment with
    `review::DECIDED_MARKER` + `harness:pr-fix`, repairs counted anew, as
    before); checks still running merge nothing now — the PR gets a comment
    carrying `review::MARKER` + `review::CLEAN` + `review::DECIDED_MERGE`
    (it reads as reviewed-clean, so the merge train — `tasks_to_merge` —
    takes it once green instead of asking for a decision again), the task
    gets "merges when its checks are green", the decision is written as a
    `merge`, `harness:needs-decision` is not posed again.
  - `main_agent_merge` tells a red milestone PR apart from one still
    running (`Outcome::Red { pr, checks }`), still poses `harness:pr-fix`
    on it as `main` did, and the watch warns every tick it holds.
- `3331b9a` — **the sentinel** (`harness_workflows::common::sentinel::watch`),
  called by the watch's tick once per **full read**, before the snapshot
  (a quiet tick reads nothing, by design; a red base then keeps the full
  reads going every tick, `Pulse.watching_ci`, since the push that turns it
  green moves no issue):
  - red → journal line `integration branch <b> is red: development and
    merges wait` every full read; one `harness:human` issue titled exactly
    `Integration branch <b> is red` (body: the head sha, what the harness
    holds, what to look at), found again by that title among the open
    `harness:human` issues — never a second one, across ticks or restarts;
    `BaseRed` once per red spell;
  - green → that issue commented and closed, `BaseGreen`;
  - `None` → nothing; `--dry-run` → the read and the line, no write.

  **The hold** is in the scheduler (`routing::schedule`, pure): on
  `Snapshot.base_checks == Some(false)` (`routing::base_red`) neither
  `in_flight` lists `MergeIntoMilestone` / `MergeMainAgent` nor `to_build`
  lists `DevTask` / `DevLoop`, so `plan()` starts none of them under
  `--parallel` and `decide()` never routes one; `PrFix`, `PrReview`,
  `Rules`, `Decide`, `TestDebt`, `ContractChange`, `Split`, `Planner`,
  `Refinement`, `Arbiter` go on. `saw: base=Some(false), …` in the debug
  line.
- `35cb8bf` — **the train takes a rules PR the harness decided to merge**
  (review finding): a rules' task's PR (head `rules/<n>`, labelled
  `harness:rules`) sits on no board, so the `merge_when_green` leg above
  left it to nobody — and `routing::rules_first` held every task on the
  open rules' issue. Now `milestone_merge::rules_ready_to_merge` (since the
  contest-flow lot: `rules_prs`, `RulesPr::Mergeable`; read by
  the snapshot only while a rules' issue is open: one
  `gh pr list --label harness:rules`, then the comments, mergeability and
  checks of each candidate) offers the PR whose last review is the decision
  (`review::decided_to_merge`: `MARKER` + `CLEAN` + `DECIDED_MERGE`, no
  repair after it), mergeable and green, as a `MergeIntoMilestone` job with
  `base` = the integration branch; the merge closes the rules' issue, which
  frees the development. A rules PR with no decision, even reviewed clean,
  is never taken: its own judge merges it.
- `be9c52d` — **the hold lasts until green**: on `None` (runs in
  progress, or none) the sentinel reads the open `harness:human` issues and,
  while `Integration branch <b> is red` is open, answers `Some(false)` and
  says `integration branch <b> is not green again yet: its checks run —
  development and merges wait`. Before, the fix's push on a red base lifted
  the hold for the minutes its runs were queued, and `catch_up` could merge
  the unverified base into a milestone branch. A human who closes the issue
  by hand lifts the hold. Also: the sentinel's doc says "once per full
  read"; the `run_milestone_merge` journal line names the base merged into.

## To check on a real run

- [ ] Quiet cost: a green base adds **two** `gh` calls per full read
      (`gh run list`, `gh issue list -l harness:human`); a base with runs in
      progress and no red before adds the same two; a quiet tick still
      reads nothing beyond the activity probe. Count them in the watch's
      journal at `--verbose`.
- [ ] `gh run list --branch main_agent --limit 20 --json
      headSha,status,conclusion,name` on the product's repo: the first row is
      the newest run (the assumption `branch_checks_from` makes to pick the
      newest sha); 20 rows hold every workflow of the last push.
- [ ] Push something red on `main_agent` directly (or merge a red milestone
      PR by hand):
  - within one full read (at most `FULL_READ_EVERY` = 10 ticks, 5 min at
    the default interval, less if the board moves), the journal says
    `integration branch main_agent is red: development and merges wait`
    and `watch: integration branch main_agent is red at <sha> — …`;
  - **exactly one** issue `Integration branch main_agent is red`, labelled
    `harness:human`, body naming the sha and `gh run list --branch
    main_agent`; the notification "Integration branch main_agent is red"
    (warning) on the view;
  - the next ticks: the line again, **no second issue**, `saw:
    base=Some(false)` in the debug lines; no `tick: DevLoop`, `tick:
    DevTask`, `tick: MergeIntoMilestone`, `tick: MergeMainAgent` and no
    `schedule: …` entry naming one; under `--parallel`, no `LaneTaken`;
  - a `harness:pr-fix` PR or a `harness:to-review` PR is still dispatched;
    a `harness:needs-decision` issue is still decided;
  - restart the watch while red: still no second issue.
- [ ] Fix the base (push green): within one full read, the issue is
      commented "green again" and closed, `BaseGreen` in the journal, the
      notification gone, and the next tick schedules a `DevTask` / a merge
      again.
- [ ] A task PR whose checks are still running when `decide` answers
      `merge`: no merge, the PR carries `<!-- harness-decided: merge -->`
      with the clean verdict marker, the task says "merges when its checks
      are green", `harness:needs-decision` is **not** posed again, and the
      next tick after the checks turn green routes `MergeIntoMilestone`
      (the train), which merges and closes the task.
- [ ] A task PR with a red check when `decide` answers `merge`: the PR gets
      `harness:pr-fix` and the decision marker; `pr_fix` runs once more.
- [ ] A milestone PR red on `main_agent`: `watch: main_agent_merge -> PR #n
      of #m is red: k failing check(s), its repair asked — …` every tick,
      nothing merged, no `harness:waiting-merge` on the milestone,
      `harness:pr-fix` on the PR.
- [ ] A task PR the router read green that turns red between the read and
      the merge: `watch: milestone_merge -> #n refused: …`, nothing merged.
- [ ] A rules' issue decided `merge` while its PR's checks ran (the live
      case: the judge poses `harness:needs-decision` right after an
      unfaithful second round): the PR carries the decision comment, the
      issue says "the watch merges it at the first tick it is green and
      mergeable"; once green, `saw: … to_merge=[<rules n>]`, `tick:
      MergeIntoMilestone { task: <rules n>, pr: "<pr>", base: "main_agent" }`,
      `watch: milestone_merge -> #<rules n> merged into main_agent`, the PR
      merged, the issue closed with "merged by `milestone_merge`", `#<n>
      delivered` on the view, and the next tick schedules a `DevTask` (the
      rules no longer hold). While the rules' issue is open: **one** extra
      `gh pr list --label harness:rules` per full read, plus three reads per
      candidate PR; none when no rules' issue is open.
- [ ] A rules' issue decided `merge` whose PR turns red afterwards, or
      conflicts: nobody takes it (the train skips it, no `harness:pr-fix`
      posed) — the development stays held on the rules' issue. Known gap,
      the owner's call: pose the repair, or escalate once.
- [ ] After a red, push the fix on `main_agent`: while its runs are queued
      or in progress, the journal says `integration branch main_agent is not
      green again yet: its checks run — development and merges wait` each
      tick, `saw: base=Some(false)`, no `DevTask` / merge scheduled, the
      issue still open; once the runs pass, the issue is closed and the jobs
      resume. One `gh issue list -l harness:human` per tick meanwhile.
- [ ] A product whose `gh run list --branch main_agent` errors (Actions
      disabled, the API refusing): every tick fails with `TickFailed` and
      dispatches nothing — the sentinel's read is a routing read, and a
      failed routing read stops the tick. Products are initialized with CI,
      so this should never show; if it does, the read should answer `None`
      instead.
