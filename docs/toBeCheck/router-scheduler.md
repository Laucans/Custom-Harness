# One weighted scheduler for every route

## Context

The router dispatched three ways (lanes filled first with tasks, splits and
business refinements; one paid route a tick to a lane, or in the watch's
process when no lane was free; the planner, the arbiter, the technical
refinement and the test debt always in the tick). On ForgeDesMondes,
2026-10-18: 244 of 636 ticks chose a review, 92 of them one already on a
lane; four green PRs took four ticks; #105, on the critical path, had no
priority over the ahead work.

## Shipped

- `79840be` — `routing::schedule` (every job of the snapshot, weighted: base
  per route, critical-path bonus, a point per tick waited, capped at 150) and
  `routing::plan` (every free job, one paid job per free lane, one per
  issue/PR, one technical refinement / test debt / plan at a time); the
  launcher executes it: merges in the tick, tasks as `--task`, anything else
  as `harness route <kind> <n>`. A quota rest stops the paid work only.
- `57f3166` — the mutants' tests; the tick's pace as a pure function.
- `a76c2eb` — one watch per checkout (OS lock `.llocal/watch-lock`); a
  review and a plan mount their lane's own checkout.

## Verified live (ForgeDesMondes)

- [x] 21:20Z — a merge run in the tick and two reviews started on two lanes
      in the same tick.
- [x] 21:33Z — a second `harness watch` refuses: "another watch already runs
      on this checkout".
- [x] 21:34Z — the test debt runs on a lane beside a review (it ran in the
      tick before, blocking it).

## To verify

- [ ] Two reviews side by side on their own `router-lane-<k>` checkouts, no
      "cannot widen the workspace" error.
- [ ] A decision or a PR of the critical path's milestone starts before the
      same kind of work of an ahead milestone (`schedule:` debug line, weights).
- [ ] During a quota rest, a green PR still merges (`tick: MergeIntoMilestone`
      while `resting: … the free work goes on`).
- [ ] A refinement or a test debt left waiting starts within ~150 ticks even
      with tasks always runnable (its weight in the `schedule:` line grows).
- [ ] No tick logs "already on a lane" any more.
