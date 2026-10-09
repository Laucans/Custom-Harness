//! The human's pins on disk: `.llocal/annotations.json`, beside the stores.
//!
//! A file at the yard's root is a store to the janitor: never swept.

use std::fs;
use std::path::{Path, PathBuf};

use crate::ports::Notebook;

/// The file, under the yard.
const FILE: &str = "annotations.json";

/// `.llocal/annotations.json` of one checkout.
pub struct FsNotebook {
    root: PathBuf,
}

impl FsNotebook {
    /// The notebook under `state_root` — `<state_root>/.llocal`.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            root: state_root.join(".llocal"),
        }
    }
}

impl Notebook for FsNotebook {
    fn read(&self) -> Option<String> {
        fs::read_to_string(self.root.join(FILE)).ok()
    }

    fn write(&self, json: &str) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        // Written aside then renamed: a crash mid-write leaves the old pins.
        let path = self.root.join(FILE);
        let aside = self.root.join(format!("{FILE}.new"));
        fs::write(&aside, json).map_err(|e| format!("{}: {e}", aside.display()))?;
        fs::rename(&aside, &path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_round_trip_beside_the_stores() {
        let dir = std::env::temp_dir().join(format!("harness-notes-{}", std::process::id()));
        let book = FsNotebook::new(&dir);
        assert!(book.read().is_none());
        book.write("{\"b\":[]}").expect("written");
        assert_eq!(book.read().as_deref(), Some("{\"b\":[]}"));
        assert!(dir.join(".llocal/annotations.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
