//! The human's prompt edits on disk, beside the stores.
//!
//! - `.llocal/prompts.json` — the book, what runs. Written only by the
//!   harness, when the watch starts and applies the drafts.
//! - `.llocal/prompts.drafts.json` — the drafts, what waits for that start.
//!   Written by the view.
//! - `.llocal/prompts.shelf.json` — the drafts a human deactivated. Written
//!   and read by the view only: the harness's start never takes them.
//! - `prompt-texts.json`, in a run's own folder — the text of every prompt
//!   that run could send, as it stood when the run started: what a dry-run
//!   reads to replay that run with another version.
//!
//! Every file is written aside then renamed: a reader sees the old content or
//! the new, never half of either. A file at the yard's root is a store to the
//! janitor: never swept.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use harness_core::domain::prompt_book::{Drafts, PromptBook, Shelf};
use harness_core::domain::{Halt, Outcome};
use harness_core::ports::store::prompts::{DraftStore, PromptStore, ShelfStore};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The book, under `.llocal/`.
pub const FILE: &str = "prompts.json";

/// The drafts, under `.llocal/`.
pub const DRAFTS: &str = "prompts.drafts.json";

/// The deactivated drafts, under `.llocal/`.
pub const SHELF: &str = "prompts.shelf.json";

/// What a run could send, in its own folder.
pub const USED: &str = "prompt-texts.json";

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
}

impl PromptStore for PromptFile {
    fn load(&self) -> Outcome<PromptBook> {
        read_json(&self.dir.join(FILE), "prompt book")
    }

    fn save(&self, book: &PromptBook) -> Outcome<()> {
        write_json(&self.dir, FILE, book)
    }
}

/// `.llocal/prompts.drafts.json` of one checkout.
pub struct DraftFile {
    dir: PathBuf,
}

impl DraftFile {
    /// The drafts of the checkout whose state lives under `state_root`.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            dir: state_root.join(".llocal"),
        }
    }
}

impl DraftStore for DraftFile {
    fn load(&self) -> Outcome<Drafts> {
        read_json(&self.dir.join(DRAFTS), "prompt drafts")
    }

    fn save(&self, drafts: &Drafts) -> Outcome<()> {
        if drafts.is_empty() {
            return remove(&self.dir.join(DRAFTS));
        }
        write_json(&self.dir, DRAFTS, drafts)
    }

    /// Renamed out of the way first, then read: a draft the view saves in
    /// between lands in a fresh file and waits for the next start.
    fn take(&self) -> Outcome<Drafts> {
        let path = self.dir.join(DRAFTS);
        let taken = self.dir.join(format!("{DRAFTS}.taken"));
        match std::fs::rename(&path, &taken) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Drafts::default()),
            Err(e) => return Err(unreadable(&path, "prompt drafts", &e.to_string())),
        }
        match read_json(&taken, "prompt drafts") {
            Ok(drafts) => {
                remove(&taken)?;
                Ok(drafts)
            }
            Err(halt) => {
                // Put back where they were, so the human still sees them.
                let _ = std::fs::rename(&taken, &path);
                Err(halt)
            }
        }
    }
}

/// `.llocal/prompts.shelf.json` of one checkout.
pub struct ShelfFile {
    dir: PathBuf,
}

impl ShelfFile {
    /// The shelf of the checkout whose state lives under `state_root`.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            dir: state_root.join(".llocal"),
        }
    }
}

impl ShelfStore for ShelfFile {
    fn load(&self) -> Outcome<Shelf> {
        read_json(&self.dir.join(SHELF), "deactivated prompt drafts")
    }

    fn save(&self, deactivated: &Shelf) -> Outcome<()> {
        if deactivated.is_empty() {
            return remove(&self.dir.join(SHELF));
        }
        write_json(&self.dir, SHELF, deactivated)
    }
}

/// Records, in a run's folder, the text of every prompt it could send.
///
/// # Errors
///
/// [`Halt::Failed`] if the file could not be written.
pub fn record_used(run_dir: &Path, texts: &BTreeMap<String, String>) -> Outcome<()> {
    write_json(run_dir, USED, texts)
}

/// The texts a run recorded, `None` for a run older than the record.
///
/// # Errors
///
/// [`Halt::Unreadable`] if the record exists and cannot be decoded.
pub fn read_used(run_dir: &Path) -> Outcome<Option<BTreeMap<String, String>>> {
    let path = run_dir.join(USED);
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path, "prompt record").map(Some)
}

/// A JSON file, or `T::default()` when it does not exist or is blank.
fn read_json<T: DeserializeOwned + Default>(path: &Path, what: &str) -> Outcome<T> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => return Err(unreadable(path, what, &e.to_string())),
    };
    if text.trim().is_empty() {
        return Ok(T::default());
    }
    serde_json::from_str(&text).map_err(|e| unreadable(path, what, &e.to_string()))
}

/// Written aside then renamed.
fn write_json<T: Serialize>(dir: &Path, name: &str, value: &T) -> Outcome<()> {
    std::fs::create_dir_all(dir).map_err(|e| wrote_nothing(dir, &e.to_string()))?;
    let path = dir.join(name);
    let text =
        serde_json::to_string_pretty(value).map_err(|e| wrote_nothing(&path, &e.to_string()))?;
    let aside = dir.join(format!("{name}.new"));
    std::fs::write(&aside, text).map_err(|e| wrote_nothing(&aside, &e.to_string()))?;
    std::fs::rename(&aside, &path).map_err(|e| wrote_nothing(&path, &e.to_string()))
}

fn remove(path: &Path) -> Outcome<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(wrote_nothing(path, &e.to_string())),
    }
}

fn unreadable(path: &Path, what: &str, detail: &str) -> Halt {
    Halt::Unreadable(format!(
        "{what} unreadable at {} ({detail}) — running the defaults instead would ignore \
         what a human edited. Fix or delete the file, or edit the prompt again from the view.",
        path.display()
    ))
}

fn wrote_nothing(path: &Path, detail: &str) -> Halt {
    Halt::Failed(format!(
        "couldn't write {} ({detail}) — the prompt change was not kept",
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

    fn drafted(book: &PromptBook) -> Drafts {
        let mut drafts = Drafts::default();
        drafts
            .propose(book, "agent-loop/code", "default", Some("mine"), false, 7)
            .expect("proposed");
        drafts
    }

    #[test]
    fn nothing_saved_yet_is_empty_not_an_error() {
        let dir = Dir::new("none");
        assert_eq!(
            PromptFile::new(&dir.0).load().expect("read"),
            PromptBook::default()
        );
        assert!(
            DraftFile::new(&dir.0).take().expect("taken").is_empty(),
            "expected nothing"
        );
    }

    #[test]
    fn a_book_round_trips_and_leaves_nothing_aside() {
        let dir = Dir::new("round");
        let store = PromptFile::new(&dir.0);
        let mut book = PromptBook::default();
        book.apply(&drafted(&book), |_| Some("default".to_string()), 8);
        store.save(&book).expect("saved");
        assert_eq!(store.load().expect("read"), book);
        assert!(dir.0.join(".llocal/prompts.json").exists());
        assert!(!dir.0.join(".llocal/prompts.json.new").exists());
    }

    #[test]
    fn a_shelf_round_trips_and_outlives_the_drafts_being_taken() {
        let dir = Dir::new("shelf");
        let store = ShelfFile::new(&dir.0);
        assert!(store.load().expect("read").is_empty(), "expected nothing");
        let mut drafts = drafted(&PromptBook::default());
        let mut shelf = Shelf::default();
        drafts.deactivate(&mut shelf, "agent-loop/code");
        store.save(&shelf).expect("saved");
        DraftFile::new(&dir.0)
            .save(&drafted(&PromptBook::default()))
            .expect("saved");
        DraftFile::new(&dir.0).take().expect("taken");
        assert_eq!(store.load().expect("read"), shelf);
        store.save(&Shelf::default()).expect("saved");
        assert!(!dir.0.join(".llocal").join(SHELF).exists());
    }

    #[test]
    fn taken_drafts_are_gone_and_saving_none_removes_the_file() {
        let dir = Dir::new("take");
        let store = DraftFile::new(&dir.0);
        let drafts = drafted(&PromptBook::default());
        store.save(&drafts).expect("saved");
        assert_eq!(store.load().expect("read"), drafts);
        assert_eq!(store.take().expect("taken"), drafts);
        assert!(store.load().expect("read").is_empty(), "expected nothing");
        store.save(&drafts).expect("saved");
        store.save(&Drafts::default()).expect("saved");
        assert!(!dir.0.join(".llocal").join(DRAFTS).exists());
    }

    #[test]
    fn a_store_that_does_not_decode_is_unreadable_never_empty() {
        let dir = Dir::new("bad");
        std::fs::create_dir_all(dir.0.join(".llocal")).expect("dir");
        std::fs::write(dir.0.join(".llocal/prompts.json"), "{not json").expect("written");
        std::fs::write(dir.0.join(".llocal").join(DRAFTS), "{not json").expect("written");
        assert!(matches!(
            PromptFile::new(&dir.0).load(),
            Err(Halt::Unreadable(_))
        ));
        assert!(matches!(
            DraftFile::new(&dir.0).take(),
            Err(Halt::Unreadable(_))
        ));
        assert!(
            dir.0.join(".llocal").join(DRAFTS).exists(),
            "put back for the human to see"
        );
    }

    #[test]
    fn a_run_s_texts_round_trip_and_an_older_run_has_none() {
        let dir = Dir::new("used");
        assert_eq!(read_used(&dir.0).expect("read"), None);
        let texts = BTreeMap::from([("split/slice".to_string(), "Slice {num}".to_string())]);
        record_used(&dir.0, &texts).expect("recorded");
        assert_eq!(read_used(&dir.0).expect("read"), Some(texts));
    }
}
