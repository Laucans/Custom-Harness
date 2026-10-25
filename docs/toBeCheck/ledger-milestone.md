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
  empty cells; `.bak` copy, atomic rename. It refuses a first line that is
  no header the ledger ever had (renamed column, missing header) and
  touches nothing. The rewrite holds the ledger's lock (`File::lock`),
  which `Ledger::append` takes too and then reopens the file if it was
  renamed over. A writer that does not lock (an older binary) is caught
  only up to the last look before the rename (file, size, mtime changed →
  refused, nothing changed); one appending in the instant after it would be
  lost, so the command refuses while the checkout's watch lock is held and
  holds it itself: **stop the watch first**.
- The view prefers the column, falls back on the board, and names what
  neither holds `roadmap`, `PR`, `rules` (rules write/fidelity),
  `contract` (contract evolve), `hors board` (test debt, a data diagnosis,
  a run on no issue) or `unknown` (any other run on an issue the board
  does not list: its milestone is unknown, not absent — a decide on an
  issue outside every milestone stays there for good).
- A second backfill never overwrites nor removes an earlier copy: it
  writes the first free `costs.tsv.bak.<n>` and the report names it; a
  refused run removes only its own copy.
- Until the backfill has run on a ledger, `unknown` holds every old
  dev-loop row whose milestone closed beyond the board's window and, with no
  board read, every old task row — no longer mixed with roadmap, PR or
  off-board work.

Tried on a copy of the ForgeDesMondes ledger (2 378 rows, $1 462.56):
508 rows filled ($1 024.84 now under a milestone), 1 870 left empty — PR
review and fix ($395.41), rules write/fidelity ($27.39), test debt ($5.63),
contract evolve ($5.41), planner ($3.27), one decide, one data diagnosis.
A second run filled 0 and left the file byte for byte; every row's first
19 cells were unchanged.

## To verify

- [ ] With the watch still running, from the factory checkout
      (`cd ~/.harness/factories/forgedesmondes`), the command below is
      refused (`a watch runs on this checkout`) and the ledger untouched.
- [ ] Stop the watch, then from that checkout, on the real ledger:
      `TARGET_REPO_URL=https://github.com/Laucans/ForgeDesMondes harness
      backfill-milestones --ledger
      ~/.harness/factories/forgedesmondes/.llocal/logs/agent-loop/costs.tsv`
      — expect about 508 filled (more if rows were added since), a
      `costs.tsv.bak` beside it, the same row count before and after.
      Run it again: 0 filled. (partly seen 2026-10-24 17:31 Paris: watch stopped, the backfill filled 511, costs.tsv.bak beside it, 2418 lines before and after (old rows padded to 20 cells); the second run not made)
- [ ] Restart the watch; the first dev-loop round writes a row whose last
      cell is `#<n> <milestone title>` (`tail -1 costs.tsv | cut -f20`).
      Same for a refinement, a split and an arbitration. (partly seen 2026-10-24 18:01 Paris on: 17 code rows and 1 slice row name their milestone; no refinement nor arbitration ran since)
- [ ] Before the backfill, the *Spending* screen, *By milestone*: the old
      dev-loop weight of closed milestones (about $960 minus what the `#N`
      fix placed) sits in `unknown`, not in `hors board`.
- [ ] After it: `unknown` only one decide row (an issue no milestone
      lists); `rules` about $27.39, `contract` about $5.41, `PR` about
      $395, `hors board` about $6 (test debt, the data diagnosis),
      `roadmap` about $3; the closed milestones (#3, #4, …) carry their own
      cost again.
- [ ] A second backfill that fills rows (after the board placed more)
      leaves `costs.tsv.bak` as it was and writes `costs.tsv.bak.1`, named
      in its report.
- [ ] A copy of the ledger with its first header cell renamed, then one
      with its header line removed: the backfill refuses both, names the
      line it found, and leaves the file byte for byte, no `.bak`.
- [ ] `harness costs --by task` still sums each task as before (the budget
      reads `task`, untouched).
- [ ] Follow-up to decide: PR review/fix rows ($395) could be placed under
      the milestone their PR targets (base branch `milestone/<n>-…`), at
      the source and in the backfill.
