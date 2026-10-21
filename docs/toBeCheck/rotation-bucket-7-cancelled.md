# Rotation bucket 7 cancels the runner

## Context
The harness CI run #71 (2026-10-21, commit 80abb2d, A3) went red on
`mutants-rotation` alone. The step `mutants-rotation.sh 71` was *cancelled*
about five minutes in, twice: the first attempt and a rerun. The logs are
unavailable, as for a runner lost or killed. The rotation is designed never
to fail on what it finds, so a cancelled runner is a different failure. It
is likely a mutant that fills the disk or the memory, for example a logbook
that never stops writing.

Bucket 71 mod 64 = 7 holds files A3 never touched:

- crates/core/src/traces/logbook.rs
- crates/launcher/src/dispatch/test_debt.rs
- crates/view/src/adapters/mockups.rs
- crates/view/src/adapters/sqlite_usage.rs
- crates/workflows/src/data_diagnosis/data/mod.rs
- crates/workflows/src/init_repo/ports.rs
- crates/workflows/src/main_agent_merge/run.rs
- crates/workflows/src/pr_review/action/mod.rs
- crates/workflows/src/pr_review/orchestration/workflow.rs
- crates/workflows/src/test_debt/orchestration/round.rs

Every 64th run lands on this bucket again.

## To check
- [ ] Run the bucket locally:
      `bash scripts/mutants-rotation.sh 7 -- --in-place --output /tmp/rot7`
      Use the rustup toolchain on PATH, and watch disk and memory. Find the
      mutant that blows up.
- [ ] Bound it: give the step `timeout-minutes`, and make sure a runner
      that is killed files a test-debt issue instead of turning the run red.
      Or add a test that catches the mutant before it runs away.
      The step now has `timeout-minutes: 90` (the CI-speed commit, which
      also gave the rotation nextest and no debug info); a step cut by it
      still turns the run red and files nothing — that part stays open.
