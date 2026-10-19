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

## To check on a real run

- [ ] Quiet cost: a green base adds **two** `gh` calls per full read
      (`gh run list`, `gh issue list -l harness:human`); a quiet tick still
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
