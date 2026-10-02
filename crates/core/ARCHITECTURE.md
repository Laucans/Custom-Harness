# `harness-core` — architecture

The framework: vocabulary, traces, execution shapes, adapters. It names no
workflow, and Cargo refuses the reverse dependency — see
[ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md) for the crate graph.

This file describes the **current** state. Rust rules (lints, errors, `?Send`,
docs on every `pub`) live in `../../CLAUDE.md`.

## The four top-level modules

```
src/lib.rs            re-exports only — no public item lives here
  domain/             pure vocabulary: no disk, no subprocess, no library
  traces/             the run journal (leaf: imports nothing else from core)
  adapters/           external dependencies, wrapped
  execution/          what runs: data, action, checks, orchestration
```

`domain` names no workflow even in comments; that is what makes the reverse
dependency inconceivable rather than merely forbidden.

## `domain/` — the vocabulary

| item | what it carries |
| --- | --- |
| `Halt`, `Severity` | stops as values, not exceptions. Four variants, and **the exit code is a frozen contract**: `Halted`/`Unreadable` → 1, `Failed` → 2, `Quota` → 3. `Unreadable` stays distinct from `Halted` because "I could not read" must never be spent as "there is nothing to do". |
| `Verdict`, `Outcome<T>` | what an executable returns — `Continue`, `Skip(why)`, `NothingLeft(why)`; `Outcome<T> = Result<T, Halt>`. |
| `Issue`, `Pr` | the **form** of a tracked item. What a label *means* is a workflow's definition, never here. |
| `Spend`, `Tokens` | what a turn cost. Every field is `Option`: `None` reads as "not observed", never "zero". |
| `markers` | the verbal contract — `AGENT_LOOP_OK`, `AGENT_LOOP_STOP`. |
| `prompts` | the preamble, the `Scope` block, and `splice` — one substitution pass, so values coming back from GitHub can't be re-substituted. `Named`, `Scope`, `Scoped`. |
| `workspace` | `Strategy`, `Wanted`, `Workspace`. **Two roots**: `root` is the code, `state_root` is the accounting — which is what makes a workspace disposable. Describes only; mounting lives in `execution::provisioning`. |
| `Resumable` | what a round state must expose so a stage is never billed twice. |
| `remote` | `Slug` (`owner`, `name`), `Slug::parse`, `same_repo` — the only place a GitHub URL is named. `same_repo` moved here from `execution::provisioning`, which now calls it rather than carrying its own copy. |

## `traces/` — the journal

`Logbook`, `Sink`, `Verbosity`. A leaf: it knows nothing of `Halt` or `Verdict`,
as it knows nothing of workflows. The sink is injected (the launcher writes to
console *and* file).

## `adapters/` — the outside, wrapped

One submodule per external component. **A port decides, an adapter calls.**

```
agent/    claude_cli   one process per turn, stitched with `--resume`
          rehearsal    the `--dry-run` carrier: returns the prompt, opens nothing
          (traits)     Session, SessionFactory, SessionSpec, Reply
shell/    process      the only place in the crate that spawns a subprocess
          git          Repo, Repos — every call names the repo with `-C <root>`
          github       GitHub — issues go through `gh api`, not `gh issue`
          disk         Disk — exists so deletion is testable
store/    checkpoint   the resume point: a two-line pointer + one JSONL per flow
          ledger       the cost ledger, frozen header, new columns appended
          review_ledger  a separate ledger: a review charges per PR, not per round
          lock         Locks — an atomic `mkdir`, one bearer per name
          spending     the Spending **port**; its impl lives in the launcher
```

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
- a new external binary, file or API → `adapters/<family>/`, and only
  `shell/process.rs` spawns
- a new shape shared by more than one workflow → `execution/`, in the category
  that answers *does it carry, write, judge, or sequence?*
- anything that needs a clock, a run id, or a machine name → declare a port
  here, implement it in the launcher
