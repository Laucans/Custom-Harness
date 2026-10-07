#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// The sole exception to "no `allow` at the root" (`CLAUDE.md`), and it is
// categorical rather than local: `clippy::nursery` includes `future_not_send`,
// which presupposes we want `Send` futures. Decision #3 of
// `docs/MIGRATION.md` says the opposite — `#[async_trait(?Send)]` everywhere,
// tokio `current_thread`, because the harness drives one session at a time and
// has no `Send` bound to pay for. The future of each trait method is thus
// non-`Send` by construction, and everything awaiting it is also: the lint
// would fire on nearly every `async fn` in the package. Documenting site by site
// would say "known defect" where there is only disagreement with an intentional choice.
#![allow(clippy::future_not_send)]

//! The harness framework: vocabulary, traces, execution.
//!
//! Nothing here imports `harness-workflows` or `harness-launcher` — and Cargo
//! refuses to compile the reverse. The invariant "the framework ignores its
//! users" is thus maintained by the crate graph at compile time,
//! rather than by a test that walks ASTs.

pub mod adapters;
pub mod domain;
pub mod execution;
pub mod ports;
pub mod traces;
