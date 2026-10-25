# One read of the open PRs, the board kept while quiet

**Status: on `main`, deployed on ForgeDesMondes 2026-10-20, not yet measured live.**

## Context

The factory's token (5 000 GitHub calls an hour) ran out twice on
2026-10-20 (04:25Z, 05:32Z) at 5 lanes and a 60 s tick: while any PR
waits on its CI, the watch read the whole board every tick — each open
PR's checks, mergeability and comments one request at a time, each
building milestone's sub-issues, each task's blockers. Measured at 4 lanes
and 120 s: 162 calls in 10 minutes.

## What shipped (harness-adapters, `GhCli`)

- `3a2305b`: one `gh pr list --state open` (CI rollup, mergeability,
  update time, head) answers `pr_checks_green`, `pr_failing_checks` and
  `pr_mergeable` for 45 s; any write drops it; an `UNKNOWN` mergeability
  is still asked of the PR. `pr_comments` is read again only once the PR's
  update time moved. A merge is pinned to the head whose checks were read
  (`--match-head-commit`).
- the board's issue reads (`issues/…`: labelled lists, sub-issues,
  blockers) are kept while the activity read answers 304, at most 10 min,
  dropped by any write; a process that never reads activity (a lane)
  keeps nothing.

## To verify live

- [ ] `gh api rate_limit` `core.used` over 10 min with PRs under CI: well
      under 100 (was 162 at 4 lanes / 120 s).
- [ ] No merge of a PR whose head moved after its checks were read
      (a refused merge says `--match-head-commit`, and the next tick
      re-reads).
- [x] A task closed or a label changed by a lane is seen by the watch at
      its next tick (the activity read changes, the kept board is dropped). (seen 2026-10-24 18:04 Paris: decide's lane marked #486 decided at 18:04:08; the next tick, 18:04:29, offered `DevTask` #486)
- [x] Back to `--parallel 5 --interval 60`: no `rate limit exceeded` in
      `watch.log` over two hours. (seen 2026-10-25 01:40 Paris: no `rate limit exceeded` in watch.log after 2026-10-20 07:38 Paris; the watch ran every 60 s on lanes 0–4 through 21–24 Oct)
