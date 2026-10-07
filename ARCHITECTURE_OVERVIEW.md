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
| `harness-core` | **the framework**: vocabulary, traces, execution shapes, ports, adapters. Names no workflow. | [crates/core/ARCHITECTURE.md](crates/core/ARCHITECTURE.md) |
| `harness-workflows` | **the instances**: three workflows declared against that framework — `dev_loop`, `pr_review`, `refinement`. | [crates/workflows/ARCHITECTURE.md](crates/workflows/ARCHITECTURE.md) |
| `harness-launcher` | **the entry points**: the `harness` binary, the CLI, and the only place that builds a concrete adapter. | [crates/launcher/ARCHITECTURE.md](crates/launcher/ARCHITECTURE.md) |

## The shape: hexagonal, and it is a rule

The harness is a **ports-and-adapters** (hexagonal) design, and every change is
expected to keep it that way:

```
        workflows (declare states, stages, and which ports they need)
                      │  names ports only
                      ▼
   ┌──────────────────────────────────────┐
   │  harness-core — the inside           │
   │    domain/      the vocabulary       │
   │    execution/   what runs            │
   │    ports/       traits, no I/O       │
   └──────────────────────────────────────┘
                      ▲
                      │  adapters -> ports, never the reverse
   adapters/ (git, gh, claude, the disk, the ledgers)
                      ▲
                      │  builds one concrete adapter per port
                 launcher
```

Four rules, in the order they are most often broken:

1. **A port is declared in `core::ports`, never beside its implementation.** A
   trait that sits in `adapters/` reads as part of the outside, and the next
   workflow family — a consulting report with no `git` in it — inherits the
   binary along with the seam.
2. **The inside never names a type from `adapters`.** `domain`, `execution`,
   `ports` and every workflow take `Rc<dyn Trait>`; the launcher builds
   `GhCli`, `GitCli`, `ClaudeCliFactory`, `Checkpoint`, the ledgers. A
   `use ...::adapters::` in a workflow is the review finding, not a detail.
3. **Anything that leaves the process goes through a port.** A `std::fs` call
   or a spawned binary in a workflow, an action or a gate is the violation even
   when it is three lines: it is also what makes that code untestable without
   a real folder.
4. **A store the inside reads needs a port too** — not only the ones that look
   like infrastructure. `Checkpoints` and `ReviewCosts` exist for exactly that
   reason.

What the rules buy, concretely: one test seam per port, fakes instead of mocks,
`--dry-run` as a wiring choice rather than a branch in the framework, and a
workflow family that reuses `harness-core` without inheriting `gh`.

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
  session factory) rather than a branch in the framework. The two exceptions are
  test-only and listed in `CLAUDE.md`.
- **Reusability is the point.** A different workflow family — a consulting
  report, say — depends on `harness-core` only and declares its own states,
  stages and ports. Nothing in `core` has to change to accept it.

Per-crate rules that aren't visible in the code: `CLAUDE.md`. Migration
decisions the shapes refer to by number (#1 judge ≠ write, #3 `?Send`, #6
immutable `Settings`, #13 the core counts turns): `docs/MIGRATION.md`.
