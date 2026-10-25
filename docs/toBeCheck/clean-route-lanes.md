# A route leaves its lane checkout clean; a read-only route resets a dirty lane that holds nothing unpushed

## Context

Under `--parallel`, every paid route runs on a lane with one checkout per
lane (`.llocal/agentic_workspaces/router-lane-<k>`, commit `a76c2eb`), so a
writing tenant (`pr_fix`, `rules`) and a read-only one (`pr_review`,
`planner`, `split`, a refinement, `decide`, `arbiter`) take turns in the same
clone.

ForgeDesMondes, 2026-10-18 23:09Z: a `pr_fix` on PR #260 ran in
`router-lane-1`, merged the milestone branch, hit the known conflict on
`crates/dataguard/server/src/lib.rs`, ended on exit 0 and left the merge
unresolved (`UU lib.rs`, 5 added files). The unmount only forced a checkout of
the PR's branch — which does drop the merge state and the staged files, but
not the untracked ones — and the next job on the lane, the `pr_review` of
#261, mounted it read-only (no `--force-reset` on purpose) and halted:
"the workspace router-lane-1 carries 2 uncommitted change(s) … re-run with
--force-reset". `router-lane-3` was in the same state. A lane that halts on
a leftover halts for every job after.

## Shipped

- `5c68274` — `Provisioner::release` (core): every router tenant gives
  its checkout back through it (`dispatch/shared.rs::release_checkout`): an
  unfinished merge aborted (`Repo::merge_in_progress` / `Repo::abort_merge`,
  two new port verbs, `git rev-parse MERGE_HEAD` / `git merge --abort` in
  the adapter), fresh refs, then — when nothing on it is unpushed — the
  branch back at `origin`'s state, untracked files and scratch branches gone.
  When commits no remote has (or a stash) are on it, nothing is reset and
  `Event::WorkKept` goes to the journal (a warning line, a `work-kept:<id>`
  notification in the view).
  `Wanted::drop_uncommitted` (true for every router checkout, false for
  `dev_loop`'s): a read-only mount that still meets uncommitted leftovers —
  a killed run never releases — drops them and says so instead of halting;
  it halts only on commits no remote has, naming the branch and how many
  (`the workspace "router-lane-1" carries 1 unpushed commit(s) on feat/x …`).

## Found on the live lanes (2026-10-19, read only — nothing done)

- `router-lane-1`: on `feat/battlemap-instance-aggregate`, clean tree, no
  `MERGE_HEAD`, **4 commits in `@{u}..HEAD`** (`3e07a57` a merge of
  `origin/milestone/18-…` into the feature branch, plus three the milestone
  branch already holds). Not the brief's case (no `UU`, unpushed non-empty),
  so untouched. Under the new code: a `pr_fix` on it (`--force-reset`) resets
  it; a read-only tenant halts naming `1 unpushed commit(s) on
  feat/battlemap-instance-aggregate` until someone pushes or drops that
  merge commit.
- `router-lane-3`: on `main_agent`, clean, nothing unpushed, no `MERGE_HEAD`.

## To verify live

- [x] A `pr_fix` that hits the dataguard `lib.rs` conflict ends with its
      lane's `run.log` saying `workspace router-lane-<k>: an unfinished merge
      aborted` then `… N uncommitted change(s) the run left — dropped, nothing
      on it is unpushed`; `git status --porcelain` in the lane is empty right
      after, `git rev-parse --verify MERGE_HEAD` fails. (seen 2026-10-20 16:52 Paris: pr-fix 20261020-142015 on #304's conflict — `an unfinished merge aborted`, `2 uncommitted change(s) the run left — dropped`, `back on origin/feat/soundbox-stop-all, clean`; a #304 conflict, not the dataguard lib.rs one)
- [x] The next read-only job on that lane (a review, a split) mounts with no
      `--force-reset` halt; the tick's `LaneEnded` for it is exit 0. (seen 2026-10-20 08:27–08:35 Paris: after pr-fix 20261020-062109 aborted a merge on router-lane-3, decide #157 then pr-review #271 ran there, both exit 0)
- [x] A lane left dirty by a killed process (kill -9 a route child mid-session,
      then let the watch pick the lane again): its next run's `run.log` says
      `workspace router-lane-<k>: N uncommitted change(s) a previous run left —
      dropped, nothing on it is unpushed` and goes on. (seen 2026-10-22 07:50 Paris: pr-review 20261022-055017 — `router-lane-1: 17 uncommitted change(s) a previous run left — dropped`, the review went on; a leftover after a release, not a kill -9)
- [ ] `router-lane-1` as found above: the first read-only route on it halts
      with `carries 1 unpushed commit(s) on feat/battlemap-instance-aggregate`
      (not "2 uncommitted change(s)"); after `git push` or `git reset --hard
      @{u}` in the lane, the next one runs. (partly seen 2026-10-19 23:48 Paris: a pr_review halted naming `feat/battlemap-instance-aggregate` on router-lane-0 (`carries work on 1 branch(es) origin does not have`), not `uncommitted`; the run after a push or reset not seen)
- [x] A `pr_fix` whose push fails (e.g. the PR branch deleted meanwhile): its
      journal carries `warning: workspace router-lane-<k> kept as it is: N
      unpushed commit(s) on <branch> — it is at …`, the view shows "Work kept
      on router-lane-<k>" (warning), and the next tenant halts naming it. (seen 2026-10-24 17:54–18:01 Paris: pr-fix 20261024-155017/155605 — `kept as it is: 1 unpushed commit(s) on feat/ai-analyze-brief`, WorkKept recorded, the next decide halted naming it; the view not checked)
- [ ] `harness trial` and the shared `router-readonly` checkout (one lane):
      unchanged behaviour, no new line in their logs beyond the debug
      `back on origin/<branch>, clean`. (not seen: router-readonly unused since 17 Oct)
