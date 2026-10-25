# H1-B+: lag hold, 5-minute retries, the catch-up across the handler split

**Status:** on `main`, after `d0ca9ce`. Not committed yet. Not seen on a live
`watch`. Design, critique and decision:
`harness-dnd2-run/night/h1b-plus-design.md` (§ Decision, 2026-10-25 03:20).

## Context

ForgeDesMondes, 2026-10-24:

- **#219, 22:07 Paris.** #24 landed on `main_agent` at 22:06. Its checks
  were still running, so milestone/28 did not hold OralBrief /
  ToCreateList yet. A branch behind a pending head was neither due nor
  held, so the task started on the old base and stopped "missing"
  ($0.16).
- **milestone/24.** An agent caught it up outside the watch. Its tasks
  still waited the hour of the residual mark (`PARK_SECONDS`).
- **#530 (`a55bd4d`).** `main_agent` split `mock/src/{commands,reads}.js`
  into `mock/handlers/`. Every building milestone branch still edits the
  legacy files. Its catch-up meets `UD` on both files, so it is residual,
  then a pr-fix session. That session's parent-first `-s ours` procedure
  itself conflicts when both sides appended to the legacy literals.

## What shipped (uncommitted on `main`)

- **Lag hold** (`workflows::common::catch_up`):
  - `CatchUps::holding` now also holds a branch that lags the head:
    - the head is green and the catch-up is not made yet;
    - or the head's checks have not concluded within `LAG_HOLD_SECONDS`
      (1200 s), counted from the head's commit date (`GitHub::commit_time`,
      new port method; the reading's second if the date is unreadable or
      in the future).
  - When the hold expires, the board is read once in full. The expiry is
    warned once per head (`routing::read_catch_ups`).
  - `CatchUps::reread`: when a catch-up settles a branch, the next tick
    reads the board in full.
- **Retries:**
  - A residual is tried again through `merge_branch` every `RETRY_SECONDS`
    (300 s), no longer every hour.
  - A parked failure is rechecked the same way. It is not merged on GitHub
    when the merge stales the bundles. Its full local retry stays at 1 h.
  - `read_head` asks for a full read when a retry is due (head green).
  - The "no landing PR" warning is said once per head.
  - A lane working on the landing PR pushes the next retry back 5 min.
  - A full read forgets the marks of branches that are no longer targets,
    or that already hold the head.
- **Handler split** (`workflows::common::handler_split`, called from
  `branching::regenerate`). Done in the one checkout, in sequence, with no
  worktree:
  1. Abort the merge.
  2. Split the merge base on a detached checkout and commit it there. The
     result is on no branch.
  3. Split the branch. Commit only `mock_handlers::SPLIT_STAGED`. The head's
     `scripts/split-handlers.mjs` and `mock/src/server.js` are written for
     the run, then put back.
  4. Merge again.
  5. Merge each `AA` under `mock/handlers/` three-way with `git merge-file`,
     using the split merge base as the base.
  6. Generated files go through the existing loop. `mock/build.sh` always
     runs.
  7. If the splitter refuses the branch, the result is Residual on the
     legacy files. If it cannot start, is killed, or the node is too old,
     the result is Failed (parked).
- **Detached catch-up:** the whole local merge runs on a detached `HEAD`
  (`checkout --force origin/<branch>`, the split re-checked-out at that
  tip), pushed with `push origin HEAD:refs/heads/<branch>`
  (`Repo::push_head`). No local branch ever holds an unpushed commit, so a
  SIGKILL mid-catch-up leaves no branch for `branches_at_risk` to find.
- **Cleanup:** any result other than `Locally` puts the checkout back to
  `origin/<branch>` (checkout --force, reset --hard, clean). This removes
  the false `WorkKept` alert after a residual or a raced push.
- **Unsplit base said:** when the merge base cannot be split (splitter
  refused, no merge base) and add/adds remain, the journal says why:
  `<branch> cannot merge the handler files both sides added three ways
  (<why>): they stay a hand's`.
- **Lag expiry on red:** a red head (`verdict = Some(false)`) never
  "expires": no "has no verdict" warning, no extra full read; `base_red`
  holds the tasks.
- **Ports:** `Repo::{merge_base, show, stage_paths, merge_file, push_head}`,
  with `GitCli` implementations and fakes.
- **pr_fix:** `mock_handlers::on_conflict` now uses the same method (one
  strategy, no `-s ours`). The three sides go under `S=$(mktemp -d)`, never
  a shared `/tmp/ours`; the session commits or stashes its own changes
  before the `checkout --force` / `reset --hard` steps.
- **Replay** in a scratch clone (milestone/28 `7e2a839` vs main_agent
  `a55bd4d`, node 26), running the same git sequence:
  - merge base `19262b9`: 59 handler files;
  - branch split: 63 handler files;
  - split commit: 65 files, +1349/−1165, nothing outside
    `mock/handlers/` and the two legacy files;
  - second merge: `UU` on Cargo.lock, registry `Cargo.toml`,
    `queries/registry.json`, `mock/pages.json` (14 hunks, all `builtFrom`),
    plus `AA` on `_lib/commands.js`, `_lib/reads.js` and
    `campaign.saveEntity.js`;
  - all three merge-file runs exit 0, no markers left.

## To check (live `watch`)

- [ ] **Lag hold.** After a `main_agent` move, no task or dev-loop session
      starts on a building milestone between the move and its
      `watch: catch_up -> milestone #N holds the base`. Expected: 0 task
      sessions started on a lagging branch, per `watch.out` timestamps.
- [ ] **Reread.** The tick after a catch-up settles is a full read (no
      "quiet: nothing moved" line). The held tasks dispatch within 1 tick.
- [ ] **No CI run.** For a head with no CI run, tasks are released 20 min
      after its commit, with exactly one line:
      `warning: watch: the integration branch's head <sha7> has no verdict
      20 min after its commit — …`.
- [ ] **Restart.** After a `restart_watch.sh`, the first green tick makes
      every building milestone due (GitHub no-op merges, `holds the base`).
      Their tasks dispatch the tick after. Expect about 2 ticks when
      `main_agent`'s head is green and older than 20 min.
- [ ] **Hand-fixed residual.** Within 5 min:
      `<branch> holds main_agent now (merged outside the watch) — its
      residual mark goes`, then `watch: catch_up -> milestone #N holds the
      base`. No 1 h wait.
- [ ] **Parked failure.** A failure GitHub can merge ends with
      `holds main_agent on GitHub's merge — its park ends` within 5 min.
      One whose merge stales bundles waits for the 1 h local retry.
- [ ] **No-landing warning.** At most one warning per head
      ("no landing PR to repair"). Retries every 5 min stay debug.
- [ ] **milestone/28.** The harness pushes a split commit and a merge
      commit. Expected:
      - the journal says `its legacy handler files split first,
        mock/handlers/_lib/commands.js and mock/handlers/_lib/reads.js and
        mock/handlers/campaign.saveEntity.js merged three ways on the split
        merge base, Cargo.lock and … regenerated`;
      - the split commit has 65 files and +1349/−1165 if the branch has not
        moved since `7e2a839` (the outcome should not change if it has);
      - `git grep '<<<<<<<' mock/handlers` is empty;
      - `mock-handlers` and `pages-mocked` are green on its next CI.
- [ ] **Other legacy branches.** Every other building legacy branch (all
      `origin/milestone/*` still had both legacy files on 2026-10-25) goes
      the same way, one per tick (`LOCAL_PER_TICK`).
- [ ] **Merge train.** A legacy task PR into a split milestone is refreshed
      split-first by the merge train (`Attempt::Stale`) and lands green. A
      conflicting one, which is never green, goes to pr_fix, whose
      `on_conflict` now follows the same method: check the session runs
      `git merge-file` and no `-s ours`.
- [ ] **No false alert.** No `Work kept on catch-up` notification after a
      residual or a raced catch-up. `git -C <catch-up workspace> status`
      is clean, on the branch, after each catch-up.
- [ ] **No leftovers.** `git log --branches --not --remotes` in the
      `catch-up` workspace is empty. The dangling split-base commit is on
      no branch.
- [ ] **Kill mid-catch-up.** Hard-stop the watch (the view's SIGKILL) while
      a split catch-up runs (between the split commit and the push, e.g.
      during `mock/build.sh`), restart, and let it catch up **another**
      branch. Expected: no `Work kept on catch-up` event on that release;
      `git log --branches --not --remotes` in the catch-up workspace is
      empty; the killed branch is caught up again on a later tick.
- [ ] **pr-fix in parallel.** Two pr-fix sessions resolving handler
      add/adds at the same time each write their sides under their own
      `mktemp -d` folder; no file of one appears in the other's merge.
- [ ] **Node version.** The catch-up process runs node ≥ 22 (`node
      --version` in the toolchain line). On older nodes the result is
      `Failed`, parked, never a pr-fix session.
- [ ] **Base-split self-check.** If the base split refuses (its self-check
      runs the merge base's tree with the head's `server.js`), the journal
      says `… cannot merge the handler files both sides added three ways
      (the merge base cannot be split: …)` and the add/adds stay residual.
      Note how often this happens.
- [ ] **Red head.** A head whose checks conclude red shows no "has no
      verdict 20 min after its commit" warning; its tasks stay held by the
      red base.
- [ ] **#219.** It reruns once milestone/28 holds #519, with no "missing"
      stop. Stop the 22:12 hand loop that re-adds `harness:ready` after
      deploy.

## Not covered

- A `DU` on a legacy file stays residual: the receiving side split, the
  incoming side is legacy.
- A PR whose only conflicts are legacy `UD` is not sent to the local
  refresh unless its checks are green (critique point 4). It goes to
  pr_fix, which now uses the harness method.
