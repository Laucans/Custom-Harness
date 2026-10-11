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

## Isolation of the slices, and what the planner must know to run them in parallel

*Parked 2026-10-10, by the human, while adding the capability-isolation gate.*

Findings from reading `dnd_helper2` and the harness as they are, with the
three changes made alongside (the isolation gate on the real graph, the
registration of a DataCapability without a shared file, the serialization
of structure changes per aggregate). Kept here because the next rework of
the planning and the cutting (`/planner`, `/split`) needs it.

### What is true today, and was not what the rules say

- **Rule 12 ("write-side tasks run one at a time") is not enforced at
  development time.** `write_side` only changes the end of a task: its PR
  stays open for a human. `runnable_tasks` (`dev_loop/data/tasks.rs`)
  launches every ready task whatever its side. DataCapabilities are already
  developed in parallel when the lanes allow it. The merge is the only
  serialization, and it is a review, not a lock.
- **The old isolation gate read the text of `Cargo.toml`**, not the graph.
  It missed a dependency declared at the workspace level
  (`foo.workspace = true`), a transitive one (a capability depends on a
  "utils" crate which depends on another capability), a shared crate named
  anything but `capabilities/`, `dataguard`, `dataqueue` or `resolver`, and
  code shared by `#[path]`, `include!` or a symlink. The new gate reads
  `cargo metadata`.
- **A DataCapability is as isolated as a Capability** (its own crate,
  dependent on the DataGuard alone; ~190 lines each in `dnd_helper2`). What
  made two of them conflict was the form, not the substance: adding one
  touched the workspace `Cargo.toml` (members), `serveur/src/startup.rs`
  (the registry) and `Cargo.lock` — commit `3edf97f` shows it. Globbed
  members and registration by discovery remove the shared lines.
- **Structure changes are the real serialization point.** An aggregate, an
  invariant, a migration, `data/model.json` live in the DataGuard and the
  schema, shared by construction. Two tasks changing the same aggregate in
  parallel collide there, whatever the isolation of the DataCapabilities.
  The key already exists in every manifest: `target.aggregate`.

### The rule of isolation, derived from principle 5 and rule 3

No whitelist, no human gate: three mechanical rules, which separate
infrastructure (downwards: the store, the network) from business code.

1. A Capability or a DataCapability depends on no other Capability or
   DataCapability, directly or transitively.
2. It depends only on crates of its own system (`crates/<system>/…`), never
   on another system's.
3. A crate shared inside a system depends on no Capability, no
   DataCapability and no crate of another system — so it is plumbing, never
   a bridge. Shared *meaning* stays a Concept with fixtures, which is not a
   crate.

Plus: no `#[path]`, `include!` or `include_str!` that leaves the crate's
folder, no symlink. The DataCapabilities follow the same three rules with
`crates/dataguard` as their one allowed dependency beside their system's
plumbing.

### What the parallelism becomes

| unit | parallel | what guarantees it |
| --- | --- | --- |
| Capability | unbounded | the isolation gate |
| DataCapability, pure (no structure change) | unbounded | the isolation gate + registration without a shared file |
| structure change (aggregate, invariant, migration, model) | one at a time **per aggregate**; different aggregates together | the planner's serialization |
| write-side merge | unchanged: a human reviews | rule 12 |

### What the planner lacks, and a proposal for `/planner` and `/split`

The `## Architecture` section (`harness_architecture::vocabulary::Declaration`)
carries `unit`, `system`, `concept`, `effect`, `touches`. It does **not**
carry the aggregate, so `runnable_tasks` cannot know that two tasks change
the same one.

Proposal, in three parts:

1. **An `aggregate:` line in `## Architecture`** for every unit that writes
   or changes structure (`data-capability`, `invariant`, `migration`), the
   aggregate as `target.aggregate` names it in the manifest. `Declaration`
   parses it; `runnable_tasks` holds back a task whose aggregate another
   running write-side task already declares. Two aggregates run together.
   (Done alongside: `Declaration.aggregate`, `held_aggregate`,
   `tasks::startable_tasks`, called by the lanes. The line is what `/split`
   must now write — its skill names it; the plan's JSON has the field. A
   task without the line holds nothing, so nothing changes until `/split`
   writes it.)
2. **`/split` cuts the data layer per aggregate, not as one task.** Today it
   opens the data layer as ONE task ("mutations are serialized behind the
   DataGuard, and so is this"). That reasoning conflated the queue's
   serialization at runtime with the development's. Cut instead: one task
   per aggregate for its structure (aggregate + invariants + migration),
   then one task per DataCapability, each `blocked_by` its aggregate's
   structure task only. Tasks of different aggregates carry no link between
   them and run in parallel.
3. **`/planner` orders milestones by the aggregates they touch**, not only
   by the data they read. Two milestones changing the same aggregate are
   chained; two milestones on different aggregates may be opened together.
   The milestone's `## Architecture` gains an `aggregates:` line, the
   planner's equivalent of the task's `aggregate:`.

Done since (`split` on the graph): `aggregate` is a unit, `migration` and
`invariant` read as it; `produces`/`needs` chain the tasks and the layers
only order the plan; the "interface contract" task is gone; a `profile`
(`strong`/`mid`/`small`) sets how much one slice holds. **Still parked:**
the profile is not deduced from the loop's model (`RunArgs.model`,
`DATA_LAYER_MODEL`) — `--split-profile` sets it, else `mid`; `split` reads
no run config today, and wiring one would couple it to the loop. Deduce it
at the place both are known (the watch's route), when a product runs on a
small model for real.

Open with it: whether a DataCapability's task should be allowed to *also*
add a field to its aggregate (one task, one unit says no — a field is the
aggregate's task), and what `/refinement` does with the `aggregate:` line
when the aggregate does not exist yet.
