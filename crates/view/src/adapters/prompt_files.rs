//! The prompt book and its drafts, through the harness's own stores — the
//! book through a read-only door: the view never writes what runs.

use std::path::Path;

use harness_adapters::store::prompt_book::{DraftFile, PromptFile, ShelfFile};
use harness_core::domain::prompt_book::{Drafts, PromptBook, Shelf};
use harness_core::ports::store::prompts::{DraftStore, PromptStore, ShelfStore};

use crate::ports::Prompts;

/// `.llocal/prompts.json`, read; `.llocal/prompts.drafts.json` and
/// `.llocal/prompts.shelf.json`, read and written.
pub struct PromptFiles {
    book: PromptFile,
    drafts: DraftFile,
    shelf: ShelfFile,
}

impl PromptFiles {
    /// The stores of the checkout whose state lives under `state_root`.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            book: PromptFile::new(state_root),
            drafts: DraftFile::new(state_root),
            shelf: ShelfFile::new(state_root),
        }
    }
}

impl Prompts for PromptFiles {
    fn book(&self) -> Result<PromptBook, String> {
        self.book.load().map_err(|halt| halt.reason().to_string())
    }

    fn drafts(&self) -> Result<Drafts, String> {
        self.drafts.load().map_err(|halt| halt.reason().to_string())
    }

    fn keep_drafts(&self, drafts: &Drafts) -> Result<(), String> {
        self.drafts
            .save(drafts)
            .map_err(|halt| halt.reason().to_string())
    }

    fn shelf(&self) -> Result<Shelf, String> {
        self.shelf.load().map_err(|halt| halt.reason().to_string())
    }

    fn keep_shelf(&self, deactivated: &Shelf) -> Result<(), String> {
        self.shelf
            .save(deactivated)
            .map_err(|halt| halt.reason().to_string())
    }
}
