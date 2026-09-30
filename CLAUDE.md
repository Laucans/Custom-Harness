# harness

Rust rewrite of `event_assistant/pipeline` (the unattended agent-loop
runner) as an agent harness: it drives Claude Code sessions through
verification gates, rather than just moving data through stages. Standalone
repo, currently staged inside `event_assistant` under the working directory
`pipelinev2/` pending its move to its own repository — the directory name is
provisional, the crates below are the actual project identity. Scaffold
stage — architecture is being redecided as part of the rewrite, not carried
over by default.

## Project Context

- Cargo workspace, three crates, one direction only:
  `harness-launcher` (bin) → `harness-workflows` (lib) → `harness-core`
  (lib). `core` never depends on `workflows` or `launcher` — enforced by
  Cargo, not a lint: a `use` the wrong way is a cyclic-dependency error, not
  a warning. See `docs/MIGRATION.md` for why this mirrors the Python
  original's three packages.
  `edition = "2024"`, pinned `rust-version` (MSRV) at the workspace level
  (`[workspace.package]`); each crate's `Cargo.toml` inherits it
  (`edition.workspace = true`). Toolchain pinned via `rust-toolchain.toml`.
- Within `harness-core`: `src/lib.rs` re-exports `domain/`, `traces/`,
  `execution/` — no public item lives directly in `lib.rs`. Integration
  tests for a crate live in that crate's own `tests/` and only see its
  public API (see `crates/workflows/tests/depends_on_core.rs`).
- External calls (subprocesses like `git`/`gh`, network, filesystem) are
  isolated behind trait-based adapters — core logic decides, an adapter
  executes. Tests substitute a fake adapter; nothing mocks at the call site.
  Carried over from the Python pipeline on purpose.
- Async: `#[async_trait(?Send)]` on every execution trait, tokio
  `current_thread`. No `Send` bound to pay for — the harness drives one
  session at a time, and this is a deliberate migration decision
  (`docs/MIGRATION.md`), not a default.

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

- Lints: crate root sets `#![warn(clippy::pedantic, clippy::nursery,
  missing_docs, rust_2018_idioms)]` (already set). CI runs
  `cargo clippy --all-targets --all-features -- -D warnings`. No blanket
  `#[allow]` at the crate root — narrow allows only, with a comment
  explaining why.
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
