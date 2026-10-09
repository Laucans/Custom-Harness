//! Where the human's prompt edits are kept — the port.
//!
//! Read by the launcher once per run, before a workflow builds its stages;
//! written by the view, where the human edits a prompt. Synchronous, like
//! [`Checkpoints`](crate::ports::store::checkpoint::Checkpoints): a few
//! kilobytes gain nothing from `await`.
//!
//! **An unreadable book is never "no edits".** Reading it as empty would send
//! the defaults while the human believes their edit is running. A missing book
//! is the harmless case and reads as empty; anything else is
//! [`Halt::Unreadable`](crate::domain::Halt::Unreadable).
//!
//! The implementation — one JSON file under `.llocal/` — lives in
//! `harness_adapters::store::prompt_book`.

use crate::domain::Outcome;
use crate::domain::prompt_book::PromptBook;

/// Where the prompt book is kept.
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
