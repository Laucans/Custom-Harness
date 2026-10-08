# harness

Rust agent harness: it drives Claude Code sessions through
verification gates, rather than just moving data through stages. Standalone
The crates below are the actual project identity. Scaffold
stage — architecture is being redecided as part of the rewrite, not carried
over by default.

## Role
You are one of the many sessions used to build the harness

## Interaction context
If the user speak in french, translate in english before to think, then before to answer translate to french.
Use less word as possible, speak with light sentence, straight to the point and avoid large answers, use dotlist with few words instead, if user want more details onto a point he will ask.

## Objective
The objective is to build a re-usable harness, first workflow will focus onto development, but another user can use harness-core to build another agent workflow, for example to build report to answer a question as a strategy consultant can do.

## Project Context

- **Architecture: `ARCHITECTURE_OVERVIEW.md`** — the split, each crate's role,
  and how the dependencies form. Current detail per crate in
  `crates/<crate>/ARCHITECTURE.md`; read the one for the crate you are touching
  before moving a file or adding a module.
- Cargo workspace, five crates, one direction only:
  `harness-launcher` (bin) → `harness-workflows` (lib) → `harness-core`
  (lib), and `harness-view` (bin) beside the launcher on the same arrow.
  `core` never depends on `workflows`, `launcher` or `view` — enforced by
  Cargo, not a lint: a `use` the wrong way is a cyclic-dependency error, not
  a warning. `view` is read-only: it shows the traces the harness leaves in
  `.llocal/logs` and the GitHub board, and never writes anything a run reads.
  Its one hand on the plant is the steward — an interactive Claude Code in a
  pseudo-terminal the human drives from the page (`crates/view/ARCHITECTURE.md`).
  The drawing is `harness-view-render`, a Bevy scene compiled to WebAssembly
  by `scripts/build-render.sh`; it depends on none of the other crates
  (`crates/view-render/ARCHITECTURE.md`).
- `edition = "2024"`, pinned `rust-version` (MSRV) at the workspace level
  (`[workspace.package]`); each crate's `Cargo.toml` inherits it
  (`edition.workspace = true`). Toolchain pinned via `rust-toolchain.toml`.
- Within `harness-core`: `src/lib.rs` re-exports `domain/`, `traces/`,
  `ports/`, `execution/` — no public item lives directly in `lib.rs`.
  Integration tests for a crate live in that crate's own `tests/` and only
  see its public API (see `crates/workflows/tests/depends_on_core.rs`).
- External calls (subprocesses like `git`/`gh`, network, filesystem) are
  isolated behind trait-based ports — core logic decides, an adapter
  executes. Tests substitute a fake adapter; nothing mocks at the call site.
  Carried over from the Python pipeline on purpose.
- Async: `#[async_trait(?Send)]` on every execution trait, tokio
  `current_thread`. No `Send` bound to pay for — the harness drives one
  session at a time, and this is a deliberate migration decision
  (`docs/MIGRATION.md`), not a default.

## Hexagonal Architecture — non-negotiable

The harness is a ports-and-adapters design and every change keeps it one.
`ARCHITECTURE_OVERVIEW.md` carries the diagram and the reasoning; these are
the rules a diff is checked against:

- **A port is a trait in `harness-core/src/ports/`**, one module per external
  component, traits and the values they exchange only. Never declare a port in
  the same module as an implementation of it: the dependency runs
  `adapters -> ports`, and a trait sitting in `adapters/` makes the next
  workflow family inherit `gh` along with the seam.
- **An adapter is an implementation in `harness-core/src/adapters/`**, and it
  is the only layer allowed to spawn a process, touch the disk, or call a
  library. `shell/process.rs` is the only place that spawns.
- **The inside never names a type from `adapters`.** `domain/`, `execution/`,
  `ports/` and every workflow take `Rc<dyn Trait>`; only the two outer rings
  build a concrete adapter — `harness-launcher` builds `GhCli`, `GitCli`,
  `ClaudeCliFactory`, `Checkpoint`, `ReviewLedger`, `DirLocks`; `harness-view`
  builds `GhCli` and its own read-only `FsTraces`, in its `main.rs` and
  nowhere else. Two exceptions, both test-only: a fixture may wire `Rehearsal`
  as its session factory (core's own stand-in carrier, the one `--dry-run`
  wires), and a test may name a concrete adapter when that adapter's real
  behaviour is what is under test — `DirLocks` in core's workflow test, which
  proves the lock is really released. Everywhere else, inject a fake from
  `workflows/src/common/fake_*.rs`.
- **No `std::fs`, no `Command`, no clock in a workflow, an action or a gate** —
  not even three lines of it. If a store the inside reads has no port yet,
  declare one (`Checkpoints`, `ReviewCosts` exist for exactly that reason)
  rather than reaching for the concrete type.
- **Where `core` needs a fact of the process** — the time, a run id, a machine
  name, a console — it declares a port and the launcher implements it
  (`Spending`, `Sink`). Never the reverse.
- Reviewable in one command:
  `grep -rn 'adapters::' crates/core/src/domain crates/core/src/execution crates/core/src/ports crates/workflows/src`
  Its only legitimate hits are doc links and the two test exceptions above —
  anything in shipped code is the review finding, not a detail.

## Core Rules

- Ownership: prefer owned types (`String`, `Vec<T>`, `PathBuf`) in public
  APIs; borrow inside implementations. Don't expose explicit lifetimes in
  `pub` signatures unless `Cow<'_, _>` is genuinely the right call.
- Borrowing: never hold a `std::sync::Mutex` guard across `.await`; use
  `tokio::sync::Mutex` or restructure to drop the guard first. No `&mut`
  aliasing tricks via raw pointers.
- Errors: the library crate derives `thiserror::Error` on a typed `enum`,
  with `#[from]` for transparent wrapping and `#[source]` for chains. The
  binary returns `anyhow::Result<T>` and adds `.with_context(|| ...)?` at
  every boundary.
- `unsafe`: `#![deny(unsafe_code)]` at the crate root (already set in
  `src/lib.rs`). If a justified case ever needs it, scope the `deny` down
  locally with a `// SAFETY:` comment immediately above the block explaining
  which invariants hold and why; every `unsafe fn` documents its
  preconditions in a `# Safety` doc section.
- Concurrency: spawned tasks (`tokio::spawn`) keep their `JoinHandle` and are
  awaited or aborted on shutdown — fire-and-forget tasks leak. Long-running
  loops honor cancellation via `tokio::select!` against a shutdown signal.

## Style Rules
- Comments have to be in english, if you are editing something where comments are french. Remove them and add an understandable comment in english (prefer few explicit words)
- Lints: crate root sets `#![warn(clippy::pedantic, clippy::nursery,
  missing_docs, rust_2018_idioms)]` (already set). CI runs
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`. No
  blanket `#[allow]` at the crate root — narrow allows only, with a comment
  explaining why.
  **One exception, already taken**: `clippy::future_not_send` is allowed at
  the root of `harness-core`. It is not a silenced defect — the lint
  presupposes `Send` futures, and migration decision #3 chose
  `#[async_trait(?Send)]` throughout, so every trait method's future is
  non-`Send` by construction and the lint would fire on nearly every
  `async fn`. Adding a new root-level allow needs the same kind of
  justification: the lint must contradict a documented decision globally, not
  merely be inconvenient locally.
- Naming: `snake_case` for modules, functions, variables, fields;
  `CamelCase` for types, traits, enum variants; `SCREAMING_SNAKE_CASE` for
  `const`/`static`. No Hungarian notation, no abbreviations beyond
  well-known ones (`url`, `id`, `db`).
- Formatting: `cargo fmt --all -- --check` runs in CI. No hand-formatted
  blocks.
- Docs: every `pub` item carries `///` doc comments with a one-sentence
  summary on the first line. `pub fn` returning `Result` documents
  `# Errors`; functions that can panic document `# Panics`.
- Logging: use `tracing`, not `println!`/`eprintln!`, in library code.
  `println!` is fine in `main.rs` for actual CLI output.

## Testing Rules

- Unit tests live inside the module in a `#[cfg(test)] mod tests { ... }`
  block — they have access to private items.
- Integration tests live in the top-level `tests/` directory; each
  `tests/foo.rs` is a separate crate that imports only the public API.
- Property-based testing with `proptest` for any function with non-trivial
  input space — parsers, validators, serde round-trips.
- Async tests use `#[tokio::test]`; sync tests use `#[test]`.
- CI runs `cargo test --all-features --workspace`.
- Never mock an external system in a unit test — inject a fake adapter (see
  Project Context) instead. A mocked test passing while the real adapter is
  broken is the failure mode this guards against.

## Security Invariants

- No `unwrap()` or `expect()` outside `tests/`, `examples/`, `benches/`.
  `expect("...")` is allowed only when the message documents an invariant
  the type system can't express.
- No `todo!()`, `unimplemented!()`, or `panic!()` as flow control reachable
  from `pub` APIs in shipped code.
- Integer arithmetic on untrusted input uses `checked_*`, `saturating_*`, or
  `wrapping_*` explicitly.
- Prefer `.get(i)` returning `Option` over indexing (`vec[i]`) on any
  offset derived from untrusted input.
- Never log secrets, tokens, or PII. `#[serde(skip)]` on secret fields;
  redact at the boundary, not after the fact.

## Workflow Rules

- Hooks live in `.githooks/` (tracked) — `git config core.hooksPath
  .githooks` once per clone to enable them. `pre-commit` runs
  `rustfmt --check` on the staged `.rs` files only.
- `cargo check --workspace --all-targets` before `cargo build` — faster,
  same type errors. Configure the editor to run it on save.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` and
  `cargo fmt --all` before every commit.
- `cargo audit` runs in CI (see `.github/workflows/ci.yml`); known
  advisories block the build until patched.
- `cargo update` is a deliberate action with its own PR — never bundled
  with feature work.
- Add `default-features = false` on a dependency when only some of its
  features are used.

## Common Commands

```
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo audit
```
