# `harness-launcher` — architecture

The entry points: the `harness` binary, the CLI, and **the only place in the
workspace that builds a concrete adapter**. That is the outer ring of the
hexagon: every `Rc<dyn Trait>` a workflow receives is chosen here, and nowhere
else. The crate graph is in
[ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md); the rules the
split is checked against are in [../../CLAUDE.md](../../CLAUDE.md).

Errors here are `anyhow` at the boundary (`.with_context(…)`), unlike the two
library crates, which carry typed `thiserror` enums — see `../../CLAUDE.md`.

## The modules

Three roots and two folders, and the split between them is **who decides to
run something** versus **what running it takes**:

```
src/main.rs       entry point: parse, find the repo, dispatch, map the exit code
src/cli.rs        what a human types: the arguments and their environment variables
src/router.rs     what decides on its own: `harness watch`, the polling loop

src/dispatch/     one module per thing that can be run, plus what they share
  dev_loop.rs       the first workflow: an exclusive workspace, several sessions deep
  planner.rs        a roadmap item into milestones
  split.rs          a milestone into tasks
  refinement.rs     an issue body into its five canonical sections
  pr_review.rs      an advisory review on a PR
  pr_fix.rs         one repair attempt on a red PR — the only writable checkout here
  milestone_merge.rs  a finished milestone's PR, opened then merged
  init_repo.rs      `harness init-repo`: the only place that builds `GhCli::for_slug`
  shared.rs         the checkouts and adapters the dispatch modules share
  tooling.rs        the gates that belong to no workflow

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
code` keeps meaning the dev loop). Two subcommands today:
`Command::InitRepo(InitRepoArgs)` and `Command::Watch(WatchArgs)`. The other
six workflows have **no** subcommand on purpose: they are reached by the
router, whose own arguments (`--interval`, `--once`, `--dry-run`) are what a
human tunes instead.

## `router.rs` — the trigger nobody types

`harness watch`: read a snapshot, call `harness_workflows::common::routing::
decide` (pure, tested without a fake adapter), dispatch, sleep `--interval`
seconds, repeat. `--once` does a single pass.

Two rules, and they are not symmetric:

- **A failed routing read stops the loop.** Bad credentials or an unreadable
  repo will not fix themselves, and retrying silently forever hides them.
- **A failed dispatch does not.** One workflow's quota or session hiccup is
  that workflow's business; it is logged and likely to succeed on a later tick.

It builds one `GitHub` of its own for the decision alone — no checkout, since
a route is a decision and not a payload. The reads that a flat issue list
cannot answer happen here too (does the lowest roadmap item have a milestone,
are a milestone's tasks all closed, has a labelled PR actually broken), one
extra call per candidate, stopping at the first match. That cost is paid here
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
clone) and `RealDisk`, calls `harness_workflows::init_repo::run::build(...)`,
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
| shared, read-only | `router-readonly` | `planner`, `split`, `refinement`, `pr_review` | none of them commits, so none can conflict |
| its own, writable | `router-prfix` | `pr_fix` | it commits; `force_reset`, on the PR's branch |

`init_repo` and `milestone_merge` mount nothing: both only talk to GitHub.

**Adding a workflow** means a new module in `dispatch/` that builds the
concrete adapters for that workflow's `Ports` — reusing `shared.rs` if it
never commits — plus a `Route` variant and a dispatch arm in `router.rs`. A
new `Command` variant in `cli.rs` only if a human should also be able to run
it by hand, which for a label-triggered workflow is usually not the case.
