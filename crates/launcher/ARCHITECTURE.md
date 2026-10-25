# `harness-launcher` — architecture

The entry points: the `harness` binary, the CLI, and **the only place in the
workspace that builds a concrete adapter** from `harness-adapters`. That is the outer ring of the
hexagon: every `Rc<dyn Trait>` a workflow receives is chosen here, and nowhere
else. The crate graph is in
[ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md); the rules the
split is checked against are in [../../CLAUDE.md](../../CLAUDE.md).

Errors here are `anyhow` at the boundary (`.with_context(…)`), unlike the two
library crates, which carry typed `thiserror` enums — see `../../CLAUDE.md`.

## The modules

`tests/skeleton_rehearsal.rs` (ignored, on demand) wires the dev loop's
`Scaffold` with `RealDisk`, `Processes` and `GitRepos` on a scratch clone a
human names (`SKELETON_PRODUCT`, `SKELETON_BODY`): the rehearsal of a
skeleton on a real product, before a night relies on it.

Three roots and two folders, and the split between them is **who decides to
run something** versus **what running it takes**:

```
src/main.rs       entry point: parse, find the repo, dispatch, map the exit code
src/cli.rs        what a human types: the arguments and their environment variables
src/router/       what decides on its own: `harness watch`
  mod.rs            the loop: a tick, its reads of the board, the rest after a `QUOTA` stop — and, starting, the prompt drafts applied (`shared::apply_drafts`)
  routes.rs         what a tick decided, run in this process: the route's workflow, the merges, a refinement round, the record a failed dispatch leaves for the doctor
  lanes.rs          the lanes of a parallel watch filled: a `harness` child per runnable task, split or refinement, in a checkout of its own — the slot's two build folders pruned first (`prune_slot`)
  floor.rs          the disk floor: under `DISK_FLOOR_GB` free on the lanes' volume (core's `Volume` port, `Statvfs`), no lane starts; each crossing said once
src/dispatch/lanes.rs  the lanes of a parallel watch: one `harness --task <n>` child per runnable task, up to `--parallel`; a lane that stops (exit 1) parks its task until the issue's body or labels change, or a blocker holding it lands; `prune_target`, through the `Disk` port, keeps a lane's `target` under `LANE_TARGET_CAP_GB`

src/dispatch/     one module per thing that can be run, plus what they share
  dev_loop.rs       the first workflow: an exclusive workspace, several sessions deep (`--no-scaffold`: no unit skeleton before `code`)
  planner.rs        a roadmap item into milestones
  split.rs          a milestone into tasks
  refinement.rs     an issue body into its five canonical sections
  arbiter.rs        a task's graph question settled, or handed to `decide` — the shared
                     read-only checkout, for the manifests
  decide.rs         a decision taken where a human was asked — the shared read-only
                     checkout, the session reading the design dossier
  contract_manager.rs  a contract a consumer needs changed, evolved for every consumer —
                     the shared read-only checkout, the writes on the integration branch
  pr_review.rs      an advisory review on a PR
  pr_fix.rs         one repair attempt on a red PR — a writable checkout of its own
  catch_up.rs       `Route::CatchUp`, in the tick: `common::catch_up::catch_up` handed a
                     local merge — a writable `catch-up` checkout (`shared::mount_bare`:
                     the watch's toolchain untouched) with `GitCli`, the merged tree's
                     own pin (`toolchain::by_the_tree`, rustup's proxies ahead) and
                     `RealDisk`, where the generated files are regenerated, and the
                     bundles rebuilt when the head staled them
                     (`branching::catch_up_locally`); told as `watch: catch_up -> …`
  test_debt.rs      the tests one file owes, from its test-debt issue — writable too,
                     on the integration branch
  rules.rs          the rules and their tests, or a contest settled, from a rules'
                     issue — writable, on the integration branch
  data_diagnosis.rs  a version of main or main_agent whose two sides disagree, right after
                     its snapshot: one Opus session in the snapshot clone says why; its
                     answer is the `DataModelDiverged` event, an error in the view
  data_snapshot.rs  every tick that read the board: the data model of each branch whose head
                    moved (main, main_agent, open milestones), dumped and archived on the
                    target's `harness/data-model` branch; a failure is an event, never a failed tick
  scratch_sweep.rs  from the watch's loop, after every tick whatever it did (a quiet board,
                    a quota rest): the stale scratch databases a product's tests left on
                    the server `DATABASE_URL` names, dropped through `PgDatabases`, at most
                    every ten minutes; never under `--dry-run`, never a failed tick
  main_agent_merge.rs  a finished milestone's PR into main_agent, opened then merged
                       (a task's PR into its milestone — `milestone_merge` —
                       is called by the router with its `gh`; a PR on a base that
                       stales its bundles is refreshed through `catch_up::locally`)
  init_repo.rs      `harness init-repo`: a `GhCli::for_slug` (a URL, no clone), a
                    `GitRepos` and `Processes` for the install clone
  backfill.rs       `harness backfill-milestones`: the ledger's `milestone` column filled
                    once from every milestone, open and closed
                    (`hierarchy::milestone_index` → `Ledger::backfill_milestones`), its
                    `gh` a `GhCli::for_slug` of `TARGET_REPO_URL`; refused while the
                    checkout's watch lock (`router::watch_lock_dir`) is held, which it
                    holds itself until done — run with the watch stopped
  shared.rs         the checkouts and adapters the dispatch modules share
  tooling.rs        the gates that belong to no workflow
  trial.rs          `harness trial`: a prompt's dry-run — a past run's prompt replayed with
                    another text (stdin), asked once in plan mode in the `prompt-trial` clone

src/toolchain.rs  the one toolchain resolver: the `PATH` and `SHELL` a product's processes
                  and every session get (shims for GNU sed/grep, bash ≥ 5, node ≥ 22; the
                  product's pinned cargo, else rustup's proxies; then the inherited `PATH`),
                  handed to the adapters the launcher builds (`processes()`,
                  `environment()`: `with_env`), with the shared compilation cache's variables
                  when sccache is found, and said as `Event::ToolchainResolved` when
                  the watch starts and when a run mounts its checkout; `harness toolchain`
                  prints the same line

src/adapters/     the ports only the launcher can fill — written here, not wired
  sink.rs           where log lines go: console + file
  spending.rs       the `Spending` port, implemented: clock, run id, machine
```

**Why `dispatch/` is one folder and not two.** Splitting it by trigger — what
the CLI runs versus what the router runs — would put `dev_loop` in both: it is
the `harness` binary's default *and* a route. The trigger is a property of the
decision, so it lives with the deciders (`cli.rs`, `router.rs`) and each
dispatch module's own doc names the one that reaches it.

**There is no `hooks/`.** The original design triggered `pr_review` from a hook
on `gh pr create`; that needs a permanently reachable public URL, so the
harness polls instead (`router.rs`) and every trigger is a label. The repo's
git hooks are shell, in `.githooks/` at the workspace root — nothing in this
crate.

## `main.rs` — the exit code is a contract

An external scheduler reads it, and the values are frozen: `0` fine, `1` a
voluntary halt or an unreadable store, `2` a failure, `3` a quota exhausted. They
come straight from `Halt::exit_code()`; moving one would be a hidden contract
change dressed up as a refactor.

`#[tokio::main(flavor = "current_thread")]`: decision #3 chooses `?Send`
everywhere, so a multi-thread scheduler would have nothing to schedule.

The repository root is found by **walking up** from the current directory looking
for `.git` — no subprocess.

## `cli.rs` — declared together

Flags and their environment variables live in the same `clap` struct, so "every
variable the code reads appears in `--help`" is true by construction rather than
asserted by a test. `RunArgs` is the dev loop's flags and the only type that
reads the environment; `Cli` flattens it (`#[command(flatten)]`) alongside an
optional `#[command(subcommand)] command: Option<Command>` — no subcommand
still parses exactly as the flat `Cli` once did (`harness --rounds 1 --stages
code` keeps meaning the dev loop). Thirteen subcommands today: `init-repo`,
`watch`, `doctor`, `refine`, `split`, `arbitrate`, `trial`, `data-snapshot`,
`data-model-draft`, `costs`, `route`, `plan-graph` and `toolchain` — plus one
hidden, `architecture-check` (`dispatch/architecture_check.rs`), which no
human types: the product's `scripts/gates-local.sh` runs it through
`HARNESS_BIN` on a session's work before its pull request — the loop's own
`judge` (`workflows::dev_loop::checks::architecture::closure_check`) on a
full copy of the work, the changes from the merge base to the snapshot, the
declaration from the session facts (`--facts`) or, in a repair session, the
task's body off GitHub (`--task`); exit 1 on a finding of the round's own,
the older ones printed `(before this round, not yours)`. `refine` and `split` exist for the lanes of a parallel
watch — one child process per issue — and for a human who wants one issue
done now; `planner`, `pr_review`, `pr_fix`, `arbiter`, `test_debt`, `rules`, the merges and `data_diagnosis`
have **no** subcommand on purpose: they are reached by the router, whose own
arguments (`--interval`, `--once`, `--dry-run`, `--parallel`) are what a
human tunes instead.

## `router/` — the trigger nobody types

`harness watch`: read a snapshot, schedule it, dispatch, sleep `--interval`
seconds, repeat. `--once` does a single pass.

**One scheduler for every route** (`routing::schedule`, then
`routing::plan`, both pure and tested without a fake adapter). Every piece of
work the snapshot shows — every PR to repair or review, every decision,
split, task, refinement, merge — is a weighted job: its route's base
(`routing::base_weight`: the catch-ups, then the two merges, then plan, rules, decisions,
contracts, repairs, reviews, splits, arbitration, tasks, business then
technical refinement, test debt), a bonus for a milestone on the critical
path, a bonus for a task by the pending rules it unblocks (4 a rule touched,
8 more a rule completed, capped at 100, +5 for a `DataCapability` —
`common::rule_waits::bonus`; a task never weighs past
`routing::DEV_TASK_CEILING`, 779, just under a review), a point per tick it
waited, capped. Under `--parallel` the tick
(`router/lanes.rs::run_schedule`) runs every free job in the tick — the
catch-ups and the merges, a few GitHub calls — and starts the paid ones heaviest first, one
per free lane: a task as the dev loop's `--task` child, anything else as
`harness route <kind> <n>` in its own `router-lane-<k>` checkout. **Nothing
paid runs in the watch's process**, so a long session never keeps the
lanes from being refilled. One job per issue or PR at a time — a job's
key and its holds (`Job::holds`): a decision holds the open PRs of the
issue it decides on, a repair the task or milestone its PR delivers, so a
decision and a repair of the same task never run side by side, nor does
either start beside the other's lane (W, `ForgeDesMondes`' #542). A hold
waits only on a session (a lane's key, a paid job picked this tick), never
on a free job: a red landing PR's milestone is ready to merge every tick,
and its free merge must not starve the repair; the
technical refinement, the test debt and the planner one at a time each (an
exclusive group a lane holds). With one lane, `routing::decide` — the schedule's
first job — runs in the tick, as before.

**The rules' waits are read once per landing** (`router/mod.rs::rule_waits`):
the integration branch's `rules-status.json`, the `rules-status` artifact
`gates.yml` keeps (`GitHub::artifact_file`), once the head the catch-ups
read has a verdict and was not read yet (`routing::rules_status_due`), kept
in the pulse in between (`routing::took_rules_status`). A failed read is a
debug line and is tried again at the next full read. The `saw:` line names
the top unblockers (`rules: #157 unblocks 17, …`); a task a human holds (no
`harness:ready`) or a parked one that holds `routing::RULES_SAID` (5) rules
or more is said each tick (`#157 has no harness:ready, holds 17 rules`,
`… parked until its issue changes …, holds 17 rules`).

**A starved building milestone pulls its producer's split past the cap**
(`routing::pulls`, `routing::pulled`): no task of it may start and none is
on a lane, or every task is delivered and its landing waits. Starved, it
pulls only what holds a task nothing else holds (no sibling, person or own
PR cycle: `tasks::held_only_by_milestones`) — the producer of a need no
mock-up stands in for (`tasks::held_needs`) or a milestone blocker; held at
its landing, the producer of any need or blocker of its tasks. The
producer, unsplit, earlier in plan order and a `split_candidate`, is split
whatever `MAX_CURRENT`, one a tick, only when the cap holds it back
(`splits(.., None)` would not start it). One that may not split yet is
pulled through: the first unsplit, splittable milestone it waits on, walked
through the graph's edges. A pull neither can serve is said once per change
(`routing::stuck_pulls`, `routing::unpulled_news`: `… would be split past
the cap …, but its own producers are not split and none can be yet`).
`Event::SplitPulled` (`… needs X[ through #P] (starved | held at landing)`)
is told when that split starts on a lane (`router/lanes.rs::pulled_event`),
not each tick it waits. A pulled split is a lane's job like any: the disk
floor holds it while the disk is short.
**Ahead work is judged per task, against every milestone before it**
(`routing::startable`, `board::building`, `router/lanes.rs::startable_now`:
`tasks::runs_ahead` over `merge_train::unlanded_before`, the milestones
earlier in plan order not landed — not only the graph's edges): a task
waits on the producer of a table, a Concept, or a Micro-UI it builds on;
a Capability or a `DataCapability` stands on its fixture, and a screen
(a composition) on the mock-up's fragment of every Micro-UI it lays out —
named rules no longer hold a screen (`proves_rules` is gone: its rules stay
pending until their units are built, and `rules-hold` proves them). The
lanes say which producer holds which task (`#28's tasks … wait for them to
land (#216 on #20)`). A screen parked `blocked_by` a milestone waives it
when every id it needs of that milestone is one the mock-up stands in for
(`tasks::screen_stands_on`, judged on the graph by `tasks::runnable_in`;
the lane's `--task` run, which reads no graph, trusts the router for a
screen's milestone blockers: `tasks::runnable_for_lane`). A Micro-UI's
agent blockers are waived as before.
**The landing waits on the producers of its tasks' needs**
(`milestones_ready_to_merge` → `merge_train::landing_waits`, closed tasks
included), beside the graph's edges — an edge the plan forgot holds the
landing, not the task.

**The disk budget.** Five lanes each build the product in their own
checkout; a cold build is 3.4 GB of `target`, one with incremental on grew to
8.2 GB, and four cold builds at once took the disk to 1 GB (2026-10-24
22:28). Three answers, each its own:

- *a shared compilation cache* (`toolchain.rs`, below): sccache, incremental
  off — a cold product build 54 s, warm 44 s;
- *a floor* (`router/floor.rs`): every tick measures the free space of the
  lanes' workspaces folder (else the factory), one `statvfs` through core's
  `Volume` port. Under `DISK_FLOOR_GB` (default 12, `0` none) no lane starts
  — the free work, the merges, goes on; with one lane nothing is dispatched.
  `Event::DiskUnderFloor` is said when it falls and notify raises it (an
  error, `disk-floor`); `DiskAboveFloor` when it is back, which clears it,
  and that tick reads the board like a freed lane. Under the floor the free
  slots' targets are pruned first (`prune_slot` on every idle slot) and
  the volume measured again, so space the watch owns never holds it. A
  volume that cannot be asked changes nothing: unheld it holds nothing
  back, held it stays held. A new watch starts unheld, and notify clears
  `disk-floor` on `WatchStarted`;
- *a cap* (`dispatch/lanes.rs::prune_target`, called by
  `router/lanes.rs::prune_slot` on the slot about to be taken, so nothing
  builds there): over `LANE_TARGET_CAP_GB` (default 6, `0` never) a lane's
  `target` — its task checkout's `lane-<k>` and its route checkout's
  `router-lane-<k>` — loses `debug/incremental` and every executable a
  later build session superseded (`<name>-<16 hex>` in `debug/deps` more
  than six hours older than the newest of its name, with the files cargo
  wrote beside it; a name cargo uplifted into `debug/` — a bin — is never
  touched); over twice the cap the build profiles go (`debug`, `release`),
  what is not cargo's staying (`target/rules-runtime`, a server's pids).
  Said as `Event::LaneTargetPruned` when something went or was refused. A
  dry run prunes nothing.

**Every line of the watch and its lanes is an event first**
(`Logbook::event`): the sink of a `run.log` or of `watch.log`
(`adapters::sink::Both`) also keeps it in the checkout's event store,
`.llocal/harness.db`, through `adapters::events::Teller`. A lane's halt is
kept there as `Halted` — not in `errors.tsv`, whose last row the doctor
repairs from.

**`SIGTERM` is a soft stop.** The tick under way finishes, the lanes close
(`Lanes::closed_by` — no slot is free from then on, so no task starts), the
running lanes are waited for, and the watch exits 0, writing
`watch: draining …` then `watch: stopped …` to `watch.log`. A plain `pkill`
sends `SIGTERM`, so it drains too. A hard stop is `SIGKILL` to the watch's
tree and process group, sent from outside (the view's switch).

**A quiet tick costs one call.** Before the snapshot, one conditional read
(`GitHub::activity`, the newest issue or PR with `If-None-Match`): a `304`
is free; and one read of the integration branch's head
(`GitHub::branch_sha`), since a push that updates no issue — an upgrade, a
human's, the plan's graph written — moves it all the same. When nothing
moved — no head moved, no PR waiting on its checks, no lane just freed —
the tick ends there (`routing::may_skip`, `routing::Pulse`). The forty-odd
reads of a snapshot are paid when something changed, or every
`routing::FULL_READ_EVERY` ticks as a valve.

**Every building milestone branch follows the integration branch.** Each
tick reads the head (`common::catch_up::read_head`, one `branch_sha` call)
and, while a branch is behind it and it is not green, that commit's
own checks (`GitHub::commit_checks`, one more) — never the branch's newest
run, which seconds after a push still belongs to the head before. Both are
kept in the pulse (`CatchUps`), with what each milestone branch was brought
to. Once that commit is green (never merely "not red"), every branch behind
it is a free `Route::CatchUp` (weight 1010, before the merges;
`routing::read_catch_ups`) that merges that very sha: GitHub's merge first,
else one local merge a tick in the `catch-up` checkout with the generated
files regenerated — a file the base deleted removed — by the cargo the
merged tree pins. A merge that stales the mock-up's bundles
(`catch_up::staled`, two compares at most through `GitHub::changed_files`:
the head changed a file every bundle is built from since the branch's
merge base, or brings bundles while the branch changed such a file) skips
GitHub's merge: the local merge runs `mock/build.sh` even without a
conflict, or one side's components keep digests of the other side's file.
Held: the open task PRs into the branch take it (`refresh_task_prs`, never
a force; a PR a lane works on, or whose task's session still runs —
`catch_up::Busy` — skipped; a PR the merge stales left on its old base),
said by number. The merge train never lands a PR on a base that stales its
bundles (`milestone_merge::Attempt::Stale`): `router::routes` refreshes it
in the same `catch-up` checkout (`dispatch::catch_up::locally`, rebuilt,
pushed) and it lands once green again; one that cannot be refreshed is
labelled `harness:pr-fix`, and waits for that repair (`Attempt::Repairing`). A
residual conflict labels the landing PR `harness:pr-fix`, or is said once;
GitHub's merge alone is tried again every five minutes at the same head,
warned once while there is no landing PR. A failure is parked an hour,
said once, GitHub's merge alone tried every five minutes meanwhile; a
push the remote refuses as behind (a session pushed meanwhile) is merged
again next tick. A branch still on the legacy handler files meeting a head
that split them is split first in the same checkout, its add/adds merged
three ways on the split merge base (`workflows::common::handler_split`).
The tasks of a branch that lags the head — from the head's move to its
catch-up, its checks running included, at most 20 min from the head's
commit when no verdict ever comes — or one that could not take it, wait
(`Snapshot::catch_up`, `catch_up_held`) — `start_task`
and `run_dev_loop` no longer merge anything themselves; a catch-up waits
while a repair works on its landing PR, and that repair while the catch-up
is due. The board is read in full when the head moved, when its verdict
came in, while a catch-up due is unsettled (one left for the next tick
by the local budget, one whose push raced), the tick after one settled
(its tasks start), when a lag hold runs out, or when a retry is due — not
while a head is merely pending. `router::catching_up` hands `read_head`
the clock's second. A landing no longer catches the others up itself: it moved the
head, which the route follows.

**A red integration branch holds the plant.** On every full read, before the
snapshot, the sentinel (`workflows::common::sentinel::watch`) reads the
integration branch's own workflow runs (`GitHub::branch_checks`): a failed
run on its newest head opens one `harness:human` issue, `Integration branch
<b> is red` — found again by its title, never twice — and the schedule drops
what would build on the red or merge into it (`routing::base_red`: the
tasks, whose milestone branch takes the integration branch first, and the
two merges), keeping the reviews, the repairs, the rules, the test debt and
the decisions. The board is read in full every tick meanwhile, as for a PR
under CI, since the push that turns it green moves no issue; the hold lasts
until green — runs still in progress on the fix's push hold as the red did,
while the issue is open — and green closes the issue and the jobs resume.
The rules' task's PR, decided merged by the harness while its checks ran,
sits on no board: the snapshot reads it on its own
(`milestone_merge::rules_prs`, `rules_train`) and the train merges it into the
integration branch once green; one judged faithful whose checks still run
(`RulesPr::Checking`) is `rules_on_pr`, neither tapped nor attempted again —
`routing::RULES_HELD_READS` full reads at most (`routing::bound_rules_on_pr`,
before the taps), then released with a warning.
**A PR waits when the board says it waits** (`repairs_due`,
`workflows::common::contest::waiting`, the open and the closed rules' issues
read first): its contest open, its latest contest closed and not resumed yet,
or an issue a decided `wait` named still blocking — it is left out of the
repairs and the merges, and its task's decision is not routed; each full read
says `watch: pr #N waits on …`, and warns of an open contest naming no rule.
A PR still waiting on a settled contest after `routing::SETTLING_TELL` (60)
full reads is told once (`SettlingStalled`, a notification cleared by its
`Resumed`): its base never took the rules — a catch-up held. A
settled contest (`contest::settled`, one page of closed rules' issues) is
`Route::Resume`, free, weight 1005 — after the catch-ups, before the merges,
keyed on its PR (or on the milestone whose branch it lifts on, so that
branch's catch-up goes first): `run_resume` deletes `CONTEST.md` with a
contents API commit — or, with no lift of its own, merges a task PR's base
into its head so its checks run again — and marks the PR resumed
(`Resumed`), no checkout.
**The follow-ups H2 added, all free and read once a full read**
(`router::follow_ups`, dispatched by `routes::run_follow_up`):
- `Route::CloseMerged` (weight 1003, also on a red base: it merges nothing)
  — each building milestone's merged PRs read once (`merged_prs`), the open
  agent tasks one of them delivered closed (`common::close_merged`: a
  `Closes` line, or a title confirmed by the PR's head; no lane on the
  task), the PR's `Closes` line written first when it lacked it,
  `TaskDelivered`.
- `Route::ContractLanded` (1003) — a PR waiting on a contract change its
  repair stopped on (`PrWait::Contract`), the change closed and its work on
  the PR's base — the producer task the contract manager recorded on it
  landed there (`contest::landed_on_base`), or, with none recorded, the
  manifest (`contest::contract_landed`): resumed — `harness:pr-fix`, then
  the change's last word quoted on the PR, its repairs counted anew; a
  producer cut hands the PR's task to a human instead, the PR still
  waiting.
- `Route::DropDebtPr` (995) and `Route::MergeDebt` (985, not on a red base)
  — the PRs labelled `harness:debt-pr` (`test_debt::data::landing::ready`):
  a red, conflicting or out-of-bounds one closed with the reason
  (`close_issue` closes a PR: the issues endpoint serves both), a red one
  held while the base is red; the first
  green one merged into the integration branch and its issue closed
  (`milestone_merge::run`), at most one per `routing::DEBT_MERGE_EVERY` —
  `Pulse::debt_merged_at`, set only when the attempt merged (`landed`: a
  stale debt PR refreshed holds nothing), `routing::hold_debt_merges` drops
  the merges meanwhile.
A landing PR red or conflicting past its repairs is
`MergeOutcome::Escalated` from `MergeMainAgent`: `RepairsExhausted`
recorded once, when the decision was asked; a `harness:pr-fix` PR past its
cap no merge path covers is escalated by `repairs_due`
(`milestone_merge::exhausted_repairs`), `RepairsExhausted` the same way; the lanes' numbers (`Busy::numbers`) go in as
`main_agent_merge::Config::repairing`, and the train's `ready_to_merge` and
`repairs_due`'s `exhausted_repairs` get the same `running`: a PR a lane
repairs is never escalated under it, nor a PR whose task or milestone a lane
holds — `decide` takes `harness:needs-decision` off before its session, and
asked again meanwhile the question was decided twice (W, `ForgeDesMondes`'
#542, PR #547, 2026-10-25: two "repair" verdicts three minutes apart).
`decide` gets the integration branch as `Config::integration` (the shared
checkout's branch), `pr_fix` gets it as `Config::integration` and a `GitCli`
on its checkout as its `Git` port (`dispatch/pr_fix.rs`, the composition
root).
`--dry-run` reads and says, writes nothing.

Three rules, and they are not symmetric:

- **A failed routing read stops the loop.** Bad credentials or an unreadable
  repo will not fix themselves, and retrying silently forever hides them.
- **A failed dispatch does not.** One workflow's session hiccup is that
  workflow's business; it is logged and likely to succeed on a later tick.
- **A `QUOTA` stop makes the watch rest** (`routing::Rest`, kept in the
  `Pulse`): nothing paid is started for `FIRST_REST` ticks, twice as many
  after each stop in a row up to `LONGEST_REST`, and the count starts again
  once a lane's session went through (`Lanes::went_through`). Under
  `--parallel` the free work — the merges — goes on meanwhile; with one lane
  nothing is read. A lane that exits 3 counts as a stop too
  (`Lanes::struck_by_quota`). The rest is one `QuotaRest` event, a
  warning in the view. Before it, an exhausted window bought one `QUOTA` row
  and one doctor's visit per tick, four a minute at the default interval.

It builds one `GitHub` of its own for the decision alone — no checkout, since
a route is a decision and not a payload. The reads that a flat issue list
cannot answer happen here too (does the lowest roadmap item have a milestone,
are a milestone's tasks all closed, has a labelled PR actually broken), one
extra call per candidate — every candidate, since every one is a job. That cost is paid here
rather than inside a workflow so that a candidate which would skip never
costs a mounted checkout every thirty seconds.

## `dispatch/dev_loop.rs` — the deepest wiring

`GitCli`, `GitRepos`, `Checkpoint` are named here and nowhere else (`Checkpoint`
goes out as `Rc<dyn Checkpoints>`, so the loop holds the port, not the file);
`GhCli`, `RealDisk`, `ClaudeCliFactory`, `Rehearsal` and `LedgerSpending` are
named by every dispatch module (`shared.rs` builds them for the ones that
share a checkout), and `pr_review.rs` is the one place that names
`ReviewLedger`, handed over as `Rc<dyn ReviewCosts>`. None of it is ever named
in `harness-workflows` — that is what leaves every port a single test seam, and
what keeps a workflow unable to name an adapter.

**What a run's sessions are told.** Their environment is the toolchain's
(`HARNESS_BIN` in it) plus `HARNESS_FACTS`, the session facts' path in the
run's own folder (`<loop>/<run>/session-facts.json`, per run because a
parallel watch's lanes share the loop's folder; none for a dry run). The
signature index carries, beside the product's files, `harness-dataguard`'s
API (`dependency_index`): the lock file's `git` source, its commit found in
cargo's checkouts (`$CARGO_HOME/git/checkouts/*/<rev7>/crates/dataguard`),
else `cargo metadata --offline`; its `.rs` read through `Disk`, ordered by
the product's tracked `.rs`; logged `dataguard signatures: harness-dataguard
dataguard-v5 @ <sha7> — N module(s)…`; any failure an empty index and a
warning, never a halt. `--no-signatures` turns it off too.

**The assembly of the loop itself is not here** — `DevLoop`, its round factory
and its preflight gates are generic to the workflow and live in
`harness_workflows::dev_loop::run`. This file builds what no workflow has the
right to name, and hands it to `run::build`.

The order is imposed by the mount — **changed by D4** (`docs/to_build.md` §6):
the target repo is no longer necessarily the repository the harness was
launched from, so the two gates that read the target's branch and CI had to
move from step 2 to a new step after the mount:

1. open the log sink (console + the run's `run.log`);
2. **tooling gates, pre-mount, on the launching checkout** —
   `ClaudeIsRecentEnough`, `GhIsAuthenticated`, and `WorkingTreeIsClean` (only
   under `--no-workspace`): a several-hundred-megabyte clone must not precede
   the discovery that `gh` is not authenticated;
3. mount the workspace (`Provisioner::mount`), or run in place with
   `--no-workspace` (which halts if `TARGET_REPO_URL` names a repo other than
   this checkout's own `origin` — D4 step 4, `domain::same_repo`);
4. **tooling gates, post-mount, on the mounted workspace** —
   `TheIntegrationBranch` and `CiTriggersOnTheBranch`: wired here, not in
   step 2, because once the target comes from a setting (`TARGET_REPO_URL`)
   rather than the launching checkout, checking it there would check the
   *harness's own* branch and CI instead of the target's;
5. wire the ports **against the clone** — that is where sessions edit and where
   `gh` must answer;
6. read the resume pointer (it takes precedence over the table's choice),
   announce the run, build the workspace gates, `run::build`, execute;
7. **unmount no matter what** — that is what keeps a workspace carrying work and
   deletes one that carries none.

`--dry-run` is wired here as a `Rehearsal` session factory rather than branching
the framework: nothing downstream knows it is a rehearsal.

`Wanted.url`'s precedence (`mount`, step 3): `--workspace-url` wins when given,
then `TARGET_REPO_URL`, and empty still means "this checkout's own `origin`" —
`Provisioner`'s own fallback (`url_for`), unchanged.

## `dispatch/init_repo.rs` — the second entry point

Builds `GhCli::for_slug(&slug)` (no checkout — `init-repo` has a URL, not a
clone, like every subcommand that reads a board it has no checkout of), `RealDisk`, `GitRepos` and `Processes` (the install runs the
clone's `scripts/register.sh` through it), calls `harness_workflows::init_repo::run::build(...)`,
prints the report with `println!` (the one place in library-adjacent code
where that's correct — library code keeps `tracing`), and maps the result to
an exit code: `Ok((report, Verdict::Ready))` → 0, `Ok((report,
Verdict::StillBlocking))` → 1 (not a `Halt` — the writes it owns all
succeeded), `Err(halt)` → `halt.exit_code()`, same contract as `dev_loop`.

`main.rs` dispatches on `Cli::command` before anything else: `Some(InitRepo(..))`
goes here, `None` goes to `dev_loop::run` exactly as before this subcommand
existed.

## `dispatch/tooling.rs` — gates that belong to no workflow

`claude` on `PATH` and recent enough, `gh` authenticated, the integration branch
exists, CI triggers on it, the tree is clean. Generic over `S` because none reads
the state. They verify **in order**, because they assume each other.

The version gate is strict (`MINIMUM = 2.1.277`) and refuses rather than warns:
since that version `total_cost_usd` on a `--resume` call is cumulative for the
whole conversation, and the ledger reads a stage's last turn. Under an older
version it would undercount silently — a gate costs one local call, a false
`costs.tsv` is invisible.

Two gates are relaxed when the run works in a clone (`TheIntegrationBranch`'s
current-branch check, `WorkingTreeIsClean`): the human's own tree is not what the
run touches.

## `toolchain.rs` — one `PATH`, said once per start

The owner asked to see easily what is on the `PATH` the harness uses. One
resolver composes it, for the harness's own spawns on a product's behalf (a
product's script, the signature index's `npx tsc`) and for every Claude
session alike. One exception, a catch-up's local merge (`by_the_tree`): its
tree's pin is the merge's, known only once merged, so its `Processes` are
resolved unpinned — rustup's proxies ahead, which read the merged
`rust-toolchain.toml` — and the watch's own toolchain is left as it was:

1. `<factory>/.llocal/toolchain/bin`, the shims: `sed` → `gsed`, `grep` →
   `ggrep` when Homebrew installed them, `bash` → the first bash ≥ 5, `node`
   → the first node ≥ 22 (searched in the inherited `PATH`, then
   `/opt/homebrew/bin`, `/usr/local/bin`). **Links, not folders**: putting
   `/opt/homebrew/bin` ahead would move every other tool in it too —
   Homebrew's cargo 1.98 ahead of rustup's, `python3`, `git`. A shim is
   replaced atomically and taken back when its tool is gone; BSD `sed` once
   wrote a wrong registry while exiting 0;
2. the `bin` of rustup's toolchain for the channel the **target product's**
   `rust-toolchain.toml` pins (`<RUSTUP_HOME or ~/.rustup>/toolchains/<channel>`
   or `<channel>-*`). Without it — a product that pins nothing, a channel not
   installed, a pin that could not be read — rustup's proxies
   (`$CARGO_HOME/bin`, else the folder of the `rustup` found, links followed:
   Homebrew's keg), which honour a checkout's own pin or refuse it clearly;
3. the inherited `PATH`, in its own order, every folder once.

Problems, each naming its fix: a pinned channel not installed (`rustup
toolchain install <channel>`), clippy missing from it, **a pin that could not
be read** (`cannot read rust-toolchain.toml on <branch> of <repo>: <why>` — a
`gh` failure is never taken for "unpinned"), a cargo that runs and is not
rustup's (`install rustup`), bash < 5, node < 22, sed or grep not GNU.

What is said is what a process will run: each tool is looked up in the
composed `PATH` and asked `--version`, after composing. The line —
`Event::ToolchainResolved`, a warning when a problem is found, a notification
in the view until a start finds it whole — is said when the watch starts (the
pin read on GitHub, `at_watch_start`) and when a run mounts its checkout (the
checkout's own pin, `at_run_start`, from `shared::mount_named` and
`dev_loop`'s mount): every lane's `run.log` carries it. `harness toolchain`
prints it (exit 1 on a problem). Never the rest of the environment.

**The shared compilation cache.** When `sccache` is found — on the composed
`PATH`, else in `/opt/homebrew/bin` or `/usr/local/bin` — every spawned
process also gets `RUSTC_WRAPPER` (that sccache, by its full path),
`SCCACHE_DIR` (`SCCACHE_DIR`, a relative one read from the home; empty:
`~/.harness/sccache`, one cache for every factory), `SCCACHE_CACHE_SIZE`
(empty: `15G`), `CARGO_INCREMENTAL=0` — sccache caches no incremental
crate, and incremental is what grew a lane's `target` — and
`SCCACHE_IGNORE_SERVER_IO_ERROR=1`, a server out of reach falling back to
rustc. Only a sccache whose `--version` exits 0 is wired. `SCCACHE=off` in
`.env.local` sets none of them; off, or without a sccache that runs, an
inherited `RUSTC_WRAPPER` is cleared (set empty). The line names every
variable a process gets, values in full: the shell's (`SHELL=…
CLAUDE_CODE_SHELL=…`), then the cache's (`build cache: sccache 0.18.0 —
RUSTC_WRAPPER=/…/sccache SCCACHE_DIR=/…/sccache SCCACHE_CACHE_SIZE=15G
CARGO_INCREMENTAL=0 SCCACHE_IGNORE_SERVER_IO_ERROR=1`, or `off`, or `none,
sccache not found`, or `none, sccache does not run (…)`), then the
inherited build variables passing through (`; inherited: …`), then the
`PATH`. sccache missing is not a problem — a build without it is only
slower.

**How it reaches the adapters — a decision.** The adapters take it by
injection (`Processes::with_env`, `ClaudeCliFactory::with_env`); the launcher
hands it to every one it builds after a start, through
`toolchain::processes()` / `toolchain::environment()`, which read what the
last `take` resolved. That value is a `thread_local`, not threaded through
every `dispatch/*` signature: a start (the watch's, a run's mount) and the
adapters it builds sit far apart in the composition root, and a watch or a
run is one thread (`current_thread`) — while each test is its own thread, so
a test that resolves a toolchain never reaches another's spawns. The same
`Toolchain` carries `HARNESS_BIN` (this binary, `current_exe`, set by
`with_harness_bin` in `take` and `harness toolchain`): what every process
and session gets and what the `toolchain:` line says come from it — a
binary not found is a problem of the line. The closure check of
`gates-local` runs through it, in a `code` session and a repair one alike. Nothing in
`harness-adapters`, `core` or `workflows` holds it. `git` and `gh` keep the
harness's own environment; a command run by hand (`init-repo`, `doctor`)
before any start gets none.

Composition-root code by nature — it reads `PATH`, `HOME`, `RUSTUP_HOME`,
`CARGO_HOME`, `SCCACHE`, `SCCACHE_DIR`, `SCCACHE_CACHE_SIZE`, the disk — and only the launcher has it; `core` and `workflows`
never see it. The pure parts (`channel_of`, `version_in`, `major`, `Pin`) and
`resolve` over folders a test builds are tested here;
`at_watch_start`/`product_file`/`by_hand` wire `gh` and are excluded from the
mutants. `.llocal/toolchain` is a store the janitor and `harness doctor` know
(`cleanup::KNOWN`, `doctor::KNOWN_LLOCAL`): a link left right never changes,
so it looks idle for as long as a watch runs through it.

## `adapters/sink.rs` — both, always

Console **and** file. The file keeps everything regardless of the requested
level: it is what gets re-read afterwards, and a `--quiet` that truncated the
only trace of a night run would be a false economy. The level only concerns the
console, and `Logbook` already handles it.

## `adapters/spending.rs` — why the port is implemented here

`Ledger` knows how to write a row but not what time it is, what this run is
called, or what machine it runs on. Those three are facts of the **launcher**, so
the `Spending` implementation lives here — and that is what leaves
`harness-core` with no dependency on time.

## Current state

Everything `harness-workflows` assembles has an entry point here. Two are
typed by a human (`harness` → `dispatch::dev_loop`, `harness init-repo` →
`dispatch::init_repo`); the rest are reached by `harness watch`, which also
reaches `dev_loop`.

Three checkouts, and the difference is whether the tenant commits:

| checkout | id | who | why |
| --- | --- | --- | --- |
| exclusive | the repo's name | `dev_loop` | several sessions deep, leaves a checkpoint |
| shared, read-only | `router-readonly` | `planner`, `split`, `refinement`, `arbiter`, `pr_review` | none of them commits, so none can conflict |
| its own, writable | `router-prfix` | `pr_fix` | it commits; `force_reset`, on the PR's branch |
| its own, writable | `router-testdebt` | `test_debt` | it commits; `force_reset`, on the integration branch |
| its own, writable | `router-rules` | `rules` | it commits; `force_reset`, on the integration branch |

Under `--parallel`, a route runs on a lane and mounts the lane's own
`router-lane-<k>` instead of the shared or the writable id, so a writing
tenant (`pr_fix`, `rules`) and a read-only one (`pr_review`, `planner`,
`split`, a refinement) take turns in the same checkout. Two facts keep the
turns clean, both in `dispatch/shared.rs` and executed by
`Provisioner::release` / `Provisioner::mount`: every tenant **releases** the
checkout when it ends (`release_checkout`: an unfinished merge aborted, the
branch back at `origin`, untracked files and scratch branches gone), and every
router checkout is `drop_uncommitted` (a read-only mount that still meets
uncommitted leftovers drops them and says so, instead of halting). Commits no
remote has are kept whole either way, said as `Event::WorkKept`, and the next
mount halts naming the branch and how many — a human's gesture, not a
`--force-reset` a watch passes blindly.

`init_repo` and `main_agent_merge` mount nothing: both only talk to GitHub.

**Adding a workflow** means a new module in `dispatch/` that builds the
concrete adapters for that workflow's `Ports` — reusing `shared.rs` if it
never commits — plus a `Route` variant and a dispatch arm in `router.rs`. A
new `Command` variant in `cli.rs` only if a human should also be able to run
it by hand, which for a label-triggered workflow is usually not the case.
