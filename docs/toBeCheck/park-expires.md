# A parked task is retried after an hour

## Context
ForgeDesMondes, 2026-10-22 15:12Z: #405's lane halted on preflight
`MilestoneIsReachable` (milestone #25 "not open"). Exit 1 parked the task
until its issue changed. #25 reopened, #405 never changed, and the router
picked #405 as its top job every tick while the park held it, at debug
level only: the factory sat idle from 20:28Z to 21:05Z on 2026-10-23, and
on every quiet stretch before, until the watch was restarted by hand.

## Shipped
- `crates/launcher/src/dispatch/lanes.rs`: a park lasts until the issue
  moves or `PARKED_FOR` (one hour) passes.
- `crates/launcher/src/router/lanes.rs`: "parked until its issue changes or
  an hour passes" is said, not debugged.

## To check
- [ ] On the next park in a live watch, `watch.out` says the park each tick
      and the task is taken again about an hour later.
- [ ] A SPEC-question stop retried after an hour halts again cheaply (one
      preflight, no paid session): count its cost in the next run.
