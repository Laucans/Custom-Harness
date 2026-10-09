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
shell/    process      the only place in the crate that spawns a subprocess
          git          GitCli, GitRepos — every call names the repo with `-C <root>`
          github       GhCli — issues go through `gh api`, not `gh issue`
          disk         RealDisk
store/    checkpoint   Checkpoint: a two-line pointer + one JSONL per flow
          ledger       the cost ledger, frozen header, new columns appended
          review_ledger  ReviewLedger: a review charges per PR, not per round
          lock         DirLocks — an atomic `mkdir`
          events       the `EventLog` over SQLite, `.llocal/harness.db`
          error_ledger what stopped a run, and what was done about it
          prompt_book  PromptFile (`.llocal/prompts.json`), DraftFile (`.llocal/prompts.drafts.json`, taken by a rename), a run's `prompt-texts.json`
          trial        a dry-run's report, `.llocal/trials/<id>/trial.json`
```

`ledger` and `error_ledger` carry no port: the launcher writes them and the
hexagon never reads them, so a trait would be a seam with one side.

Three invariants here are the expensive ones:

- **A failed read never returns an empty list.** `[]` reads as "this milestone
  has no open task left", which is the input that triggers a `/planner`: an
  expired token would buy an opus run. A failed read returns
  `Halt::Unreadable`.
- **An unreadable store is never "nothing ran yet"** — the two answers are a
  `/code` session apart.
- **A dry-run is a wiring choice**, not a framework branch: the stage opens a
  session without knowing `Rehearsal` is behind it.

## Where to add what

- a new external binary, file or API → its trait in
  `harness-core/src/ports/<family>/` first, then the implementation here in
  `src/<family>/`
- a store the hexagon reads needs a port; one only the launcher writes
  (`ledger`, `error_ledger`) does not
