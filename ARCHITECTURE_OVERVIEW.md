# Architecture overview

Three crates, one direction. Each crate's own `ARCHITECTURE.md` carries the
detail; this file is only the split and why the dependencies form the way they
do.

```
harness-launcher (bin)  →  harness-workflows (lib)  →  harness-core (lib)
```

## The role of each crate

| crate | role | detail |
| --- | --- | --- |
| `harness-core` | **the framework**: vocabulary, traces, execution shapes, adapters. Names no workflow. | [crates/core/ARCHITECTURE.md](crates/core/ARCHITECTURE.md) |
| `harness-workflows` | **the instances**: three workflows declared against that framework — `dev_loop`, `pr_review`, `refinement`. | [crates/workflows/ARCHITECTURE.md](crates/workflows/ARCHITECTURE.md) |
| `harness-launcher` | **the entry points**: the `harness` binary, the CLI, and the only place that builds a concrete adapter. | [crates/launcher/ARCHITECTURE.md](crates/launcher/ARCHITECTURE.md) |

## How the dependencies form

- **The arrow is the whole design.** `core` never names `workflows` or
  `launcher`; a `use` the wrong way is a Cargo cyclic-dependency error, not a
  lint. "The framework ignores its users" is held by the crate graph, not by a
  test.
- **What makes that possible is generics, not indirection.** Everything in
  `core` that touches run state is generic over `S` — `Context<S>`,
  `Workflow<S>`, `Stage<S>`, `Verification<S>` — and a workflow supplies its own
  `S`. No downcast, no registry, no `dyn Any`.
- **Where `core` needs something from above, it declares a trait and the layer
  above implements it.** `Spending` is declared in `core` and implemented in the
  launcher, because writing a ledger row needs a clock, a run id and a machine
  name — all facts of the launcher. That inversion is what leaves `core` with no
  dependency on time.
- **A workflow never names a concrete adapter.** It receives `Rc<dyn Trait>`
  ports; the launcher builds `GhCli`, `GitCli`, `ClaudeCliFactory`, the ledgers.
  One test seam per port, and `--dry-run` is a wiring choice (a `Rehearsal`
  session factory) rather than a branch in the framework.
- **Reusability is the point.** A different workflow family — a consulting
  report, say — depends on `harness-core` only and declares its own states,
  stages and ports. Nothing in `core` has to change to accept it.

Per-crate rules that aren't visible in the code: `CLAUDE.md`. Migration
decisions the shapes refer to by number (#1 judge ≠ write, #3 `?Send`, #6
immutable `Settings`, #13 the core counts turns): `docs/MIGRATION.md`.
