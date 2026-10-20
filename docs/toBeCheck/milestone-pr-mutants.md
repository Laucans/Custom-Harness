# A milestone's pull request skips the in-diff mutants step

**Status: on `main`, seen once live (ForgeDesMondes PR #268), the skip itself not yet seen.**

## Context

`ci.yml`'s `Mutants` step runs `cargo mutants --in-diff` on the change
measured against the base. On a task's PR the diff is one unit. On a
milestone's own PR (`milestone/<n>-…` into the integration branch) it is
the whole milestone: ForgeDesMondes' #18 (12 aggregates, 8 capabilities, 3
DataCapabilities, the server's modules) ran the step for more than 75
minutes on 2026-10-20 while the critical path waited. Every task PR of the
milestone had already passed the same step on its own diff.

## What shipped

- `crates/architecture/assets/ci.yml`: the `Mutants` step is skipped when
  `github.head_ref` starts with `milestone/`; the mutants rotation (a
  bounded bucket of files, never red) still runs on every job.
- The product's `.github/workflows/ci.yml` got the same change by hand on
  `main_agent`, merged into `milestone/18`.

## To verify live

- [ ] On the next milestone PR, the `rust` job shows `Mutants` as skipped
      and the whole job ends in the time of the rotation (minutes, not
      hours); the PR's other checks are unchanged.
- [ ] A task PR still runs `Mutants` on its diff (the step not skipped).
- [ ] `harness upgrade` at the next watch start reports `ci.yml` as
      carried already (the product's copy equals the asset), not diverged.
