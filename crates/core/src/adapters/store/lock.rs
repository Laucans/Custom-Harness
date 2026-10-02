//! Un verrou de fichiers : au plus un porteur d'un nom à la fois.
//!
//! Garde `pr_review` et `refinement` d'une double exécution — deux hooks
//! partis sur la même PR, ou deux rounds de raffinage sur la même issue,
//! posteraient chacun en double. Un `mkdir` et non un fichier témoin : la
//! création d'un répertoire est atomique sur tout système de fichiers qui
//! nous concerne, donc deux processus partis en même temps ne peuvent pas
//! l'obtenir tous les deux.

use std::path::Path;

use crate::domain::{Halt, Outcome};

/// Ce qu'un verrou demande au disque.
pub trait Locks {
    /// Tente d'acquérir le verrou `name` sous `dir`. Vrai si c'est cet appel
    /// qui l'a obtenu, faux si quelqu'un d'autre le tient déjà.
    ///
    /// # Errors
    /// Si `dir` n'a pas pu être créé.
    fn acquire(&self, dir: &Path, name: &str) -> Outcome<bool>;

    /// Relâche le verrou. Silencieux s'il n'existe déjà plus — le relâcher
    /// deux fois ne doit pas être une erreur.
    fn release(&self, dir: &Path, name: &str);
}

/// Un verrou qui est réellement un dossier sur le disque.
pub struct DirLocks;

impl DirLocks {
    fn path(dir: &Path, name: &str) -> std::path::PathBuf {
        dir.join(format!(".lock-{name}"))
    }
}

impl Locks for DirLocks {
    fn acquire(&self, dir: &Path, name: &str) -> Outcome<bool> {
        std::fs::create_dir_all(dir).map_err(|e| {
            Halt::Failed(format!(
                "impossible de créer {} pour y poser un verrou : {e}",
                dir.display()
            ))
        })?;
        match std::fs::create_dir(Self::path(dir, name)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(Halt::Failed(format!(
                "impossible de poser le verrou {name} dans {} : {e}",
                dir.display()
            ))),
        }
    }

    fn release(&self, dir: &Path, name: &str) {
        let _ = std::fs::remove_dir(Self::path(dir, name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("harness-lock-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_fresh_name_is_acquired() {
        let dir = Dir::new("fresh");
        assert!(DirLocks.acquire(&dir.0, "32").expect("acquis"));
    }

    #[test]
    fn a_name_already_held_is_refused_to_a_second_claimant() {
        // Deux hooks partis sur la même PR : le second ne doit pas aussi
        // obtenir le verrou.
        let dir = Dir::new("held");
        assert!(
            DirLocks
                .acquire(&dir.0, "32")
                .expect("le premier l'obtient")
        );
        assert!(
            !DirLocks
                .acquire(&dir.0, "32")
                .expect("le second ne l'obtient pas")
        );
    }

    #[test]
    fn releasing_frees_the_name_for_the_next_claimant() {
        let dir = Dir::new("released");
        assert!(DirLocks.acquire(&dir.0, "32").expect("acquis"));
        DirLocks.release(&dir.0, "32");
        assert!(DirLocks.acquire(&dir.0, "32").expect("réacquis"));
    }

    #[test]
    fn releasing_twice_is_not_an_error() {
        let dir = Dir::new("double-release");
        assert!(DirLocks.acquire(&dir.0, "32").expect("acquis"));
        DirLocks.release(&dir.0, "32");
        DirLocks.release(&dir.0, "32");
    }

    #[test]
    fn two_different_names_do_not_contend() {
        let dir = Dir::new("two-names");
        assert!(DirLocks.acquire(&dir.0, "32").expect("le premier"));
        assert!(
            DirLocks
                .acquire(&dir.0, "99")
                .expect("le second, sans rapport")
        );
    }
}
