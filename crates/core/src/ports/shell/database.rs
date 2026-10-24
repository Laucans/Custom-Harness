//! The product's PostgreSQL server, as the watch's sweep sees it: the
//! databases its role owns, and a drop that never takes one in use.
//!
//! The tests of a product create a database each, and one that panics,
//! times out or is killed by `cargo mutants` never drops its own
//! (`ForgeDesMondes`, 2026-10-24: 7 939 databases, 68 GB). The sweep that
//! cleans up after them decides in `harness-workflows::scratch_sweep`; this
//! port lists and drops, nothing more. The implementation connects with the
//! factory's own role (`DATABASE_URL`), never a superuser:
//! `harness_adapters::shell::database::PgDatabases`.

use std::time::Duration;

use async_trait::async_trait;

use crate::domain::Outcome;

/// The databases the factory's role owns, and how to drop them.
#[async_trait(?Send)]
pub trait Databases {
    /// Every database the connected role owns, with its live connections.
    ///
    /// # Errors
    ///
    /// The server could not be reached, or refused the listing.
    async fn owned(&self) -> Outcome<Vec<OwnedDatabase>>;

    /// Drops each of `names` nobody is connected to — a plain
    /// `DROP DATABASE`, never `WITH (FORCE)` — in order, on one connection,
    /// until `within` has passed or the server stops answering. The answer
    /// holds one entry per name reached; the names after it are left for
    /// the next sweep, so a hung server holds the watch `within`, no more.
    ///
    /// # Errors
    ///
    /// The server could not be reached at all.
    async fn drop_idle(&self, names: &[String], within: Duration) -> Outcome<Vec<Dropping>>;
}

/// One database the role owns, as the server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedDatabase {
    /// Its name, as the server spells it.
    pub name: String,
    /// The sessions connected to it right now.
    pub connections: u32,
    /// Whether it is a template (`datistemplate`): never dropped.
    pub template: bool,
}

/// What the drop of one database came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropping {
    /// The database.
    pub name: String,
    /// What its drop came to.
    pub dropped: Dropped,
}

/// What a drop came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    /// The database is gone, or was already.
    Gone,
    /// Someone connected to it: it is kept (`object_in_use`).
    Busy,
    /// The server refused for another reason (a right, a lost connection,
    /// no answer in time), in its words — never the URL's.
    Refused(String),
}
