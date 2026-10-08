# `harness-workflows` — architecture

The instances: what `harness-core` cannot name. Three workflows declared against
the framework, plus `common/` for what at least two of them actually read.

Read this before adding a workflow or moving a file inside one. The crate graph
is in [ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md); the framework
itself in [../core/ARCHITECTURE.md](../core/ARCHITECTURE.md); the repo's Rust
rules (lints, errors, `?Send`, docs on every `pub`) in `../../CLAUDE.md`.

Each rule below is here because its absence cost something. When a rule gets in
the way, read its reason before working around it.

## 1. The skeleton

A workflow is a directory, and it always has this shape:

```
<workflow>/
  mod.rs            the summary, and nothing else
  ports.rs          the ports: Rc<dyn Trait>, never a value
  config.rs         the settings: values, never a port
  run.rs            the assembly: Request + build()
  data/             what it reads and decides — pure business logic
  action/           what it writes
  checks/           what judges, without ever writing
  orchestration/    what sequences: stages.rs, round.rs, workflow.rs
```

The split is by **role**, not by subject. A file finds its place by answering
one question: *does it read, write, judge, or sequence?*

- `data/` — the state (the `S` of `Context<S>`) and the business rules that have
  neither I/O nor subprocess. A policy is tested here without network and
  without doubles.
- `action/` — everything that writes: an `Action`, a `SessionAction`. Also where
  the "writes" half of a split guard lives (rule 6).
- `checks/` — the `Verification`s. A stage's, not the launcher's: those belong
  to no workflow.
- `orchestration/` — `stages.rs` (the table), `round.rs` (the mounted round) and
  `workflow.rs` (the executable shape). Nothing else goes in.

`ports.rs`, `config.rs` and `run.rs` stay at the root because **none of the
three is a step**: the first two are the contract the launcher fulfills, the
third is the entry point that, with that contract filled, assembles the whole
workflow.

**A deterministic command departs from this skeleton**: no `orchestration/`,
no `checks/`. `init_repo` and `milestone_merge` are both this, not a
workflow — no `Context`, no stage table, no session, no cost — so a stage
table would name one entry for a sequence that does not exist, and a
`Verification` would judge against a `Context<S>` nobody carries. What
`checks/` would hold (a read-only audit's criteria, a merge's readiness
predicate) lives in that module's own `data::audit` as pure functions over
values already read. See `init_repo/mod.rs` and `milestone_merge/mod.rs`
for the same reasoning in place — the router (`common::routing` +
`harness watch`) is this too, though its own "audit" is a pure decision
function rather than a module of its own.

## 2. Ports ≠ Config

Two objects, never one:

- **`Ports`** carries what is **injected** — `gh`, `sessions`, `spending`,
  `locks`, `repo`, `disk`. What a test replaces with a fake.
- **`Config`** carries what **this run is worth** — a branch, a directory, a
  `SessionSpec`, a flag.

A single type for both existed (it was called `Wiring`) and was split: it
answered two different questions — "what is it made of" and "what does this run
change" — under one name, so no name was right.

The criterion is **nature, not form**: `now: fn() -> String` is a function
pointer and lives in `Ports`, because it is the injected clock — neither core nor
workflows carry a dependency on time.

`Config` is also distinct from core's `Settings`: `Settings` carries what
**every** workflow has (`--dry-run`, `--stages`) and core reads it from the
`Context` at every stage. `Config` is read once, at assembly.

## 3. The direction of dependencies

Outside tests, a `use crate::<workflow>::…` only goes this way:

```
data  ←  checks
data  ←  action
data, action, checks, ports, config  ←  orchestration
everything else  ←  run
```

That is: `data/` knows nobody, `checks/` and `action/` know only `data/`,
`orchestration/` knows everything but `run`, and **nothing depends on `run`**. A
`use` that climbs means a file is in the wrong place, not that an exception is
needed.

Checkable in one command:

```
grep -rn "^use crate::" <workflow>/{data,checks,action}/ | grep -v "::data\|common::"
```

## 4. Stage names are written in the table

A stage name (`"code"`, `"inline"`, `"router"`) is written in
`orchestration/stages.rs` and **descends as a field** to the gate or action that
needs it.

Never a constant shared between two layers, never a repeated literal. The
failure mode avoided: a gate looks in `ctx.results` for a stage's reply under a
name nobody wrote, doesn't find it, and concludes the stage was skipped — when
it ran, and was paid for.

## 5. Text lives with the table; the action receives it

Prompt templates live in `orchestration/` — in `stages.rs` when there are few,
in `prompts.rs` when there are seven. The action receives them as a field
(`template: &'static str`).

An action that went up to read them would invert the direction of composition
(rule 3), and the workflow would have two design surfaces instead of one.

## 6. Judging is not doing

A `Verification` **judges and does not write** (migration decision #1), and the
borrow checker holds it — it only ever receives `&Context`.

So a Python guard that wrote splits in two:

| what it did | what it becomes |
| --- | --- |
| re-reads, places a label, mutates state | a `Verification` in `checks/` **+** an `Action` in `action/` |

The price is one re-read per pair — a free local call against a session that
costs dollars.

## 7. The shape on one side, its assembly on the other

- `orchestration/workflow.rs` carries **the type** and its impls (`Workflow`,
  `Executable`). It names neither `Ports`, nor `Config`, nor `Request`.
- `run.rs` carries **the assembly**: a `Request` (what the invocation asks for,
  distinct from infrastructure) and a `build()` returning the mounted type.

What that buys: the shape is tested alone (remaining rounds, tolerance, lock)
without mounting the real table, and the wiring is tested alone (which field
receives what) without executing anything. The two files change for different
reasons — one when the workflow's semantics change, the other when the wiring
does.

**`run.rs` builds no concrete adapter — and neither does an action or a gate.**
Naming a `GhCli`, a `GitCli`, a `ClaudeCliFactory`, a `Checkpoint` or a
`ReviewLedger` is the launcher's job, and its alone: that is what leaves each
port a single test seam. A workflow imports from `harness_core::ports`, never
from `harness_core::adapters`, and it carries no `std::fs` call and no clock of
its own — if what it needs has no port yet, the port is what gets added
(`Checkpoints` and `ReviewCosts` were added exactly that way). The rule and its
two test-only exceptions are in `../../CLAUDE.md`.

## 8. One design surface, and only one

`orchestration::stages::table` **is** the sequence: the order of its entries is
the execution order, adding an entry adds a stage, removing one removes it.

This wasn't always true — the sequence used to live in a graph's `@listen`
decorators, and the table was only a registry of settings that could be reversed
entirely without execution changing. Any return to a declarative layer above the
table walks that road backwards.

Two accepted cases stay out of the table, each documented where it lives: a
**branch** (`dev_loop`'s rollover, which leaves when the sequence has nothing to
do) and **shared entries** (the repo map, which
`refinement::orchestration::round` puts in front — `stages.rs` has no way to read
the repo, and has no business having one). Both live in `round.rs`: the round
carries the order, and the assembly does not rewrite it.

## 9. The fakes

Every `ports.rs` and every `config.rs` carries its `#[cfg(test)] pub(crate) mod
fake`:

- `ports::fake::ports()` / `ports::fake::with(gh)`;
- `config::fake::config()`.

A test port **refuses** rather than doing nothing: mounting a table must open no
session and record nothing, and a silent port would let the opposite through
unnoticed. The exception is documented where it is taken — `refinement`
rehearses its sessions (`Rehearsal`) because its preflight tests run the whole
sequence in dry-run.

A fake is never invented at the call site: a fake **adapter** is injected. A
test that passes against a mock while the real adapter is broken is exactly the
failure mode this avoids.

## 10. The tests

- A test lives **with its subject**, in the module's `#[cfg(test)] mod tests`.
- A test may climb the stack — calling `run::build` to mount its subject is
  normal. Rule 3 is about production code.
- What is tested in `orchestration/stages.rs` is **order**; in `run.rs`, the
  **wiring**; in `data/`, the **policy**. A business rule is not tested through a
  mounted table.

## 11. `common/`

Only what **at least two** workflows actually need today goes in — not what might
one day serve a third.

`common/` does not take the four directories: these are not workflows. But rule
2 still holds — `explore` has its own `Ports` and `Config`.

Current contents:

| module | what it is |
| --- | --- |
| `labels` | the `harness:*` labels, created by `init-repo`; each preflight checks they exist, because a misspelled label makes a list empty and an empty list reads as "nothing left to do". Three of them say which side of the agent-native architecture a task is on: `read-side` (runs in parallel, merges alone), `write-side` (a human merges its PR), `review-pending` (that PR is open and waits). |
| `architecture` | the vocabulary of the agent-native architecture every target repo follows (`docs/ARCHITECTURE.md`, installed by `init-repo`): the `Unit` a task builds, the `Declaration` `split` writes under `## Architecture`, and the one rule derived from it — the task's `Side`, read back by the loop. |
| `sections` | the canonical sections of an issue body. `split` writes `Scope` and `Architecture`, which no refinement phase ever rewrites; the refinement writes the five after them. |
| `explore` | the repo map: `Ground` (free — glues `CLAUDE.md`, `docs/ARCHITECTURE.md`, `docs/PROJECT.md` and the tracked-file list verbatim) then one paid session that condenses it. A workflow wires both by implementing `Explored` on its state. |
| `fake_github` | `#[cfg(test)]` — an in-memory `GitHub` shared by the workflows' tests. A fake adapter, not a mock. |
| `fake_disk` | `#[cfg(test)]` — an in-memory `Disk`: what it holds reads back, what was written to it can be re-read. |
| `fake_locks` | `#[cfg(test)]` — `Grants`, a lock nobody else holds. It exists so mounting a table creates no `.lock-*` directory anywhere. |

## 12. A workflow is N rounds

The shape comes from core: `Workflow<S>` carries the precheck, the lock, the
`remaining` counter, the resume point and the summary, and calls `round(turn)`
once per turn. **A single run is a workflow whose `remaining` is 1** — there is
no "OneShot shape" and "loop shape", there is a number.

So every workflow has one round:

- it doesn't branch → core's `Round<S>`, mounted in `orchestration/round.rs`,
  with its `tolerance` if it has one;
- it branches → a hand-written type implementing `Executable`
  (`dev_loop::TaskRound`; the reason is in `docs/ROUND-DRAFT.md`, variant B).

Before that there were two shapes — a hand-written loop and a `OneShot` — which
is the same five concerns written twice, and core's `Round<S>` served nobody
because a stage's tolerance lived at workflow level. It now lives on the round,
that is, on whatever iterates the stages.

## 13. The workflows today

The skeleton is the same; what differs is justified by the nature of the
workflow, not by its history. Two tables, grouped by what a run's **subject**
is — an issue, or a pull request — because that is what decides the shape of
its precheck, its lock and its trigger.

**The four that work an issue:**

| | `dev_loop` | `refinement` | `planner` | `split` |
| --- | --- | --- | --- | --- |
| what it does | picks a task (or the one `--task` names, as a lane of a parallel watch), runs it through its stages, observes delivery — a merged PR on the read side, an open reviewed PR marked `review-pending` on the write side | rewrites an issue body into its five refined sections, never `Scope` or `Architecture` — a milestone gets the three business ones only, before `split` cuts it | opens the milestones of a roadmap item, each placed in the architecture (`systems`, `concepts`, side) | opens the tasks of a milestone, each one unit of the architecture, chained only where one builds on another |
| trigger | the `harness` binary, or the router | the `harness:refinement` label | `harness:ready` on a roadmap item with no milestone yet | `harness:ready` on a milestone |
| state | `Loop` (`Resumable` + `Scoped`) | `RefinementState` (`Explored`) | `PlannerState` (`Explored`) | `SplitState` |
| `remaining` | the `--rounds` budget | 1 | 1 | 1 |
| the round | `TaskRound`, hand-written | core's `Round<S>` | core's `Round<S>` | core's `Round<S>` |
| stages | `technical_refinement`, `code`, `create_test` | `router`, the phase sections (business: 3, technical: 2), `coherence`, `human-advice` (business), `publish` | the map, `context`, `plan`, `publish` | `context`, `slice`, `publish` |
| `orchestration/` | + `round.rs` | + `round.rs`, `prompts.rs` | + `round.rs` | + `round.rs` |
| `checks/` | + `preflight.rs` | — | + `gates.rs` | + `gates.rs` |
| lock | — | per issue | per roadmap item | per milestone |
| resume | `Checkpoint` | the counter in the issue comments | — | — |
| tolerance | — | — | one retry inside the `plan` action | one retry inside the `slice` action |

**The two that work a pull request:**

| | `pr_review` | `pr_fix` |
| --- | --- | --- |
| what it does | advisory second opinion on a PR, in fresh context | one repair attempt on a PR whose CI went red |
| trigger | the `harness:to-review` label | the `harness:pr-fix` label **and** a check concluded in failure |
| state | `ReviewState` | `FixState` |
| `remaining` | 1 | 1 |
| the round | core's `Round<S>` | core's `Round<S>` |
| stages | `inline`, `brief`, `publish` | `context`, `fix` |
| `orchestration/` | + `round.rs` | + `round.rs` |
| `checks/` | + `gates.rs` | + `gates.rs` |
| lock | per PR | per PR |
| resume | — | — |
| tolerance | `InlineMayFail` | — |

- `dev_loop` writes its round by hand because `pick` (choosing the task) and
  `delivered` (marking it) are **`Action`s, not `Stage`s**: forcing them into
  core's `Round<S>` table would subject them to the `--stages` and resume
  guards, which have no meaning for either. The others have nothing outside
  their table, so their `round.rs` mounts core's `Round<S>` and stops there.
- `dev_loop` has **preflight** gates (labels, milestone, skills, installed
  dependencies) because several opus sessions follow, and a `stat` costs less
  than a `/code`.
- `refinement` moves its seven templates into `prompts.rs`: same rule 5, just
  too much text to sit with the table.
- `refinement` and `planner` prepend the two shared `explore` entries to their
  sequence; `pr_review` tolerates a failed `inline` pass because the summary can
  still say something useful.
- `pr_review::data::notes` stays **in French**: it is published on a GitHub PR
  for a French-speaking human.
- `pr_fix` is the only workflow whose **first** stage writes: it removes its own
  trigger label before paying for anything, so a PR nobody can repair does not
  buy one session per poll. Its own `mod.rs` carries the reasoning.
- All six are wired into the binary — `dev_loop` directly (`harness`) and every
  one of them through the router's dispatch (`harness watch`). `pr_review` was
  the last to get a launcher entry point: its documented trigger used to be a
  hook on `gh pr create`, which needs a public URL this harness does not have.

Two more directories sit beside these, not in the table above: `init_repo`
and `milestone_merge`, both the deterministic-command departure described
in §1, not a workflow. Neither has a `trigger` beyond its own subcommand or
router route, no `state`, no `remaining`, no round, no lock, no resume, no
tolerance — every column above would read "n/a". `init_repo` creates the
`harness:*` labels and the integration branch, installs the architecture's
files in the repository through a throwaway clone (`docs/ARCHITECTURE.md`,
the rules block of `CLAUDE.md`, `contracts/`, the CI gates — one commit,
nothing already there overwritten without `--force`), runs a read-only
audit, and writes the link into `.env.local`. `milestone_merge` opens or merges a
milestone's PR once its tasks are closed and its CI is green.

## 14. Adding a workflow

In this order, because each step makes the next testable:

1. `data/state.rs` — the state and its bounds (`Resumable`, `Scoped`,
   `Explored`… according to what core demands);
2. `data/` — the business rules, tested without network;
3. `ports.rs` + `config.rs`, their `fake`s included;
4. `action/` and `checks/` — what writes, what judges, never both;
5. `orchestration/stages.rs` — the table, which names the stages;
6. `orchestration/round.rs` — the round: core's `Round<S>`, or a hand-written
   type if it branches;
7. `orchestration/workflow.rs` — the shape: `impl Workflow`, and the `remaining`
   this workflow carries;
8. `run.rs` — `Request` + `build()`;
9. the launcher — the concrete adapters, and those only.

Then `mod.rs`: a summary saying what the workflow does and pointing at the four
directories. No logic in a `mod.rs`.
