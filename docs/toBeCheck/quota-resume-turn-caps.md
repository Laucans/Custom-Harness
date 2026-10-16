# A session cut by a quota resumes; `code` is capped at 250 turns

## Context

`observations.md`: a `code` session cut at its 60th turn by a quota lost
4.32 $, and the next run paid 5.25 $ to untangle what it left. Ported from
the branch `claude/vertical-architecture-analysis-bxr497` (Phase 3), without
its per-task budget.

## What shipped

- The ledger row of a quota-cut turn keeps its session id; the next run's
  `PickTask` reads it (`Spending::interrupted`) and the `code` stage reopens
  that session with `--resume` instead of a fresh one.
- `code` opens with `--max-turns 250` (`TURN_CAPS`).

## To check

- [ ] A real run cut by the 5-hour quota mid-`code`: `costs.tsv` has a
      `QUOTA` row whose `session` cell is not empty.
- [ ] The next run says `stage code was cut short last time — its session
      … will be resumed`, and `session.log` shows `--resume <that id>` on
      its first turn.
- [ ] The resumed session carries on from where it stopped (no new plan, no
      re-read of every file): its turns and cost are well under a fresh one.
- [ ] A session far past its cap (`--max-turns`) ends with the CLI's own
      error, recorded, not a hang.
