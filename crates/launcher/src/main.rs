#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Même raison qu'à la racine de `harness-core` : la décision n°3 choisit
// `?Send` partout, et `clippy::future_not_send` présuppose le contraire.
#![allow(clippy::future_not_send)]

//! Le point d'entrée : lire les arguments, trouver le dépôt, faire tourner.
//!
//! Le code de sortie **est un contrat**. Un ordonnanceur extérieur le lit, et
//! les quatre valeurs sont celles du pipeline Python, inchangées par la
//! migration : `0` tout va bien, `1` un arrêt volontaire ou un magasin
//! illisible, `2` un échec, `3` un quota épuisé. Les déplacer serait un
//! changement de contrat déguisé en refactoring.

mod cli;
mod dev_loop;
mod sink;
mod spending;
mod tooling;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use harness_core::domain::{Severity, Verdict};

/// Fait tourner la boucle, et rend le code que l'ordonnanceur lit.
///
/// `current_thread` : le harness pilote une session à la fois, et la décision
/// n°3 choisit `?Send` partout — un ordonnanceur multi-thread n'aurait rien à
/// ordonnancer.
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = cli::Cli::parse();
    let here = match repo_root() {
        Ok(path) => path,
        Err(why) => {
            eprintln!("{why}");
            return ExitCode::from(2);
        }
    };
    match dev_loop::run(&args, &here).await {
        Ok(ran) => {
            if let Verdict::NothingLeft(why) = &ran.verdict {
                println!("{why}");
            }
            println!("journal : {}", ran.log.display());
            ExitCode::SUCCESS
        }
        Err(halt) => {
            // Le préfixe et le niveau sont ceux que l'ordonnanceur lisait déjà.
            let line = format!("{}: {}", halt.prefix(), halt.reason());
            match halt.severity() {
                Severity::Info => println!("{line}"),
                Severity::Warn | Severity::Error => eprintln!("{line}"),
            }
            ExitCode::from(u8::try_from(halt.exit_code()).unwrap_or(2))
        }
    }
}

/// Le dépôt d'où le run est lancé.
///
/// En remontant depuis le répertoire courant, **sans sous-processus** : le
/// Python lançait un `git rev-parse` à l'import, donc charger n'importe quel
/// module — un hook, un `--help`, une collecte de tests — en lançait un, et
/// levait une exception nue en dehors d'un checkout.
fn repo_root() -> Result<PathBuf, String> {
    let here = std::env::current_dir()
        .map_err(|e| format!("le répertoire courant ne se lit pas : {e}"))?;
    walk_up(&here).ok_or_else(|| {
        format!(
            "aucun dépôt git au-dessus de {} — lancez le harness depuis un \
             checkout",
            here.display()
        )
    })
}

/// Le premier ancêtre qui porte un `.git`, celui-ci compris.
fn walk_up(from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .find(|parent| parent.join(".git").exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::Halt;

    #[test]
    fn the_exit_codes_are_the_frozen_contract() {
        // Un ordonnanceur extérieur les lit : les déplacer serait un changement
        // de contrat déguisé en refactoring.
        assert_eq!(Halt::Halted(String::new()).exit_code(), 1);
        assert_eq!(Halt::Unreadable(String::new()).exit_code(), 1);
        assert_eq!(Halt::Failed(String::new()).exit_code(), 2);
        assert_eq!(Halt::Quota(String::new()).exit_code(), 3);
        // Et chacun tient dans le `u8` que rend un processus.
        for halt in [
            Halt::Halted(String::new()),
            Halt::Unreadable(String::new()),
            Halt::Failed(String::new()),
            Halt::Quota(String::new()),
        ] {
            assert!(u8::try_from(halt.exit_code()).is_ok());
        }
    }

    #[test]
    fn the_repository_is_found_by_walking_up_without_a_subprocess() {
        // Ce dépôt-ci : le test tourne dedans, donc la réponse est connue.
        let found = walk_up(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("un dépôt");
        assert!(found.join(".git").exists());
    }

    #[test]
    fn outside_a_checkout_the_answer_is_none_rather_than_a_bare_exception() {
        assert!(walk_up(Path::new("/")).is_none());
    }
}
