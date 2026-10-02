//! Le disque, emballé — pour que ce qui supprime soit testable.
//!
//! Un port pour cinq opérations de système de fichiers peut sembler de trop :
//! `std::fs` est déjà une bibliothèque. La raison est ailleurs. Le seul module
//! qui s'en sert est [`crate::execution::provisioning`], c'est-à-dire le seul
//! du paquet qui **supprime des centaines de mégaoctets**, et ses trois règles
//! — rien n'est écrasé sans le dire, rien n'est supprimé sans le dire, un
//! dry-run ne clone pas — sont précisément celles qu'il faut pouvoir exercer
//! sans mettre un vrai dossier en jeu.
//!
//! Un faux disque rend « ce test prouve qu'on ne supprime pas » vérifiable.
//! Sans lui, le prouver demanderait de créer un clone et d'espérer.

use std::path::Path;

use crate::domain::{Halt, Outcome};

/// Ce que le montage d'un workspace demande au disque.
pub trait Disk {
    /// Ce chemin existe-t-il ?
    fn exists(&self, path: &Path) -> bool;

    /// Crée ce dossier et ses parents. Déjà là n'est pas une erreur.
    ///
    /// # Errors
    /// Si le dossier n'a pas pu être créé.
    fn create_dir_all(&self, path: &Path) -> Outcome<()>;

    /// Supprime ce dossier et tout ce qu'il contient.
    ///
    /// # Errors
    /// Si la suppression a échoué. L'absence du dossier n'en est pas une : la
    /// fin voulue est atteinte.
    fn remove_dir_all(&self, path: &Path) -> Outcome<()>;

    /// Les noms des sous-dossiers, triés. Vide si le chemin n'est pas un
    /// dossier — ce qui sert à lister les workspaces gardés dans un message.
    fn dir_names(&self, path: &Path) -> Vec<String>;
}

/** Le vrai disque. */
pub struct RealDisk;

impl Disk for RealDisk {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, path: &Path) -> Outcome<()> {
        std::fs::create_dir_all(path)
            .map_err(|e| Halt::Failed(format!("impossible de créer {} : {e}", path.display())))
    }

    fn remove_dir_all(&self, path: &Path) -> Outcome<()> {
        match std::fs::remove_dir_all(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Halt::Failed(format!(
                "impossible de supprimer {} : {e}",
                path.display()
            ))),
        }
    }

    fn dir_names(&self, path: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(path) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_something_that_is_already_gone_is_not_a_failure() {
        // La fin voulue est atteinte, et le démontage ne doit pas requalifier
        // un run réussi en panne pour ça.
        assert!(
            RealDisk
                .remove_dir_all(Path::new("/tmp/un-dossier-qui-nexiste-pas-ici"))
                .is_ok()
        );
    }

    #[test]
    fn listing_something_that_is_not_a_directory_gives_no_names() {
        assert!(
            RealDisk
                .dir_names(Path::new("/tmp/pas-un-dossier-du-tout"))
                .is_empty()
        );
    }
}
