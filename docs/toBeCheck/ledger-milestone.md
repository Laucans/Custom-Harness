# The ledger names the milestone of each spend

## Context

Control room, *Spending*, *By milestone*: the `unknown` bucket weighed about
$960 of $1 460 (66 %). The view rebuilt a row's milestone from the board,
and the board read keeps only the open milestones and the last three closed
(`CLOSED_KEPT`), so every task of an older milestone fell into `unknown`.
The owner's decision: the ledger (`.llocal/logs/agent-loop/costs.tsv`) is
the one register, and the milestone of a spend is written where it is paid.

Diagnosis of the odd task keys (`#2`, `#423`, … about $260): not a broken
write. The dev loop bills a task under its bare number (`Issue::key`), every
other workflow under its reference (`#N`: refinement, split, planner,
decide, arbiter, rules, test debt, contract manager, PR fix); PR review
bills the bare PR number. The view looked `#N` up in a map keyed by bare
numbers, so none matched. Readers now strip the `#`
(`harness_core::domain::task_number`); writers are unchanged, since the
breaker and the budget key on the task as written.

## What shipped

- The commit `feat(ledger): each spend names its milestone at the source`
  (`git log -- crates/adapters/src/store/ledger.rs`).
- `milestone` is the ledger's 20th column; `Tags::milestone` carries it.
  Set by the dev loop (`PickTask`), split (the milestone it cuts),
  refinement (the milestone itself, or the one its task sits under) and the
  arbiter (the milestone its task sits under). Left empty by planner
  (a roadmap), PR review and PR fix (they know the PR's base branch, not
  the milestone's title), test debt, decide, rules, contract manager, data
  diagnosis and the data-model draft.
- `harness backfill-milestones [--ledger PATH]` (TARGET_REPO_URL set):
  reads every milestone, open and closed, and its sub-issues, then fills the
  empty cells; `.bak` copy, atomic rename, refused if the ledger grows
  meanwhile.
- The view prefers the column, falls back on the board, and names what
  neither holds `roadmap`, `PR` or `hors board`.

Tried on a copy of the ForgeDesMondes ledger (2 378 rows, $1 462.56):
508 rows filled ($1 024.84 now under a milestone), 1 870 left empty — PR
review and fix ($395.41), rules write/fidelity ($27.39), test debt ($5.63),
contract evolve ($5.41), planner ($3.27), one decide, one data diagnosis.
A second run filled 0 and left the file byte for byte; every row's first
19 cells were unchanged.

## To verify

- [ ] Stop the watch, then on the real ledger:
      `TARGET_REPO_URL=https://github.com/Laucans/ForgeDesMondes harness
      backfill-milestones --ledger
      ~/.harness/factories/forgedesmondes/.llocal/logs/agent-loop/costs.tsv`
      — expect about 508 filled (more if rows were added since), a
      `costs.tsv.bak` beside it, the same row count before and after.
      Run it again: 0 filled.
- [ ] Restart the watch; the first dev-loop round writes a row whose last
      cell is `#<n> <milestone title>` (`tail -1 costs.tsv | cut -f20`).
      Same for a refinement, a split and an arbitration.
- [ ] The *Spending* screen, *By milestone*: no `unknown` bucket; `PR`
      about $395, `hors board` about $40, `roadmap` about $3 on the current
      ledger; the closed milestones (#3, #4, …) carry their own cost again.
- [ ] `harness costs --by task` still sums each task as before (the budget
      reads `task`, untouched).
- [ ] Follow-up to decide: PR review/fix rows ($395) could be placed under
      the milestone their PR targets (base branch `milestone/<n>-…`), at
      the source and in the backfill.
