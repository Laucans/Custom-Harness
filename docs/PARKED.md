# Parked questions

Decisions deliberately left for later, with what is already known. Each one
names who parked it and when; whoever picks it up removes it from here.

## How does an *asking* pin reach the next roadmap?

*Parked 2026-10-09, by the human, while building the pins' kinds.*

An **asking** is a pin on a board (the data model, for now) that holds a
change the human wants considered "when the next roadmap is built" — a
`request_change`. It has an AI session behind it, like a brainstorm; its
text is the request the AI formulated (or the human wrote).

Today it lives only in the view's store (`.llocal/harness.db`, table
`notes`, `kind = "asking"`), and nothing in the harness reads it.

Options on the table:

- **The planner reads the askings** when it builds a roadmap's milestones,
  through a port onto the pins' store, and marks each one consumed. The
  view would then write something a run reads — an exception to its
  read-only rule, like the prompt drafts.
- **An asking becomes a GitHub issue** (`harness:request-change`), opened
  from the pin on a click; the planner reads the label. Visible on the
  board, nothing new for the harness to read.
- **Something else** — e.g. a roadmap pass that collects the askings and
  asks the human to sort them.

To decide with it: when "the next roadmap" is built (the planner on a new
`harness:roadmap` issue? a dedicated step?), whether an asking is consumed
once or stays as a standing request, and what the human sees of it after.

## What the cutting still leaves open

*Parked 2026-10-10, by the human, after the planner and the split were
rewritten on the graph.*

The findings that led here, and the three changes they called for, are
done and in the history (`docs/PARKED.md` at `adea1d1` holds the full
reading): the isolation gate reads the real graph (`scripts/isolation.sh`),
a DataCapability is a folder, two tasks on one aggregate run in turn
(`tasks::startable_tasks`), `aggregate` is a unit, `produces`/`needs`
chain the tasks, milestones chain by aggregate, a `profile` sets how much
one slice holds, and `/business-analyst` and `/tech-analyst` read those
lines. What remains:

- **The profile is not deduced from the model that will build the
  tasks.** `harness split --split-profile` (or `SPLIT_PROFILE`) sets it;
  empty, it is `mid`. `split` reads none of the loop's configuration
  (`RunArgs.model`, `DATA_LAYER_MODEL`), and wiring one in would couple it
  to the loop. The place both are known is the watch's route
  (`router/routes.rs`): deduce it there — `config::profile_for` already
  maps a model name to a profile — when a product runs on a small model
  for real.
- **The products' documents still speak of `migration` and `invariant`
  as units.** `crates/architecture/assets/ARCHITECTURE.md` (contract D,
  the layers) and `CLAUDE-rules.md` (rules 6, 11, 13) name them; the
  vocabulary reads both as `aggregate`, so nothing breaks, but a product's
  own `docs/ARCHITECTURE.md` describes a cut the harness no longer makes.
  Align them when the products' documents are reworked, and re-install
  through `init-repo`'s marker versioning so the products pick it up.
- **A dry run reads the shared checkout as it is, and never refreshes
  it** (`provisioning.rs`, by design: a dry run clones, checks out and
  resets nothing). `router-readonly` under `.llocal/agentic_workspaces/`
  is refreshed only by a real planner/split/refinement run. A dry run
  after weeks without one reads a stale repository — the inventory said
  "no unit" on a product with two aggregates — and the prompt it prints
  is then about the wrong repository. Either refresh it by hand before a
  dry run (`git fetch && git reset --hard origin/<branch>` in that folder),
  or let the dry run fetch into the shared checkout when it is clean:
  the "a dry run doesn't clone" rule would stand, a fetch is not a clone.
- **One task, one unit, and a field added to an aggregate.** Should a
  DataCapability's task be allowed to *also* add a field to its
  aggregate? The rule says no — a field is the aggregate's task — and the
  graph says the command then `needs` a second aggregate task. Decide
  when it first happens.
- **What `/refinement` does with `aggregate:` when the aggregate does not
  exist yet** — the task that creates it carries the same line; the
  refinement gates may want to tell "creates" from "writes".
