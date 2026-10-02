//! Où les lignes de journal tombent : la console, et le fichier du run.
//!
//! Les deux, toujours. **Le fichier garde tout** quel que soit le niveau
//! demandé — c'est lui qu'on relit après coup, et un `--quiet` qui aurait
//! tronqué la seule trace d'un run de nuit serait une fausse économie. Le
//! niveau ne concerne que la console, et [`Logbook`] le tient déjà.

use std::cell::RefCell;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use harness_core::traces::Sink;

/// La console, et le fichier du run.
pub struct Both {
    path: PathBuf,
    file: RefCell<Option<std::fs::File>>,
}

impl Both {
    /// Le journal de ce run, écrit ici en plus de la console.
    ///
    /// Le fichier est ouvert maintenant plutôt qu'à la première ligne : un
    /// dossier de journal qu'on ne peut pas créer vaut d'être su avant qu'une
    /// session soit payée, pas après.
    ///
    /// # Errors
    ///
    /// Si le dossier ou le fichier n'ont pas pu être créés.
    pub fn new(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file: RefCell::new(Some(file)),
        })
    }

    /// Où ce journal est écrit.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Sink for Both {
    fn emit(&self, line: &str) {
        println!("{line}");
        // Un fichier qui ne s'écrit plus ne doit pas tuer un run en vol, et ne
        // doit pas non plus se plaindre à chaque ligne : on le lâche une fois,
        // en le disant une fois.
        let mut held = self.file.borrow_mut();
        if let Some(file) = held.as_mut()
            && writeln!(file, "{line}").is_err()
        {
            *held = None;
            eprintln!(
                "attention : le journal {} ne s'écrit plus",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("harness-sink-{tag}"));
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
    fn the_log_directory_is_created_rather_than_demanded() {
        let dir = Dir::new("creates");
        let path = dir.0.join("20261002/run.log");
        let sink = Both::new(&path).expect("un journal");
        sink.emit("une ligne");
        assert_eq!(std::fs::read_to_string(&path).expect("relu"), "une ligne\n");
    }

    #[test]
    fn a_second_run_appends_rather_than_truncating() {
        let dir = Dir::new("appends");
        let path = dir.0.join("run.log");
        Both::new(&path).expect("un journal").emit("premier");
        Both::new(&path).expect("un journal").emit("second");
        let text = std::fs::read_to_string(&path).expect("relu");
        assert_eq!(text, "premier\nsecond\n");
    }
}
