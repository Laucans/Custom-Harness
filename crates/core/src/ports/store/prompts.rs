//! Where the human's prompt edits are kept — the ports.
//!
//! Two stores, two writers, so whatever proposes a change never writes what a
//! run reads:
//!
//! - [`DraftStore`] holds the drafts: written by the view, where the human
//!   edits a prompt; read by nothing that runs, except the harness's own
//!   start, which takes them.
//! - [`PromptStore`] holds the book, what runs: written only by the harness,
//!   when it starts and applies the drafts it took; read once per run.
//!
//! Synchronous, like [`Checkpoints`](crate::ports::store::checkpoint::Checkpoints):
//! a few kilobytes gain nothing from `await`.
//!
//! **An unreadable store is never an empty one.** Reading the book as empty
//! would send the defaults while the human believes their edit runs. A missing
//! store is the harmless case and reads as empty; anything else is
//! [`Halt::Unreadable`](crate::domain::Halt::Unreadable).
//!
//! The implementations — JSON files under `.llocal/` — live in
//! `harness_adapters::store::prompt_book`.

use crate::domain::Outcome;
use crate::domain::prompt_book::{Drafts, PromptBook};

/// Where the book — what runs — is kept.
pub trait PromptStore {
    /// The book as last saved; empty when nothing was ever saved.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if a book exists
    /// but cannot be read or decoded.
    fn load(&self) -> Outcome<PromptBook>;

    /// Keeps the whole book, replacing the previous one at once.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if it could not be
    /// written; the previous book is then still the one read.
    fn save(&self, book: &PromptBook) -> Outcome<()>;
}

/// Where the drafts — what waits for the next start — are kept.
pub trait DraftStore {
    /// The drafts as last saved; none when nothing waits.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if they exist but
    /// cannot be read or decoded.
    fn load(&self) -> Outcome<Drafts>;

    /// Keeps every draft, replacing the previous ones at once.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if they could not be
    /// written.
    fn save(&self, drafts: &Drafts) -> Outcome<()>;

    /// Takes every draft away at once and answers them: a draft saved after
    /// this call waits for the next start rather than being lost.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if they cannot be
    /// read; they are then left where they were.
    fn take(&self) -> Outcome<Drafts>;
}
