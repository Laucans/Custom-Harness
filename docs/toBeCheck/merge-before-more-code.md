# Merging comes before new code

## Context
2026-10-20, ForgeDesMondes at 5 lanes. Five task PRs (#272, #274, #280, #304,
#307) had a clean review but a red `rust` check and no `harness:pr-fix` label.
No route repaired them, some for 9 h, while dev tasks kept opening new PRs.

## What shipped
- The router's `red_prs` also takes a to-review PR whose checks fail, while it
  has repairs left: a clean review no longer hides a red CI.
- `milestone_merge::ready_to_merge` escalates a red PR past `MAX_FIXES` to a
  decision instead of leaving it open.
- WIP limit: when the open task PRs (base `milestone/*`) reach the lane count,
  the snapshot sets `review_backlog_full` and no DevLoop/DevTask is routed.
  Reviews, repairs and merges still run.

## To verify on a live watch
- [ ] The log shows `watch: N task PRs open for L lane(s) — no new task …`
      while N >= L, and no new task starts meanwhile.
- [ ] Red PRs reviewed clean get a pr-fix route (journal: `#N: repair K`).
- [ ] A red PR with 2 repairs done gets a `## Decision` comment, not a third
      repair.
- [ ] Open task PRs go down within ~2 h (from 10+ to <= 5).
- [ ] Lanes do not sit idle once the backlog is below the limit.
