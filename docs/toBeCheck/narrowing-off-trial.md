# Narrowing off, on trial (ForgeDesMondes, 2026-10-17)

## Context

The product's Cargo workspace couples every system: the DataGuard server
names every DataCapability by path, an adapter its port, the conformance
runner reads the Concepts, crates include files across systems. A task's
narrowed tree (its closure) kept lacking what Cargo needs, and the session
stopped on "missing X": #31, #32, #38, #40, #41, #42, #48 (twice) — each
stop ~1–3 $ and 5–15 min (a session, a decision, a resume). Seven closure
gaps were fixed one by one; each new assembly showed another.

## Shipped

- The factory runs with `NO_NARROW=true` (`.env.local`), owner's choice:
  option 1, measure. Isolation stays checked after the fact:
  `ArchitectureHolds` still refuses a change outside the task's closure.

## Parked: option 2

Keep narrowing, but before paying the session run `cargo check` in the
narrowed tree, and widen to the whole tree when it fails — no paid stop,
1–3 min of build per task.

## To verify

- [ ] Over #18's first ~8 tasks: stops on a missing path — expected 0.
- [ ] Cost per task against #17 (median ~1.5–3 $ per `code` session):
      no rise beyond ~30 %, or narrowing comes back (option 2).
- [ ] How often a session ends under 100k context (the chain's threshold):
      with the whole tree, chaining may fire less.
