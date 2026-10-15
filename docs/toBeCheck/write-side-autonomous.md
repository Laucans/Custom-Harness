# The write side merges without a gesture; delivery is told

## Context

Every write-side PR got `harness:needs-decision` on opening, so the board
showed "#n needs you" although `milestone_merge` merged it anyway. Nothing
told the human what was delivered, what landed on the integration branch,
or that a release was due. This is M4 of the plan "autonomous development
from the design dossier".

## What shipped

- No `needs-decision` on a write-side PR's opening. `PrOpened` is told
  instead ("merged once reviewed and green, close the PR to stop it").
- `TaskDelivered` is told for the read side (`MarkWaitingMerge`) and for the
  write side (`milestone_merge`). It takes back failing, parked, PR and
  repair signs.
- `RepairsExhausted` is told when `escalate` asks for a decision (once).
- `main_agent_merge` reports `Merged { pr, roadmap }` and emits:
  - `MilestoneLanded` (Warning: "ready to release");
  - `RoadmapDelivered` when the roadmap item's last milestone landed. The
    roadmap item is then labelled `harness:waiting-merge` and commented, and
    the planner skips it (`routing::plannable_roadmap`).
- "Ready to release" is taken back once `main` is where the integration
  branch is (`BoardReading.unreleased`, `Facts.unreleased`), or told from
  the branches alone when no event did.
- Rule 13, rule 5, the agents table and the docs no longer say a human
  merges the write side.

## To check on a real run

- [ ] A write-side task: its PR opens with `harness:to-review`. The task gets
      `review-pending` and no `needs-decision`. Notification "#t: PR #p open
      on the write side".
- [ ] After a clean review and green checks, `milestone_merge` merges and
      closes the task. Notification "#t delivered", and the PR sign is gone.
- [ ] A milestone PR merged into `main_agent`: notification "#m landed —
      ready to release". After `main` is fast-forwarded to `main_agent`, the
      sign disappears on the next board read.
- [ ] The last milestone of a roadmap item: the roadmap item gets
      `harness:waiting-merge` and a comment. A newer `harness:ready` roadmap
      item gets planned without closing the old one.
- [ ] GitHub closes the milestone through `Closes #m` (main_agent is the
      default branch). The release sign stays until `main` catches up.
