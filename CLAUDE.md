# pipelinev2

Rust rewrite of `event_assistant/pipeline` (the unattended agent-loop
runner). Standalone repo, scaffold stage — architecture is being redecided
as part of the rewrite, not carried over by default.

## Project Context

- Package: defined in `Cargo.toml`; `edition = "2024"`, pinned `rust-version`
  (MSRV). Toolchain pinned via `rust-toolchain.toml` (`channel = "stable"`).
- Layout: `src/lib.rs` defines the public API; `src/main.rs` is a thin entry
  point that calls into the library. Integration tests live in `tests/` and
  only see the public API.
- External calls (subprocesses like `git`/`gh`, network, filesystem) are
  isolated behind trait-based adapters — core logic decides, an adapter
  executes. Tests substitute a fake adapter; nothing mocks at the call site.
  Carried over from the Python pipeline on purpose.
- Async runtime, if/when one is needed: one per process — `tokio`
  multi-thread for long-running loops, `current_thread` for CLI/tests. Don't
  add it speculatively.

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

- `cargo check --all-targets` before `cargo build` — faster, same type
  errors. Configure the editor to run it on save.
- `cargo clippy --all-targets --all-features -- -D warnings` and
  `cargo fmt --all` before every commit.
- `cargo audit` runs in CI (see `.github/workflows/ci.yml`); known
  advisories block the build until patched.
- `cargo update` is a deliberate action with its own PR — never bundled
  with feature work.
- Add `default-features = false` on a dependency when only some of its
  features are used.

## Common Commands

```
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --all-features --workspace
cargo audit
```
