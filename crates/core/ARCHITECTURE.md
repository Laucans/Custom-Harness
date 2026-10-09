# `harness-core` — architecture

The framework: vocabulary, traces, execution shapes, ports, adapters. It names
no workflow, and Cargo refuses the reverse dependency — see
[ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md) for the crate graph.

The shape is **hexagonal**: `domain` + `execution` + `ports` are the inside,
`adapters` is the only outside. A port is declared in `ports/` and implemented
in `adapters/` — never the other way round, and never in the same module.

This file describes the **current** state. Rust rules (lints, errors, `?Send`,
docs on every `pub`) live in `../../CLAUDE.md`.

## The five top-level modules

```
src/lib.rs            re-exports only — no public item lives here
  domain/             pure vocabulary: no disk, no subprocess, no library
  traces/             the run journal (leaf: imports nothing else from core)
  ports/              what the inside needs from the outside, as traits
  adapters/           one implementation per port — the only outside
  execution/          what runs: data, action, checks, orchestration
```

`domain` names no workflow even in comments; that is what makes the reverse
dependency inconceivable rather than merely forbidden.

`domain`, `execution` and `ports` never name a type from `adapters` — a
dependency that way round is the hexagon leaking, and it is reviewable with
`grep -rn 'crate::adapters' src/domain src/execution src/ports`, which must
answer nothing outside a `#[cfg(test)]` block.

## `domain/` — the vocabulary

| item | what it carries |
| --- | --- |
| `Halt`, `Severity` | stops as values, not exceptions. Four variants, and **the exit code is a frozen contract**: `Halted`/`Unreadable` → 1, `Failed` → 2, `Quota` → 3. `Unreadable` stays distinct from `Halted` because "I could not read" must never be spent as "there is nothing to do". |
| `Verdict`, `Outcome<T>` | what an executable returns — `Continue`, `Skip(why)`, `NothingLeft(why)`; `Outcome<T> = Result<T, Halt>`. |
| `Issue`, `Pr` | the **form** of a tracked item. What a label *means* is a workflow's definition, never here. |
| `Spend`, `Tokens` | what a turn cost. Every field is `Option`: `None` reads as "not observed", never "zero". |
| `breaker` | the circuit breaker: `fingerprint` (FNV-1a of a prompt), `trailing_failures`, `tripped`, `LIMIT` = 3, and the `outcome` words `STOP`/`FAILED`/`QUOTA` that `Halt::prefix` returns. Identity is task + stage + **prompt**, which is what lets a reworded issue clear the count with no gesture on the ledger. `QUOTA` is transparent: nothing ran, nothing was billed. |
| `markers` | the verbal contract — `AGENT_LOOP_OK`, `AGENT_LOOP_STOP`. |
| `prompts` | the preamble, the `Scope` block, and `splice` — one substitution pass, so values coming back from GitHub can't be re-substituted. `Named`, `Scope`, `Scoped`. |
| `workspace` | `Strategy`, `Wanted`, `Workspace`. **Two roots**: `root` is the code, `state_root` is the accounting — which is what makes a workspace disposable. Describes only; mounting lives in `execution::provisioning`. |
| `Resumable` | what a round state must expose so a stage is never billed twice. |
| `remote` | `Slug` (`owner`, `name`), `Slug::parse`, `same_repo` — the only place a GitHub URL is named. `same_repo` moved here from `execution::provisioning`, which now calls it rather than carrying its own copy. |

## `traces/` — the journal

`Logbook`, `Sink`, `Verbosity`. A leaf: it knows nothing of `Halt` or `Verdict`,
as it knows nothing of workflows. The sink is injected (the launcher writes to
console *and* file).

A line carries its level at its head: none, `warning: ` (`Logbook::warn`) or
`error: ` (`Logbook::error`). An `Event` says its own (`Event::level`): a
`FAILED` stop, a lane that could not start or ended on a signal or exit 2 is an
error; a `STOP` or a `QUOTA` is a warning — the same split as `Halt::severity`,
held by a test since the leaf cannot import it. The view colours by that head.

## `ports/` — what the inside needs from the outside

One module per external component, traits only (plus the few values those
traits exchange). **A port decides, an adapter calls.**

```
agent         Session, SessionFactory, SessionSpec, Reply
shell/git     Repo, Repos — reads answer, setup verbs return `Ran`
shell/github  GitHub — a failed read is never an empty list
shell/disk    Disk — exists so deletion is testable
shell/process Ran, last_line — what a shell port hands back
store/spending    Spending, Entry — implemented in the launcher, not here
store/lock        Locks — one bearer per name
store/checkpoint  Checkpoints, Pointer — where a run says what it was doing
store/review      ReviewCosts — the one *read* of a review's spending
```

`Sink` ([`traces/`](#traces--the-journal)) is a port too, and stays with
`Logbook`: it is the journal's own collaborator, not an external component the
execution layer reaches for.

Two of these are declared here and **not** implemented here: `Spending`, which
needs a clock, a run id and a machine name — all facts of the launcher — and
`Sink`. That inversion is what leaves `harness-core` with no dependency on
time.

## `adapters/` — the outside, wrapped

One implementation per port, one submodule per external component. Nothing
outside the launcher names a type from this module.

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

## `execution/` — what runs

Four private submodules, re-exported flat from `execution/mod.rs`: two public
paths to one type would be two ways to import it.

```
data/           Context<S>, Settings            what a run carries
action/         Action, SessionAction, Open, Unpaid, ask_and_record
checks/         Gate, InThisRun, StageAlreadyDone, MarkDone
orchestration/  Stage, StageBody, Round, Tolerance, Workflow, Lock
traits.rs       Verification, Executable, Guarded
provisioning.rs Provisioner, Mount, Run
```

`traits` sits beside the summary rather than inside one of the four: it defines
the base trait of two of them. `provisioning` is separate because the launcher
calls it *around* a workflow, not from within one.

### The three traits, and the rule they hold

- `Verification<S>` receives `&Context<S>` — **immutable**. "A check judges and
  never writes" (decision #1) is held by the borrow checker, not by convention.
- `Executable<S>` has `pre()`, `post()` and `perform()` — the work proper to
  one level.
- `Guarded<S>` is a **blanket impl** over every `Executable`: pre-gate →
  `perform` → post-gate. A blanket impl rather than a default method, so the
  sequence cannot be bypassed — a hand-written `Guarded` would conflict with
  it. There is exactly one path to execute anything.

### The hierarchy

```
Workflow<S>          precheck → lock → N rounds → summary
  └─ round(turn)     an Executable: Round<S>, or a hand-written type if it branches
       └─ Stage<S>   gates + a body
            └─ StageBody::Session { spec, sessions, actions }   paid
               StageBody::Local   { actions }                   free
```

- **`Context<S>`** carries `settings`, `state: S` (declared by the workflow),
  `traces`, and `results: HashMap<String, Reply>` — what each paid stage
  answered, keyed by stage name. Results are not state: a reply feeds the next
  stage of the same pass; it is not re-read from a resume, it is re-paid.
- **`Settings`** is immutable (`dry_run`, `stages`). Mounting the workspace
  finalizes it *before* the `Context` exists, which is what removed the Python
  habit of rewriting `cfg.workspace` in place (decision #6).
- **`Stage` is the point of variation** — paid or free, nothing else. A `Round`
  only ever holds `Stage`s, so the hierarchy is held by types.
- **`Round<S>`** iterates stages in order and carries `tolerance`: tolerating a
  failed stage is a statement about the stages of a sequence, so it belongs to
  whoever iterates them, not to the workflow.
- **`Workflow<S>`** carries `remaining: Cell<u32>`, `precheck`, `lock`,
  `round(turn)`, `between`, `remember`/`forget`, `summary`. **A single run is a
  workflow whose `remaining` is 1** — there is no second shape, only a number.
  The core decides the order and the turn counting; it decides nothing about
  the round. `Executable` is implemented by hand on each workflow (two
  delegating lines) because a blanket impl would collide with `Round`'s and
  `Stage`'s.
- `remember()` is called **before** propagating a round's failure: a round that
  stopped halfway is exactly the one a resume needs. `forget()` runs only once
  a round delivered.

### `checks/guards.rs` — what every stage undergoes

`InThisRun` (the `--stages` filter), `StageAlreadyDone` (resume) and `MarkDone`.
The first two judge, the third writes — decision #1 applied to what used to be
three branches inside one Python function, impossible to read from the table or
test alone.

### `provisioning.rs` — mount and unmount

What `domain::workspace::Wanted` declares, this executes: find or clone, reset
to `origin`, delete at the end when disposable. Three rules, each against a way
to lose work: nothing is overwritten silently (`--force-reset` is the human's
exit), nothing is deleted silently, and **a dry-run doesn't clone**.

## Where to add what

- a new word with no I/O → `domain/`
- a new external binary, file or API → a trait in `ports/<family>/`, its
  implementation in `adapters/<family>/`, and only `shell/process.rs` spawns
- a new shape shared by more than one workflow → `execution/`, in the category
  that answers *does it carry, write, judge, or sequence?*
- anything that needs a clock, a run id, or a machine name → declare a port
  here, implement it in the launcher
