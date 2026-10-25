# Every task carries GitHub's native Milestone field

## Context

Owner, 2026-10-25: every task must carry GitHub's native Milestone field,
set when it is opened. A backfill already ran on ForgeDesMondes
(`harness-dnd2-run/tools/native-milestones.py`): one native milestone per
`harness:milestone` issue, titled `#<n> <issue title>` (255 characters at
most), its description carrying `<!-- harness-milestone: #<n> -->` (how it
is found again), its state mirroring the issue; every sub-issue put on it.

Now the harness keeps it true by itself, with the same convention
(`harness_workflows::common::native_milestone`, core's
`GitHub::ensure_native_milestone` / `set_issue_milestone`, `GhCli` through
`gh api`). The sub-issue link stays the harness's own truth; a refusal of
the native field is a journal warning, never a failed opening. Green tests
only (fakes): no live split, stop or landing has run with it yet.

## What shipped

- Not committed yet: core port, `GhCli`, `FakeGitHub`, the openers (split,
  `missing_task`, the stop handler's human tasks, the contract manager's
  producer task, the view's `Product::open_task`), the landing close
  (`main_agent_merge`, `milestone_merge::run` on a milestone, `decide`'s
  cut of a milestone).

## To verify

- [ ] **Next split**: the milestone split shows a native milestone titled
      `#<n> <title>` (Issues → Milestones), description starting with the
      marker; every task of the split shows it in its Milestone field. No
      second native milestone for a milestone the backfill already mirrored
      (found by the marker: exactly one per milestone issue).
      `grep "native milestone" .llocal/logs/*` shows no warning.
- [ ] **Next missing task** (journal: `opened #… for the missing task`):
      the new task carries its milestone's native milestone, as does a
      `decide` approval's task (`task` action) and a human task the stop
      handler opens (`Decide for #…`, `Approve the port …`, `Missing task
      for #…`).
- [ ] **Next contract change** with a producer task: the task is on the
      producer's (or the requester's) native milestone.
- [ ] **Next landing** (`main_agent_merge: #<n> landed by PR …`): the
      native milestone reads closed while the milestone issue stays open
      with `harness:waiting-merge` (the landing merges into `main_agent`,
      not the default branch, so `Closes #<n>` does not fire there). The
      issue closes at promotion to the default branch. No `watch:
      main_agent_merge -> #<n>'s native milestone could not be kept`
      warning.
- [ ] A task opened under a landed milestone (open, `harness:waiting-merge`
      — a contract manager's producer task, a conformity task from the
      view's `open_task`) is put on its native milestone, which stays
      closed.
- [ ] A refusal reads right: if the token lacks the right (issues write is
      enough for milestones), the journal says `warning: #<n>'s native
      milestone could not be kept: …` and the task is opened and linked all
      the same.
- [ ] A milestone issue reopened by hand: its native milestone reopens at
      the next task opened under it (not before — nothing polls it).
- [ ] Cost: one `milestones?state=all` read (one page under 100
      milestones) per opening, plus one PATCH per task; the quota stays
      within what the watch's journal reports as usual.
