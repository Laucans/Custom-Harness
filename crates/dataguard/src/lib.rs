//! The write side of the agent-native architecture, once.
//!
//! Every product the harness builds mutates its data through one frontier:
//! a typed command (a `DataCapability`, contract F) enters the `DataQueue`
//! (contract G), the `DataGuard` checks it — permissions, the effect it
//! declares against the effect it has, the aggregate's invariants (contract
//! I) — the Resolver works out what a mutation of existing data reaches
//! (contract J), one applier applies the head of each partition and takes
//! the next `dataVersion`, and the authors hear what happened (contract L).
//! A conflict between two authors is never a rejection: the later one is
//! told what was declared ahead, and confirms or cancels; what nobody can
//! settle goes to a human with the AI's proposal beside it (contract K).
//!
//! This crate is that engine, independent of any product: the rules are
//! pure ([`queue`], [`guard`], [`resolver`], [`reconcile`]), the stores are
//! ports ([`store`]) with an in-memory implementation every test runs on
//! ([`memory`]) and a PostgreSQL one behind the `postgres` feature, and the
//! wire protocol is the one the mock server of every product already
//! speaks ([`api`]). A product's `crates/dataguard/server` wires it to its
//! aggregates and `DataCapabilities`; a Capability never sees it.

#![deny(unsafe_code)]
#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]

pub mod aggregate;
pub mod api;
pub mod command;
pub mod engine;
pub mod guard;
pub mod manifest;
pub mod memory;
pub mod messages;
pub mod queue;
pub mod reconcile;
pub mod registry;
pub mod resolver;
pub mod store;

#[cfg(feature = "postgres")]
pub mod postgres;

/// What the engine refuses, and why.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// A command names a `DataCapability` the server does not register.
    #[error("no DataCapability named {0}")]
    UnknownDataCapability(String),
    /// A command names a partition or a command id the queue does not hold.
    #[error("not found: {0}")]
    NotFound(String),
    /// The caller may not trigger this command.
    #[error("{0}")]
    Forbidden(String),
    /// A gesture the command's state does not admit (confirming an applied one).
    #[error("{0}")]
    NotNow(String),
    /// A store did not answer.
    #[error("the store failed: {0}")]
    Store(String),
    /// The engine was given units that do not fit together: a
    /// `DataCapability` listing an invariant its aggregate does not declare.
    #[error("misconfigured: {0}")]
    Misconfigured(String),
    /// A read names a query the registry does not serve.
    #[error("{0} is not a registered persisted query")]
    UnregisteredQuery(String),
}

/// What an engine call answers.
pub type Result<T> = std::result::Result<T, Error>;
