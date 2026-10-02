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
//! # Lire et écrire ne rendent pas la même chose
//!
//! Les lectures rendent une **réponse** déjà interprétée — une branche, des
//! lignes, un booléen. Les verbes du montage rendent le [`Ran`] brut, parce
//! qu'un `reset --hard` qui échoue doit pouvoir dire *pourquoi* : la dernière
//! ligne de son stderr est tout ce qu'un humain aura à lire. Les traduire en
//! `Outcome<()>` perdrait exactement ça.

use std::path::{Path, PathBuf};
use std::rc::Rc;

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

    /// La branche que pointe `origin/HEAD`, ou le vide.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn default_branch(&self) -> Outcome<String>;

    /// Les branches locales, par nom.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn local_branches(&self) -> Outcome<Vec<String>>;

    /// Les entrées de `stash list`.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn stashes(&self) -> Outcome<Vec<String>>;

    /// Les commits locaux qu'aucun remote ne porte, un par ligne.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn unpushed(&self) -> Outcome<Vec<String>>;

    /// Les branches locales portant un patch qu'`upstream` n'a pas.
    ///
    /// **Par patch et non par sha**, et ce n'est pas un détail : le dépôt cible
    /// fusionne en rebase, donc les commits d'une PR fusionnée n'existent plus
    /// nulle part sous leur sha d'origine. Les compter comme du travail à
    /// sauver bloquait un workspace permanent à chaque run.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn branches_at_risk(&self, upstream: &str) -> Outcome<Vec<String>>;

    // --- les verbes du montage : ils rendent ce que `git` a dit ------------

    /// Clone `url` dans `name`, sous le dépôt que ce client nomme.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn clone_repo(&self, url: &str, name: &str) -> Outcome<Ran>;

    /// `fetch --prune` : ajoute des références distantes, élague les mortes.
    ///
    /// **Ne détruit rien de local**, et c'est ce qui permet de l'appeler avant
    /// la garde plutôt qu'après.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn fetch(&self) -> Outcome<Ran>;

    /// Bascule sur cette branche. `force` écrase les fichiers modifiés.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn checkout(&self, branch: &str, force: bool) -> Outcome<Ran>;

    /// `reset --hard <reference>`.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn reset_hard(&self, reference: &str) -> Outcome<Ran>;

    /// `clean -fd` : supprime ce que git ne suit pas, fichiers ignorés exclus.
    ///
    /// Les ignorés **restent**, et c'est ce qui laisse un workspace permanent
    /// garder ses dépendances installées.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn clean(&self) -> Outcome<Ran>;

    /// Supprime cette branche locale, même non fusionnée.
    ///
    /// # Errors
    /// Si `git` n'a pas pu être lancé.
    async fn delete_branch(&self, name: &str) -> Outcome<Ran>;
}

/// Ouvre un [`Repo`] sur un dépôt donné.
///
/// Le montage d'un workspace parle à **trois** dépôts : celui d'où le run est
/// lancé, le dossier parent où le clone tombe, et le clone lui-même. Un
/// `Repo` est lié à une racine, donc il faut de quoi en ouvrir un ailleurs —
/// et une seule couture de test pour les trois.
pub trait Repos {
    /// Un `git` sur ce dépôt.
    fn at(&self, root: &Path) -> Rc<dyn Repo>;
}

/// La fabrique qui rend de vrais `git`.
pub struct GitRepos;

impl Repos for GitRepos {
    fn at(&self, root: &Path) -> Rc<dyn Repo> {
        Rc::new(GitCli::new(root))
    }
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

    async fn default_branch(&self) -> Outcome<String> {
        // `origin/main` -> `main`. Le vide quand `origin/HEAD` n'est pas posé,
        // ce qui arrive sur un clone partiel : une réponse, pas une panne.
        let found = self
            .git(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
            .await?;
        Ok(found
            .out()
            .strip_prefix("origin/")
            .unwrap_or_default()
            .to_string())
    }

    async fn local_branches(&self) -> Outcome<Vec<String>> {
        Ok(self
            .git(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .await?
            .lines())
    }

    async fn stashes(&self) -> Outcome<Vec<String>> {
        Ok(self.git(&["stash", "list"]).await?.lines())
    }

    async fn unpushed(&self) -> Outcome<Vec<String>> {
        Ok(self
            .git(&["log", "--branches", "--not", "--remotes", "--oneline"])
            .await?
            .lines())
    }

    async fn branches_at_risk(&self, upstream: &str) -> Outcome<Vec<String>> {
        let mut at_risk = Vec::new();
        for branch in self.local_branches().await? {
            // `cherry` compare par patch-id : une ligne `+` est un commit
            // qu'`upstream` n'a pas, même réécrit par un rebase.
            let said = self.git(&["cherry", upstream, &branch]).await?;
            if said.lines().iter().any(|line| line.starts_with('+')) {
                at_risk.push(branch);
            }
        }
        Ok(at_risk)
    }

    async fn clone_repo(&self, url: &str, name: &str) -> Outcome<Ran> {
        self.git(&["clone", url, name]).await
    }

    async fn fetch(&self) -> Outcome<Ran> {
        self.git(&["fetch", "--prune", "origin"]).await
    }

    async fn checkout(&self, branch: &str, force: bool) -> Outcome<Ran> {
        // `--force` seulement quand on remet à zéro : `git` refuse de basculer
        // tant que des fichiers modifiés seraient écrasés, donc un
        // `--force-reset` échouait sur exactement le workspace sale qu'il
        // existe pour écraser — en se plaignant d'une branche absente, qui
        // était là.
        if force {
            self.git(&["checkout", "--force", branch]).await
        } else {
            self.git(&["checkout", branch]).await
        }
    }

    async fn reset_hard(&self, reference: &str) -> Outcome<Ran> {
        self.git(&["reset", "--hard", reference]).await
    }

    async fn clean(&self) -> Outcome<Ran> {
        self.git(&["clean", "-fd"]).await
    }

    async fn delete_branch(&self, name: &str) -> Outcome<Ran> {
        self.git(&["branch", "-D", name]).await
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

    #[test]
    fn clean_leaves_ignored_files_alone() {
        // Sans ça, un workspace permanent perdrait `node_modules` à chaque run
        // et chaque commande de vérification d'un stage échouerait. `-x`
        // supprimerait les ignorés ; il ne doit jamais apparaître ici.
        let args = git().argv(&["clean", "-fd"]);
        assert!(!args.iter().any(|a| a.contains('x')));
    }

    #[test]
    fn a_reset_checkout_forces_and_a_plain_one_does_not() {
        // Deux formes d'argv, et le test les fige : `--force` sur un checkout
        // qui ne remet pas à zéro écraserait un workspace qu'on voulait garder.
        assert!(
            !git()
                .argv(&["checkout", "main_agent"])
                .contains(&"--force".to_string())
        );
        assert!(
            git()
                .argv(&["checkout", "--force", "main_agent"])
                .contains(&"--force".to_string())
        );
    }

    #[test]
    fn a_fetch_prunes_so_a_stale_remote_ref_cannot_fake_unpushed_work() {
        let args = git().argv(&["fetch", "--prune", "origin"]);
        assert!(args.contains(&"--prune".to_string()));
    }
}
