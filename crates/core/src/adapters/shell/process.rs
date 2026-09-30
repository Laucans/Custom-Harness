//! Le seul endroit du paquet qui lance un sous-processus.
//!
//! Un seul, pour que « une porte décide, elle n'appelle jamais `git`/`gh`
//! elle-même » ait un endroit où être vrai. `git.rs` et `github.rs` sont des
//! traducteurs au-dessus de ce module : ils construisent des arguments et
//! lisent des sorties, ils ne parlent pas au système.

use std::path::Path;

use crate::domain::{Halt, Outcome};

/// Ce qu'un processus a laissé derrière lui.
#[derive(Debug, Clone)]
pub struct Ran {
    /// Le code de sortie. `None` quand un signal a tué le processus.
    pub code: Option<i32>,
    /// La sortie standard, telle quelle.
    pub stdout: String,
    /// La sortie d'erreur, telle quelle.
    pub stderr: String,
}

impl Ran {
    /// Vrai si le processus a rendu zéro.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// La sortie standard, sans les blancs de bord.
    #[must_use]
    pub fn out(&self) -> &str {
        self.stdout.trim()
    }

    /// Les lignes non vides de la sortie standard.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.stdout
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty())
            .map(ToString::to_string)
            .collect()
    }

    /// La dernière ligne utile de stderr — le bout qui diagnostique.
    ///
    /// C'est la seule ligne qu'un humain doit lire quand deux passes payées
    /// ont échoué à poster, donc elle ne doit jamais être vide ni ressembler
    /// à une liste Python.
    #[must_use]
    pub fn why(&self) -> String {
        last_line(&self.stderr)
    }
}

/// La dernière ligne non vide d'un texte, ou une phrase qui le dit.
#[must_use]
pub fn last_line(text: &str) -> String {
    text.lines()
        .rfind(|line| !line.trim().is_empty())
        .map_or_else(
            || "(rien sur stderr)".to_string(),
            |line| line.trim().to_string(),
        )
}

/// Lance `binary` avec ces arguments, depuis `cwd`.
///
/// **Un code de sortie non nul n'est pas une erreur ici.** `git rev-parse
/// --verify` répond « cette branche n'existe pas » par un code non nul, et
/// c'est une réponse, pas une panne. Seul un binaire qu'on n'a pas pu lancer
/// rend `Err` — l'appelant décide de ce que le code veut dire.
///
/// # Errors
///
/// [`Halt::Failed`] si le processus n'a pas pu être lancé du tout : binaire
/// absent du `PATH`, ou répertoire de travail inexistant.
pub async fn run(binary: &str, args: &[String], cwd: &Path) -> Outcome<Ran> {
    let out = tokio::process::Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| {
            Halt::Failed(format!(
                "{binary} n'a pas pu être lancé depuis {} : {e}",
                cwd.display()
            ))
        })?;
    Ok(Ran {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(stdout: &str, stderr: &str, code: i32) -> Ran {
        Ran {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn empty_and_whitespace_lines_are_dropped() {
        let out = ran("a\n\n  \nb\n", "", 0);
        assert_eq!(out.lines(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn no_output_at_all_yields_no_lines_rather_than_one_empty_one() {
        assert!(ran("", "", 0).lines().is_empty());
        assert!(ran("\n\n", "", 0).lines().is_empty());
    }

    #[test]
    fn why_takes_the_last_useful_line_of_stderr() {
        let out = ran("", "warning: blah\nfatal: not a git repository\n", 128);
        assert_eq!(out.why(), "fatal: not a git repository");
    }

    #[test]
    fn an_empty_stderr_says_so_instead_of_being_blank() {
        // Le mode de panne que ça évite : une ligne de diagnostic vide, seule
        // chose qu'un humain avait à lire après deux passes payées.
        assert_eq!(ran("", "", 1).why(), "(rien sur stderr)");
        assert_eq!(ran("", "   \n\n", 1).why(), "(rien sur stderr)");
    }

    #[test]
    fn ok_is_exactly_zero_not_merely_absence_of_error() {
        assert!(ran("", "", 0).ok());
        assert!(!ran("", "", 1).ok());
        let killed = Ran {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(!killed.ok());
    }

    #[tokio::test]
    async fn a_missing_binary_is_a_failure_not_a_non_zero_code() {
        let err = run("un-binaire-qui-nexiste-pas-ici", &[], Path::new("."))
            .await
            .expect_err("doit échouer");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[tokio::test]
    async fn a_non_zero_exit_comes_back_as_data() {
        // `false` sort avec 1 : c'est une réponse, pas une panne.
        let out = run("false", &[], Path::new(".")).await.expect("lancé");
        assert!(!out.ok());
        assert_eq!(out.code, Some(1));
    }
}
