//! The human's prompt edits on disk: `.llocal/prompts.json`, beside the stores.
//!
//! One file, written aside then renamed: a run reading it while the view
//! writes sees the old book or the new one, never half of either. A file at
//! the yard's root is a store to the janitor: never swept.

use std::path::{Path, PathBuf};

use harness_core::domain::prompt_book::PromptBook;
use harness_core::domain::{Halt, Outcome};
use harness_core::ports::store::prompts::PromptStore;

/// The file, under `.llocal/`.
pub const FILE: &str = "prompts.json";

/// `.llocal/prompts.json` of one checkout.
pub struct PromptFile {
    dir: PathBuf,
}

impl PromptFile {
    /// The book of the checkout whose state lives under `state_root`.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            dir: state_root.join(".llocal"),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE)
    }
}

impl PromptStore for PromptFile {
    fn load(&self) -> Outcome<PromptBook> {
        let path = self.path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PromptBook::default());
            }
            Err(e) => return Err(unreadable(&path, &e.to_string())),
        };
        if text.trim().is_empty() {
            return Ok(PromptBook::default());
        }
        serde_json::from_str(&text).map_err(|e| unreadable(&path, &e.to_string()))
    }

    fn save(&self, book: &PromptBook) -> Outcome<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| wrote_nothing(&self.dir, &e.to_string()))?;
        let text = serde_json::to_string_pretty(book)
            .map_err(|e| wrote_nothing(&self.path(), &e.to_string()))?;
        let path = self.path();
        let aside = self.dir.join(format!("{FILE}.new"));
        std::fs::write(&aside, text).map_err(|e| wrote_nothing(&aside, &e.to_string()))?;
        std::fs::rename(&aside, &path).map_err(|e| wrote_nothing(&path, &e.to_string()))
    }
}

fn unreadable(path: &Path, detail: &str) -> Halt {
    Halt::Unreadable(format!(
        "prompt edits unreadable at {} ({detail}) — running the defaults instead would ignore \
         what a human edited. Fix or delete the file, or edit the prompt again from the view.",
        path.display()
    ))
}

fn wrote_nothing(path: &Path, detail: &str) -> Halt {
    Halt::Failed(format!(
        "couldn't write {} ({detail}) — the prompt edit was not kept",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of our own, cleaned up at the end. The real disk: this
    /// module *is* the filesystem.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("harness-prompts-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_book_yet_is_an_empty_book_not_an_error() {
        let dir = Dir::new("none");
        assert_eq!(
            PromptFile::new(&dir.0).load().expect("read"),
            PromptBook::default()
        );
    }

    #[test]
    fn a_book_round_trips_and_leaves_nothing_aside() {
        let dir = Dir::new("round");
        let store = PromptFile::new(&dir.0);
        let mut book = PromptBook::default();
        book.edit("agent-loop/code", "mine", "default", 7)
            .expect("edited");
        store.save(&book).expect("saved");
        assert_eq!(store.load().expect("read"), book);
        assert!(dir.0.join(".llocal/prompts.json").exists());
        assert!(!dir.0.join(".llocal/prompts.json.new").exists());
    }

    #[test]
    fn a_book_that_does_not_decode_is_unreadable_never_empty() {
        let dir = Dir::new("bad");
        std::fs::create_dir_all(dir.0.join(".llocal")).expect("dir");
        std::fs::write(dir.0.join(".llocal/prompts.json"), "{not json").expect("written");
        assert!(matches!(
            PromptFile::new(&dir.0).load(),
            Err(Halt::Unreadable(_))
        ));
    }
}
