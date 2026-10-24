# Playbooks — instruments, procedures, symptoms, what to fix

Every entry was met live on ForgeDesMondes. Shape: **Seen** (evidence) /
**Action** (unblocks the instance) / **Harness fix** (removes the cause —
the fixer's; until it ships the action stays manual). Times Paris (sources
in UTC converted, `places.md`, *Clocks*). Rulings that bind every action:
`rulings.md`.

Who acts: an *Action* is the watcher's unless marked.
**(fixer)**: anything that runs a paid session, pushes, builds, edits code,
changes the product's branches or needs a product checkout — the watcher's
part is to hand it over (`watcher.CLAUDE.md`, step 5). **(owner)**: only
after the owner's go, asked in one question.

## Instruments (`<records>/tools/`, run from `<records>`)

| Tool | Use | Who |
|---|---|---|
| `python3 -u tools/follow_quiet.py tools/follow_quiet.state.json` | the factory follower: reads `harness.db`, `watch.log`, the board (`gh` every 5 min); a line only for what asks for action, a digest every 30 min, the same alert at most once per 2 h. Runs under a Monitor | watcher |
| `NIGHT_START=<UTC iso> python3 tools/night_metrics.py [-v]` | **one** snapshot per call into `night/snapshots.jsonl` (sessions and cost by workflow, events, halts, rounds, red gates, merged PRs, CI, board, disk, Docker, watch alive); a loop calls it every 30 min. `NIGHT_START` defaults to 2026-10-23T20:00Z: set it. `watch_alive` falls into the `pgrep` trap; `harness:parked` / `harness:blocked` counts are always empty | watcher |
| `sh tools/restart_watch.sh [build]` | soft stop, then start with `.env.local`. `build`: after `cargo build -p harness-launcher` in `<harness>`, copies `$SRC/target/debug/harness` into `bin/harness` whatever `CARGO_TARGET_DIR` says — run it as `env -u CARGO_TARGET_DIR sh tools/restart_watch.sh build` | per *Stopping and starting the watch*; `build`: fixer |
| `sh tools/start_view.sh [build]` | the view on :7879 (`build` swaps `bin/harness-view`). **Restarting the view ends every desk**, the attendants' included | owner / fixer, detached (`nohup … &`) |
| `sh tools/harness-cmd.sh <subcommand> …` | one `harness` command with the watch's environment; output `.llocal/logs/agent-loop/cmd-<args>.out` | watcher: read-only commands, `doctor --dry-run`; fixer: the owner-ruled `task <n> --ignore-budget` (*Quota, budget, lanes*) |
| `sh tools/route.sh <kind> <n>` | one router route by hand (`harness route <kind> <n>`), a paid session; the script is meant for a stopped watch; output `route-<kind>-<n>.out` | (fixer), outside a plan only, watch stopped per *Stopping and starting the watch* |
| `db-sweeper.sh` | **daemon, already running** (`pgrep -f '[t]ools/db-sweeper.sh'`): every 10 min drops scratch databases (`*_<pid>`, `rules_runtime_*`) idle > 1 h; log `night/db-sweeper.log`. Stopgap until H6 `ScratchDb` | — |
| `disk-cleaner.sh` | **daemon, already running** (`pgrep -f '[s]h disk-cleaner.sh'`): every 5 min, under 25 GB free, removes `target/` of idle `harness-workspaces/lane-*` and `agentic_workspaces/router-*`, and `$TMPDIR/cargo-mutants-*.tmp` when no cargo-mutants runs; log `disk-cleaner.log`. The view's janitor sweeps only `.llocal/` | — |
| `model_ab.py`; `costs.py`, `journal_costs.sh` | opus vs sonnet on data-layer tasks (code cost/turns, cost after, repair rounds, red gates, blocking reviews); spending by workflow/milestone from `costs.tsv` (`costs.py` assumes UTC−4) | watcher |
| retired | `install-with-backfill.sh` (done 10-24 17:50), `catchup-prs.sh`, `catchup-recipe-*.md`, `unfreeze.py`: never run (they push to or rewrite the product's branches) | nobody |

Disk alert (Monitor; floor 12 GB):
`while true; do f=$(df -g / | tail -1 | awk '{print $4}'); [ "$f" -lt 12 ] && echo "DISK LOW: $f GB"; sleep 120; done`.

## Stopping and starting the watch

Who (the ruling itself: `rulings.md`, the development plan row):

- **During a plan** (`night/dev-plan.md` exists): nobody starts or stops
  the watch but the fixer's one restart. No hot-swap.
- **Opening a plan** (fixer, on the owner's word): soft stop, wait for the
  drained line, then write `night/dev-plan.md` (start, plan, lots).
- **Outside a plan, the watcher**: a soft restart only in the named cases
  (*The watch died*, the GitHub quota); a soft stop when the plant harms
  itself and the board cannot stop it (the disk falling, a paid loop); a
  hard stop only under the disk floor and still growing. After a stop of
  its own, the restart is the owner's: ask.
- **Outside a plan, the fixer** never stops or starts the watch by itself.
  Work that needs it stopped (a leftover push, a residual catch-up, a
  `route.sh` run): score it in `priorities-H.md` and ask the
  owner for a soft stop (owner). The yes covers that one stop and its
  restart: soft stop, the work, `sh tools/restart_watch.sh` (no `build`),
  both logged. A red `main_agent` needs no stop: a PR from your scratch
  clone, watch running (the red-base sentinel already holds tasks and
  merges); if no route merges it, ask the owner — never merge it yourself.

Procedures:

- **Soft stop**: `pgrep -f '[b]in/harness watch --force-reset'`, confirm
  with `ps -o pid,command -p <pid>`, `kill -TERM <pid>`, wait for `watch:
  stopped — soft stop, every lane done` (no cap: a code session runs 30–40
  min; reviews already routed still finish). **Hard**: `kill -KILL <pid>`.
- **Before any start**: the view's prompt drafts (`.llocal/prompts.drafts.json`
  or the Prompts pane) — a start applies them, say so in the report; one
  watch only (10-18 23:25 two ran, #251 merged twice; the instance lock
  refuses a second, check anyway); unpushed work in the lanes' workspaces
  (hand it over first).
- **The plan's one restart**: the fixer's, steps in `fixer.CLAUDE.md`.

Traps:

- A restart forgets what the watch holds in memory: parks, budget halts,
  `catch_up_held` residuals. Restart only when needed.
- A stop with a `soft`/`hard` request in `view.log` is the owner's
  status button, not a crash.
- **`pgrep -f` matches argv**: the view's desks carry their brief in argv,
  and the steward's says "harness watch" (pid 77091 matched
  `harness.*watch` with the watch stopped); your own shell matches too.
  Always the bracket pattern and the full line, then `ps -o pid,command`.
  The same trap blinds `disk-cleaner.sh` and `night_metrics.py`.
- Hot-swap (replacing `bin/harness` under a running watch): new lanes and
  routes run the new binary, the watch keeps the old one.
- zsh does not word-split `$var` in `for`: use `while read` or python.

## Caps and decisions

- A red task PR: 2 `pr_fix` rounds (`MAX_FIXES`, sonnet, sonnet) →
  `decide` → a decided repair resets the count (the next repair on opus) →
  after 2 decisions (`REVIEW_DECISIONS`) the third time is `harness:human`;
  nothing more is paid.
- A milestone's landing PR into `main_agent` has **no repair cap** (#423:
  25 rounds, ~$10). Past 5 rounds on one PR: remove `harness:pr-fix`, log,
  hand over.
- Writing a decision yourself (any rewrite of `## Decision`):
  1. no `decide` running on it: `.llocal/decide-locks/` empty for it, and
     the newest `.llocal/logs/decide/<run-id>/stream.jsonl` has a `result`
     line;
  2. write `decided-by: owner` — `decide` rereads the issue just before
     writing and yields to it;
  3. keep `blocks:`; keep **one** `## Decision` section.
  - Seen: #157 (10-20, two sections); #143 (10-18, answer overwritten).

## Quota, budget, lanes

- **Claude quota** (5 h and 7-day windows, `quota.json`): above 90 % the
  dev loop refuses (`QuotaHasRoom`) and the watch logs `resting: quota
  exhausted`; a refusal spends no retry. `SESSION QUOTA` lines are not
  incidents (570 by 10-24 vs 125 STOP, 10 FAILED): no restart; report only
  if lanes idle > 1 h. `IGNORE_QUOTA` is the owner's (`rulings.md`).
- **GitHub quota** (5 000/h, shared by the watch and the sessions): `API
  rate limit exceeded` in `watch.log` → wait for `gh api rate_limit --jq
  .resources.core.reset`, add no `gh` load.
- **Lanes and interval** (`--parallel`, `--interval` in
  `HARNESS_WATCH_COMMAND`, the owner's numbers): the watcher lowers them
  for **one cause only**, the GitHub quota hit twice in an hour, outside a
  plan — first `--interval 120`, then `--parallel 4`. An edit takes effect
  only at a start, so each cut is a soft restart (a named case), logged
  with the old value and said in the same message. Never for the disk
  (stop instead, *The disk runs out*). Raising them back is the owner's.
  Seen: 10-18 15:17 `--interval 30→60` (4.8 k calls/h); 10-20 06:25 and
  07:32, back to 5/60 at 12:30 once batching (`3a2305b`) held ~900/h.
- **Budget halt** (a task past `BUDGET_FLOOR`, USD):
  - Seen: #214 (10-20, $6.23 in 23 wait rounds of a harness loop); #157
    ($17.29, killed sessions and a repair loop; lifted once 10-23).
  - Action: find who spent it (`costs.tsv`, run logs). The task's own
    spending → leave it parked, report. A harness loop → hand over. A
    budget park lives in the watch's memory: after a restart the task is
    taken and refused again — remove `harness:ready` to hold it.
  - Lifting it once (owner, then fixer): only with the owner's go for that
    task, outside a plan, in a lane workspace no lane uses (the watch fills
    from lane-0: take the highest): `sh tools/harness-cmd.sh task <n>
    --ignore-budget --use-workspace lane-4`. `BUDGET_FLOOR` is the owner's.
  - Harness fix: a budget halt persisted on the board, not in memory.

## Symptoms

### The router idles, or runs nothing useful, while work waits
- Seen: #405 parked 40 min (10-23); the `MAX_CURRENT` deadlock (#219
  waiting on #24, which the cap kept unsplit, 10-24 11:05).
- Action, by the `idle:` / `watch:` line:
  - `parked until its issue changes`: expires in 1 h (`089b6bf`); if the
    cause is gone and the task is critical, edit its body (unparks it).
  - `N task PRs open for L lane(s)`: WIP limit — why don't they merge?
    (next entry). Rules task open: *A rules PR never counts green*. The
    database: *Docker / Postgres is down*. Quota: *Quota, budget, lanes*.
  - an open `harness:human` "Integration branch main_agent is red"
    (red-base sentinel; no task or merge until green, it closes on green):
    find the commit that turned it red, hand over (fixer, a PR on
    `main_agent`, no stop: *Stopping and starting the watch*).
  - a building milestone's task waits on a milestone the cap keeps
    unsplit: hand over `sh tools/route.sh split <n>` (fixer, outside a plan
    only, watch stopped per *Stopping and starting the watch*).
  - `test_debt` or `rules` running alone = nothing else was routable.
- Harness fix: P (a blocked building milestone admits its blocker past the
  cap).

### A clean, green PR does not merge, or a task stays open after its PR merged
- Seen: #77, #498 (`review-pending` lost); #482 (`harness:human` left on
  a clean review); #146/#170/#198/#212 (`Closes #N` stripped by the
  unfreeze); #165/#207/#362/#463/#464 (task open after its PR merged).
  Rules PRs stalled 13:38–15:20 on 10-24: `28a9439` (committed 12:52,
  installed 13:10) denied `gh pr merge` to sessions while the fidelity
  prompt still said "merge" — fixed by `d0a2a93` (installed 15:55).
- Action — the merge train needs, in this order:
  1. the task carries `harness:review-pending`;
  2. the task does not carry `harness:human` (remove it once the latest
     review is clean and checks are green);
  3. the PR body's `Closes #N` line (`places.md`, *The product*);
  4. the latest review carries `<!-- agent-review-verdict: clean -->` (a
     rules PR: `<!-- rules-fidelity: faithful -->`).
  - A PR merged, task still open: close the task naming the PR. A flow
    still ending on a "merge" instruction: hand over.
- Harness fix: H2 (a merged PR closes its task whatever merged it).

### A session stops on a contract and nothing opens
- Seen: #156 `media.listMedia` (10-21); PR #423 `session.listScenes`
  (10-24 05:15); PR #500 `campaign.saveEntity` (13:47) — no
  `harness:contract-change`, `pr_fix` kept spending.
- Action: open the issue by hand (`places.md`, *The product*, **Contract
  change issue**), set the task `blocked_by` it, remove `harness:pr-fix`
  from the PR until the producer lands. A request on an aggregate or a
  landed contract is answered to the task (`78c43f2`). A manifest or
  fixture to write: hand over (fixer).
- Harness fix: H2 (a contract stop in `pr_fix` raises the change).

### A task halts on its own spec (`TaskHasASpec`, `ArchitectureHolds`)
- Seen: contract-manager tasks #362 (no `produces:`), #434, #503, #482
  (told the consumer to change the contract); hand-written #507 (no
  `produces:`, the narrowed checkout lacked its own test).
- Action: write `## Business Goal`, `## Acceptance Criteria` and an
  `## Architecture` with `unit:`, `system:`, `produces: <ids>`; re-add
  `harness:ready` (it also retries within the hour).
- Harness fix: H2 (the contract manager writes split's full shape); the
  gate names the missing line (open).

### A missing task lands on the wrong issue
- Seen: #448 parked on #473 (both produce `Entity`, 10-24 09:20); #486
  found 4 gaps over 4 stops ($4.88).
- Action: append the stopped task's scope and criteria to the matched task,
  or open a separate agent task under the milestone; check the stopped
  task is `blocked_by` it.
- Harness fix: MT (dedup by title, not `produces` id; one stop declares
  every missing task).

### A contest blocks a PR, then decide says "wait"
- Seen: #516 (task #486), 10-24 21:05 — `node-gates` and `rules-hold` red
  while contest #517 was open; 2 `pr_fix`, then decide; each "wait"
  counted as a round and sent #486 to `harness:human` (21:12).
- Action: nothing to repair. Let the rules PR land (judged `faithful`,
  merged by the harness), then the milestone's catch-up. Then resume: a PR
  comment starting with `<!-- harness-decided: repair -->` and a narrow
  brief (what to do; what NOT: never merge `main_agent` or a milestone
  into the branch), add `harness:pr-fix` to the PR, remove
  `harness:human` from the task, comment why on the task, log it.
- Harness fix: contest-flow lot (`night/contest-flow-design.md`): a PR
  waits on its open contest, a "wait" is never a round.

### decide tells a session to merge a branch itself
- Seen: #486 (10-24 18:04), #219 (22:12) — "the next session merges
  origin/main_agent into its branch".
- Action: remove `harness:ready`; rewrite the `## Decision` (*Caps and
  decisions*): `verdict:` "wait for the harness's catch-up; stop `missing
  <path>` if a unit is still absent"; no `harness:needs-decision`; comment
  why; re-add `harness:ready` when `gh api
  repos/<product>/compare/<base>...<sha> --jq .ahead_by` is 0 (poll ≥ 30 s).
- Harness fix: decide never prescribes a branch merge (H2).

### A catch-up cannot take the base (residual conflict)
- Seen: milestone/24 (10-24 18:09) and /28 (22:13), `mock/src/commands.js`,
  `reads.js` — both sides appended to the legacy shared handler file. A task
  started before the catch-up stops "missing …" and reruns after it.
- Action: note the residual line and files, remove `harness:ready` from
  that milestone's tasks that would start on a stale base, hand over.
- Action (fixer, watch stopped: inside a plan, or outside one per
  *Stopping and starting the watch*): the product's `node
  scripts/split-handlers.mjs` on `main_agent` and on the milestone branch,
  each through a PR, bundles rebuilt (`sh mock/build.sh`), gates green,
  merged by the harness; generated-file conflicts left to the catch-up.
  Never push to a milestone branch directly.
- Harness fix: H1-C, new handlers one file per unit (`a45a86b`); the
  legacy split is per product (due since 10-24 23:00).

### A rule turns red on its first run (after a catch-up or a landing)
- Seen: 10-24 05:00–07:55 — 8 contests (R101, R112, R16/R111, R10, R97),
  all at a rule's first run, 3 real product defects.
- Action: diagnose from the failing test, not the PR.
  - Test mounted wrong (slugs for ids, a wrong need, a reply format, a
    strict-mode selector like R103's `getByText('CA')`) → a
    `harness:rules` contest issue naming the rule and the cause.
  - Product defect → an agent task under the milestone (and a contract
    change if a manifest is involved).
  - Never weaken a rule test in a product PR.
  - A settled contest missing from the contested PR's milestone branch
    keeps it red: check the catch-up ran; a hand merge of `main_agent` into
    the milestone is (fixer), watch stopped as in the previous entry.
- Harness fix: H1-B/H4 (settled contests reach the milestone branches).

### A rules PR never counts green, or the plant idles behind it
- Seen: #501 (10-24 15:08) — a superseded FAILURE beside the newer SUCCESS
  in `statusCheckRollup` (close/reopen did not help); 10-24 21:00–21:25,
  the rules task re-attempted 3× while its PR only waited on CI.
- Action: while a rules task is open nothing else runs: watch its PR.
  A superseded red (owner, a deletion): `gh run delete <id>` of the failed
  run older than the label. No fidelity verdict and no review → add
  `harness:to-review`. The harness merges on `faithful` with every check
  green.
- Harness fix: latest row per (run, check), rules PRs by `head_ref`
  `rules/`, the rules route waits on its PR's checks (R, contest-flow lot).

### A route halts on what an earlier session left in its checkout
- Seen: router-lane-0 after a killed `pr_fix` (10-19 23:49, 10-24 18:05:
  "carries work on 1 branch origin does not have"); a stash from #264
  (10-20); `lane-0-save-entity` holding milestone/20 (10-24 01:25). A
  session waiting on a long job (`cargo mutants`, `gh pr checks --watch`)
  gets killed as hung.
- Action: note it, hand over.
- Action (fixer, watch stopped: inside a plan, or outside one per
  *Stopping and starting the watch*): an unpushed commit that answers the PR's
  review and fast-forwards its branch → `git fetch <workspace> <sha>` into
  your clone, push it to the PR branch, comment; then reset the workspace
  to origin. Drop stashes; `git worktree remove` stale worktrees you made
  (never one a lane runs in).
- Harness fix: clean route lanes (`5c68274`, `e30cde8`); a session ends at
  its pushed PR (`95dd36c`). A recurrence is new evidence.

### Docker / Postgres is down
- Seen: 10-20 (OrbStack stopped overnight: 28 sessions without a DB, 7/20
  `rust` checks red); 10-22 07:48 (a full disk stopped OrbStack, idle 3 h 20).
- Signs: `warning: the database at … does not answer`; no
  `harness-postgres` in `docker ps`; `rust` reds on DB tests across PRs.
- Action: free the disk first if it is the cause, then `orbctl start` and
  `scripts/postgres.sh status` (from `<harness>`). Never a Homebrew
  Postgres (same port). The watch resumes by itself. PRs whose CI ran
  during the outage: `gh run rerun <id> --failed`, not a repair. Teardown
  reds (`DROP DATABASE … WITH (FORCE)` refused) = the role lacks
  `pg_signal_backend` (granted 10-24 10:20; `docker/postgres/provision.sh`
  grants it since H6).
- Harness fix: the database hold (shipped); H6 provisioning.

### The disk runs out
- Seen: 10-24 22:28, 1 GB free — a split started 4 cold product builds
  (~8 GB per lane target), a 41 GB scratch target, a cargo-mutants copy.
  Earlier: 7 939 scratch databases (68 GB), `target/debug` at 105 GB. A
  full disk also **stops OrbStack**.
- Action: find the grower (`du -sh` on lane targets,
  `$TMPDIR/cargo-mutants-*`, scratch targets, `docker system df`); stop
  only your own jobs, by pid; delete stale incremental dirs, dead mutants
  copies, scratch targets. The plant is the grower → soft stop; under 12 GB
  and still growing → hard stop. The restart is then the owner's (ask).
  Check Docker after every sweep.
- Harness fix: lot D — sccache + `CARGO_INCREMENTAL=0` for lanes (3.4 GB a
  target instead of 8.2), a disk floor before a lane starts, targets pruned
  between tasks; H6 `ScratchDb`.

### The watch died
- Seen: 10-19 02:05 (log cut, no `stopped` line); the follower says
  `WATCH GONE`.
- Action: check it was not the owner (`view.log`) and that no plan is open
  (`night/dev-plan.md`). Then the checks *Before any start*, then `sh
  tools/restart_watch.sh` (no `build`), logged.
- Harness fix: none known; new evidence goes to the fixer.

## Reference numbers

Night 23→24 Oct, 5 lanes (`night/report-2026-10-24.md`): ~$10/h ($109 over
10 h 45, 99 sessions); per merged task PR $3.9 code, $6.8 all-in; code
session opus $4.4 / ~60 turns, sonnet $1.1; opus data-layer task $14.66
all-in, 110 turns, 2.3 fixes; `gates` red 32 % (41/127; on 10-23, 40/43
red gates were rules-hold).

Alert when: a task passes 2× its kind's mean; a PR passes 5 repair
rounds; an hour passes $20; the same `decide` answer repeats on one issue.
