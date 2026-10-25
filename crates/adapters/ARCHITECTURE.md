# `harness-adapters` — architecture

The outside of the hexagon: one implementation per port declared in
`harness-core::ports`, one module per external component. It is the only
layer allowed to spawn a process, touch the disk, or call a library, and only
`shell/process.rs` spawns.

## Why a crate of its own

- **The rule is held by Cargo.** `harness-core` and `harness-workflows` do not
  depend on this crate, so a workflow naming `GhCli` does not compile.
- **`core` stays reusable.** A workflow family with no `git` in it depends on
  `harness-core` and inherits none of `gh`, the `claude` CLI or SQLite.
- **Only the outer rings build it.** `harness-launcher` wires every adapter in
  its composition root (`dispatch/*.rs`, `router.rs`); `harness-view` builds
  `GhCli` in its `main.rs`. `harness-workflows` takes it as a dev-dependency
  only, for `Rehearsal` in test fixtures.

## The modules

```
agent/    claude_cli   one process per turn, stitched with `--resume`
          rehearsal    the `--dry-run` carrier: returns the prompt, opens nothing
          stream_log   the JSON stream, read into `Reply`
shell/    process      the only place in the crate that spawns a subprocess; `Processes`
                       carries the variables it was built with (`with_env`: the
                       launcher's toolchain `PATH`) to every child it spawns
          git          GitCli, GitRepos — every call names the repo with `-C <root>`
          github       GhCli — issues go through `gh api`, not `gh issue`; a PR's
                       check rows keep, per run id and job name, the newest attempt
                       only (`latest_per_job`: a re-run's SUCCESS or PENDING hides
                       its old FAILURE), never collapsed across runs (a landing PR's
                       `push` and `pull_request` runs stay apart); `delete_file`
                       is a contents API DELETE against the blob's sha
          disk         RealDisk
          database     whether the product's database answers (a TCP connect), and
                       PgDatabases — the databases the factory's role owns and a plain
                       DROP DATABASE that keeps one in use, a sweep's drops on one
                       connection within its deadline; the crate's only Postgres
                       client (tokio-postgres, the version harness-dataguard locks)
store/    checkpoint   Checkpoint: a two-line pointer + one JSONL per flow
          ledger       the cost ledger, frozen header, new columns appended
          review_ledger  ReviewLedger: what a PR cost, read from the one ledger on its rows
          lock         DirLocks — an atomic `mkdir`
          events       the `EventLog` over SQLite, `.llocal/harness.db`
          factories    the `Factories` register over SQLite, `~/.harness/factories.db`, outside every checkout; a `creations` table beside it
          error_ledger what stopped a run, and what was done about it
          prompt_book  PromptFile (`.llocal/prompts.json`), DraftFile (`.llocal/prompts.drafts.json`, taken by a rename), ShelfFile (`.llocal/prompts.shelf.json`, the deactivated drafts, never taken), a run's `prompt-texts.json`
          trial        a dry-run's report, `.llocal/trials/<id>/trial.json`
```

`ledger` and `error_ledger` carry no port: the launcher writes them and the
hexagon never reads them, so a trait would be a seam with one side.

The cost ledger (`.llocal/logs/agent-loop/costs.tsv`) is the one register of
what the harness spent, and what a row was spent on is written at the
source. Its last column, `milestone`, is `#<n> <title>`
(`harness_core::domain::milestone_label`) of the milestone the run worked
for: the dev loop, split, refinement and the arbiter set it from what they
already read (`Context::tags`); every other workflow leaves it empty rather
than guess. A row older than the column has fewer cells and reads as "no
milestone known". `Ledger::backfill_milestones` fills those rows once
(`harness backfill-milestones`): it copies the file to one of its own
(`costs.tsv.bak`, or the first free `costs.tsv.bak.<n>` when an earlier
backfill left one, which it never overwrites nor removes), writes the new
text beside it and renames it over the ledger, and never
touches a row that names its milestone — a second run changes nothing. It
refuses a first line that is no header the ledger ever had (a renamed
column, a missing header): cells it cannot place by name are not rewritten,
and no data row is taken for a header. The rewrite holds the ledger's
advisory lock (`File::lock`), which `Ledger::append` takes too and, once it
has it, reopens the ledger if its file was renamed over meanwhile — so a
writer of this version waits and appends to the new file. A writer that
does not lock (a watch or a run still on an older binary) is not kept out:
the rename is refused if the ledger's file, size or modification time
changed since it was read (its own copy removed, nothing changed), but a row
appended between that last look and the rename lands in the replaced file
and is lost. The window is a few system calls wide — narrow, not closed.
**Run the backfill with the watch stopped**: `harness backfill-milestones`
refuses while the checkout's watch lock is held, and holds it itself.

Three invariants here are the expensive ones:

- **A failed read never returns an empty list.** `[]` reads as "this milestone
  has no open task left", which is the input that triggers a `/planner`: an
  expired token would buy an opus run. A failed read returns
  `Halt::Unreadable`.
- **An unreadable store is never "nothing ran yet"** — the two answers are a
  `/code` session apart.
- **A dry-run is a wiring choice**, not a framework branch: the stage opens a
  session without knowing `Rehearsal` is behind it.

**The toolchain is injected, never ambient.** The launcher's resolver
(`harness-launcher`'s `toolchain.rs`) composes a `PATH` and a `SHELL`; the
adapters that spawn on a product's behalf are built with them —
`Processes::with_env` (a product's scripts, `cargo`, `npx`) and
`ClaudeCliFactory::with_env` (every session: the toolchain first, the
session's own variables after, which win on a shared name). Nothing in this
crate holds it globally: an adapter built without it spawns with the
harness's own environment, as `GitCli` and `GhCli` always do.

## Where to add what

- a new external binary, file or API → its trait in
  `harness-core/src/ports/<family>/` first, then the implementation here in
  `src/<family>/`
- a store the hexagon reads needs a port; one only the launcher writes
  (`ledger`, `error_ledger`) does not
