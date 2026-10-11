//! The factories this machine knows — the port, not the database.
//!
//! One human can drive several factories from one page: each is a folder with
//! its own `.llocal/` and its own target repository. The register lives
//! outside every checkout (the view lists factories whichever it was started
//! in), so a switch is "start the view again in that folder".
//!
//! A factory being created keeps a record of where its creation stands
//! beside it — opaque here: the tool that creates it decides its shape.

use std::path::PathBuf;

use serde::Serialize;

use crate::domain::Outcome;

/// One factory, as the register keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Factory {
    /// Its name in the register, unique; what `--factory` takes.
    pub id: String,
    /// The name on the sign.
    pub name: String,
    /// `owner/name` of the target repository, or empty while the factory is
    /// not linked to a GitHub repository.
    pub slug: String,
    /// The folder the factory runs in: its checkout, its `.llocal/`.
    pub root: PathBuf,
    /// The branch the agents integrate into.
    pub integration_branch: String,
}

/// An id from a name: lowercase letters and digits, the rest folded into
/// single dashes (`Dnd Helper 2` → `dnd-helper-2`). Never empty.
#[must_use]
pub fn id_from(name: &str) -> String {
    let folded: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let id = folded
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if id.is_empty() {
        "factory".to_string()
    } else {
        id
    }
}

/// The register of factories.
pub trait Factories {
    /// Every factory, by name.
    ///
    /// # Errors
    ///
    /// The register could not be read.
    fn list(&self) -> Outcome<Vec<Factory>>;

    /// The factory called `id`, if the register knows it.
    ///
    /// # Errors
    ///
    /// The register could not be read.
    fn get(&self, id: &str) -> Outcome<Option<Factory>>;

    /// Makes sure the folder `root` is in the register and says what it is
    /// now: a known folder keeps its id and takes the name, repository and
    /// branch given; an unknown one gets a fresh id.
    ///
    /// # Errors
    ///
    /// The register could not be written.
    fn enroll(
        &self,
        root: &std::path::Path,
        name: &str,
        slug: &str,
        integration_branch: &str,
    ) -> Outcome<Factory>;

    /// The record of the factory `id`'s creation, as the tool creating it
    /// wrote it; `None` once it is created, or when it never was by a tool.
    ///
    /// # Errors
    ///
    /// The register could not be read.
    fn creation(&self, id: &str) -> Outcome<Option<String>>;

    /// Keeps `record` as the factory `id`'s creation, in place of the last.
    ///
    /// # Errors
    ///
    /// The register could not be written.
    fn keep_creation(&self, id: &str, record: &str) -> Outcome<()>;

    /// Forgets the factory `id`'s creation: it is created.
    ///
    /// # Errors
    ///
    /// The register could not be written.
    fn end_creation(&self, id: &str) -> Outcome<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_folds_everything_but_letters_and_digits_into_single_dashes() {
        assert_eq!(id_from("Dnd Helper 2"), "dnd-helper-2");
        assert_eq!(id_from("  --a__b--  "), "a-b");
        assert_eq!(id_from("Laucans/dnd_helper"), "laucans-dnd-helper");
    }

    #[test]
    fn an_id_is_never_empty() {
        assert_eq!(id_from(""), "factory");
        assert_eq!(id_from("---"), "factory");
        assert_eq!(id_from("é"), "factory");
    }
}
