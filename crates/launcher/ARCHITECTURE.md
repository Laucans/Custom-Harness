# `harness-launcher` — architecture

The entry points: the `harness` binary, the CLI, and **the only place in the
workspace that builds a concrete adapter**. The crate graph is in
[ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md).

Errors here are `anyhow` at the boundary (`.with_context(…)`), unlike the two
library crates, which carry typed `thiserror` enums — see `../../CLAUDE.md`.

## The modules

```
src/main.rs       entry point: parse, find the repo, dispatch, map the exit code
src/cli.rs        the arguments and the environment variables, declared together
src/dev_loop.rs   the wiring of the first workflow: the real adapters
src/init_repo.rs  the wiring of `harness init-repo`: the only place that builds `GhCli::for_slug`
src/tooling.rs    the gates that belong to no workflow
src/sink.rs       where log lines go: console + file
src/spending.rs   the `Spending` port, implemented: clock, run id, machine
```

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
code` keeps meaning the dev loop), and `Command::InitRepo(InitRepoArgs)` is the
one subcommand today.

## `dev_loop.rs` — the only concrete wiring

`GhCli`, `GitCli`, `GitRepos`, `RealDisk`, `ClaudeCliFactory`, `Rehearsal`,
`Checkpoint`, `LedgerSpending` are named here and nowhere else. That is what
leaves every port a single test seam, and what lets a workflow stay unable to
name an adapter.

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

## `init_repo.rs` — the second entry point

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

## `tooling.rs` — gates that belong to no workflow

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

## `sink.rs` — both, always

Console **and** file. The file keeps everything regardless of the requested
level: it is what gets re-read afterwards, and a `--quiet` that truncated the
only trace of a night run would be a false economy. The level only concerns the
console, and `Logbook` already handles it.

## `spending.rs` — why the port is implemented here

`Ledger` knows how to write a row but not what time it is, what this run is
called, or what machine it runs on. Those three are facts of the **launcher**, so
the `Spending` implementation lives here — and that is what leaves
`harness-core` with no dependency on time.

## Current state

`dev_loop` and `init_repo` have entry points, selected by `Cli::command`.
`pr_review` and `refinement` are assembled and tested in `harness-workflows`
(`run::build`), but nothing here calls them yet. Wiring a third workflow means
a new module beside `dev_loop.rs`/`init_repo.rs` that builds the same concrete
adapters for that workflow's `Ports`, and a new `Command` variant in `cli.rs`.
