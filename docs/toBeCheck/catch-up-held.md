# A milestone branch that cannot take the base holds its tasks

**Status: on `main`, not yet seen live.**

## Context

Before a task starts, the watch merges the integration branch into the
milestone's branch (`branching::catch_up`). When GitHub refuses the merge
(a conflict), the watch only warned and started the task anyway, on a
branch that lacked what the base had: on ForgeDesMondes, right after
milestone #18 landed (2026-10-20 01:33Z), five sessions stopped at once on
"missing crates/dataguard/aggregates/…" ($3.40), and the wait rule read
three of those stops wrong (see `waits::in_path`, fixed in `6d2a440`).

## What shipped

- `branching::catch_up` says whether the branch holds the base now.
- `router::lanes::start_task`: a refused catch-up holds the task — one
  journal line a tick, `#n held: <branch> cannot take <base> — merge it by
  hand, the task starts then` — and the merge is tried again every tick.
- `router::routes::run_dev_loop` (the one-lane route) holds the milestone
  the same way.
- The conflicts themselves are still a hand's: regenerated files
  (`mock/pages.json`, the bundles, `queries/registry.json`, `Cargo.lock`)
  and the server's wiring. The next step is a harness route that resolves
  the regenerated ones itself and asks a session for the rest.

## To verify live

- [ ] A milestone branch in conflict with `main_agent` shows the held line
      once a tick and starts no session; the tick's cost is one merge call
      per milestone.
- [ ] Once the branch is merged by hand, its tasks start at the next tick
      without a restart.
