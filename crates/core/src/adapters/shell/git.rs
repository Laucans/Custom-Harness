//! Le binaire `git`, emballé.
//!
//! Chaque appel **nomme le dépôt** (`-C <root>`) plutôt que de dépendre du
//! répertoire courant : le harness tourne depuis n'importe où, et un `git
//! status` qui répondrait sur un autre dépôt laisserait passer un arbre sale.
//!
//! Rien ici ne décide. Savoir qu'une branche existe est une réponse ; savoir
//! si c'est la bonne branche est une porte, et les portes vivent chez le
//! workflow.
//!
//! **Surface volontairement réduite** à ce que la boucle et son préflight
//! demandent. Le montage d'un workspace — `clone`, `fetch`, `reset --hard`,
//! `clean`, les branches à risque — est un autre métier, et il attend son
//! étape.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::adapters::shell::process::{self, Ran};
use crate::domain::Outcome;

/// Le binaire appelé.
const BINARY: &str = "git";

/// Ce que le harness demande à git, et rien de plus.
#[async_trait(?Send)]
pub trait Repo {
    /// La branche courante, ou le vide si `HEAD` est détaché.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn current_branch(&self) -> Outcome<String>;

    /// Le sha court de `HEAD`.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn head_sha(&self) -> Outcome<String>;

    /// Les fichiers que `status --porcelain` signale, un par ligne.
    ///
    /// Vide veut dire arbre propre.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn dirty_files(&self) -> Outcome<Vec<String>>;

    /// Cette branche existe-t-elle localement ?
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn has_branch(&self, name: &str) -> Outcome<bool>;

    /// `origin` porte-t-il cette branche ?
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn origin_has_branch(&self, name: &str) -> Outcome<bool>;

    /// L'URL d'un remote, ou le vide s'il n'y en a pas.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn remote_url(&self, remote: &str) -> Outcome<String>;
}

/// `git`, appelé sur un dépôt donné.
pub struct GitCli {
    root: PathBuf,
}

impl GitCli {
    /// `git`, sur ce dépôt.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Les arguments d'un appel, dépôt nommé en tête.
    ///
    /// Pur, et c'est là que vit l'invariant : **aucun appel ne part sans
    /// `-C <root>`**. Un test le vérifie, parce que l'oubli ne se voit pas —
    /// il répond correctement, sur le mauvais dépôt.
    fn argv(&self, args: &[&str]) -> Vec<String> {
        let mut out = vec!["-C".to_string(), self.root.display().to_string()];
        out.extend(args.iter().map(ToString::to_string));
        out
    }

    async fn git(&self, args: &[&str]) -> Outcome<Ran> {
        process::run(BINARY, &self.argv(args), &self.root).await
    }
}

#[async_trait(?Send)]
impl Repo for GitCli {
    async fn current_branch(&self) -> Outcome<String> {
        // Le vide est une réponse : un `HEAD` détaché n'a pas de branche, et
        // `symbolic-ref` sort alors en non nul.
        Ok(self
            .git(&["symbolic-ref", "--short", "HEAD"])
            .await?
            .out()
            .to_string())
    }

    async fn head_sha(&self) -> Outcome<String> {
        Ok(self
            .git(&["rev-parse", "--short", "HEAD"])
            .await?
            .out()
            .to_string())
    }

    async fn dirty_files(&self) -> Outcome<Vec<String>> {
        Ok(self.git(&["status", "--porcelain"]).await?.lines())
    }

    async fn has_branch(&self, name: &str) -> Outcome<bool> {
        Ok(self
            .git(&["rev-parse", "--verify", "--quiet", name])
            .await?
            .ok())
    }

    async fn origin_has_branch(&self, name: &str) -> Outcome<bool> {
        Ok(self
            .git(&["ls-remote", "--exit-code", "--heads", "origin", name])
            .await?
            .ok())
    }

    async fn remote_url(&self, remote: &str) -> Outcome<String> {
        Ok(self
            .git(&["remote", "get-url", remote])
            .await?
            .out()
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git() -> GitCli {
        GitCli::new(Path::new("/tmp/le-clone"))
    }

    #[test]
    fn every_call_names_the_repository_first() {
        // L'oubli qu'on rend impossible : `git status` répondrait
        // correctement, sur le dépôt du répertoire courant.
        let args = git().argv(&["status", "--porcelain"]);
        assert_eq!(args[0], "-C");
        assert_eq!(args[1], "/tmp/le-clone");
        assert_eq!(args[2], "status");
    }

    #[test]
    fn the_verb_and_its_flags_keep_their_order_after_the_repository() {
        let args = git().argv(&["rev-parse", "--verify", "--quiet", "main"]);
        assert_eq!(
            args,
            vec![
                "-C".to_string(),
                "/tmp/le-clone".to_string(),
                "rev-parse".to_string(),
                "--verify".to_string(),
                "--quiet".to_string(),
                "main".to_string(),
            ]
        );
    }

    #[test]
    fn a_call_with_no_arguments_still_names_the_repository() {
        assert_eq!(git().argv(&[]).len(), 2);
    }
}
