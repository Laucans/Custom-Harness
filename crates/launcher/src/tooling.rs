//! Les portes qu'aucun workflow ne possède : les outils, et la branche.
//!
//! Elles sont ici et non chez un workflow parce qu'aucune n'est propre à l'un
//! d'eux : `claude` sur le `PATH` et à la bonne version, `gh` authentifié, une
//! branche d'intégration qui existe et qui déclenche la CI, un arbre propre. Un
//! workflow composera la liste qui le concerne ; ce qui lui est propre reste
//! chez lui.
//!
//! Génériques sur l'état du workflow (`S`) : aucune ne le lit.
//!
//! # La porte de version, et pourquoi elle est dure
//!
//! `claude --version` doit rendre **au moins [`MINIMUM`]**. La raison est dans
//! `docs/SESSION-CARRIER.md` : depuis la v2.1.277, `total_cost_usd` sur un
//! appel `--resume` est cumulatif pour toute la conversation, et ne couvrait que
//! l'appel avant. Le registre lit le dernier tour d'une stage ; sous une version
//! plus ancienne cette valeur serait le coût du dernier tour seul, et le
//! registre sous-compterait sans que rien ne le montre.
//!
//! Une porte coûte un appel local. Un `costs.tsv` faux ne se voit pas — c'est
//! tout l'arbitrage, et c'est pourquoi elle refuse plutôt que d'avertir.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::disk::Disk;
use harness_core::adapters::shell::git::Repo;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::shell::process;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

/// La version de `claude` en dessous de laquelle le registre mentirait.
pub const MINIMUM: Version = Version(2, 1, 277);

/// Une version, en trois nombres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// La version que ce texte annonce, ou `None`.
///
/// `claude --version` rend `2.1.285 (Claude Code)` : on lit le premier mot, et
/// on ignore le reste — ce qui suit a déjà changé de forme une fois.
///
/// `None` plutôt qu'un zéro optimiste : une version qu'on ne sait pas lire n'est
/// pas une version ancienne, et les deux ne méritent pas le même message.
#[must_use]
pub fn parse_version(text: &str) -> Option<Version> {
    let first = text.split_whitespace().next()?;
    let mut parts = first.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    // Un suffixe de pré-publication (`277-beta.1`) ne doit pas rendre la
    // version illisible : on lit les chiffres de tête.
    let patch = parts.next().unwrap_or("0");
    let digits: String = patch.chars().take_while(char::is_ascii_digit).collect();
    Some(Version(major, minor, digits.parse().ok()?))
}

/// `claude` est sur le `PATH`, et assez récent pour que le registre soit juste.
pub struct ClaudeIsRecentEnough {
    /// La version au moins exigée.
    pub minimum: Version,
}

impl Default for ClaudeIsRecentEnough {
    fn default() -> Self {
        Self { minimum: MINIMUM }
    }
}

#[async_trait(?Send)]
impl<S> Verification<S> for ClaudeIsRecentEnough {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        let said = process::run(
            "claude",
            &["--version".to_string()],
            std::path::Path::new("."),
        )
        .await
        .map_err(|_| Halt::Halted("claude CLI not on PATH".to_string()))?;
        if !said.ok() {
            return Err(Halt::Halted(format!(
                "`claude --version` failed: {}",
                said.why()
            )));
        }
        let Some(found) = parse_version(said.out()) else {
            return Err(Halt::Halted(format!(
                "cannot read the claude version from {:?} — the harness needs \
                 to know it is at least {}, because `total_cost_usd` changed \
                 meaning there (see docs/SESSION-CARRIER.md)",
                said.out(),
                self.minimum
            )));
        };
        if found < self.minimum {
            return Err(Halt::Halted(format!(
                "claude {found} is older than {} — on a resumed session \
                 `total_cost_usd` only covered the last call before that \
                 version, so costs.tsv would undercount every stage without \
                 showing it. Run: claude update (and check that `command -v \
                 claude` resolves to what you just updated)",
                self.minimum
            )));
        }
        ctx.traces.debug(&format!("claude {found}"));
        Ok(Verdict::Continue)
    }
}

/// `gh` répond, et il est authentifié.
pub struct GhIsAuthenticated {
    /// Le port GitHub.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl<S> Verification<S> for GhIsAuthenticated {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        if self.gh.authenticated().await? {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(
            "gh is not authenticated — run: gh auth login".to_string(),
        ))
    }
}

/// La branche d'intégration existe, est celle du checkout, et est sur
/// `origin`.
///
/// Les trois en une porte parce qu'elles se supposent : demander si `origin` la
/// porte n'a pas de sens tant qu'elle n'existe pas localement, et chaque échec
/// nomme sa propre commande.
pub struct TheIntegrationBranch {
    /// Le dépôt où la question se pose.
    pub git: Rc<dyn Repo>,
    /// La branche attendue.
    pub branch: String,
    /// Faux quand le run travaille dans un clone : la branche courante y est
    /// posée par le montage, pas par un humain.
    pub check_current: bool,
}

#[async_trait(?Send)]
impl<S> Verification<S> for TheIntegrationBranch {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        let branch = &self.branch;
        if !self.git.has_branch(branch).await? {
            return Err(Halt::Halted(format!(
                "branch {branch} does not exist — run: git branch {branch} \
                 origin/main"
            )));
        }
        if self.check_current {
            let current = self.git.current_branch().await?;
            if current != *branch {
                return Err(Halt::Halted(format!(
                    "on branch {current} — run: git checkout {branch}"
                )));
            }
        }
        if !self.git.origin_has_branch(branch).await? {
            return Err(Halt::Halted(format!(
                "origin has no {branch} — run: git push -u origin {branch}"
            )));
        }
        Ok(Verdict::Continue)
    }
}

/// La CI se déclenche sur la branche d'intégration.
///
/// Sans ça les PR que les stages ouvrent ne portent aucun check `ci`, et le
/// `gh pr checks` que les skills attendent ne se résout jamais — la stage
/// tourne jusqu'au bout de son budget et meurt sans rien livrer.
pub struct CiTriggersOnTheBranch {
    /// De quoi lire le fichier.
    pub disk: Rc<dyn Disk>,
    /// Le workflow de CI du dépôt cible.
    pub path: PathBuf,
    /// La branche attendue.
    pub branch: String,
}

#[async_trait(?Send)]
impl<S> Verification<S> for CiTriggersOnTheBranch {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        let missing = Halt::Halted(format!(
            ".github/workflows/ci.yml does not trigger on {} — the PRs the \
             stages open would carry no `ci` check, and the `gh pr checks` the \
             skills wait on would never resolve",
            self.branch
        ));
        if !self.disk.exists(&self.path) {
            return Err(missing);
        }
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| Halt::Unreadable(format!("{} : {e}", self.path.display())))?;
        if text.contains(&self.branch) {
            return Ok(Verdict::Continue);
        }
        Err(missing)
    }
}

/// L'arbre de travail est propre, sauf si le run a demandé le contraire.
pub struct WorkingTreeIsClean {
    /// Le dépôt où la question se pose.
    pub git: Rc<dyn Repo>,
    /// `--allow-dirty`.
    pub allowed: bool,
}

#[async_trait(?Send)]
impl<S> Verification<S> for WorkingTreeIsClean {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        if self.allowed {
            return Ok(Verdict::Continue);
        }
        let dirty = self.git.dirty_files().await?;
        if dirty.is_empty() {
            return Ok(Verdict::Continue);
        }
        for line in &dirty {
            ctx.traces.warn(line);
        }
        Err(Halt::Halted(
            "working tree is dirty — commit, stash, or re-run with \
             --allow-dirty"
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_read_off_the_first_word_and_the_rest_is_ignored() {
        // Ce qui suit le numéro a déjà changé de forme une fois.
        assert_eq!(
            parse_version("2.1.285 (Claude Code)"),
            Some(Version(2, 1, 285))
        );
        assert_eq!(parse_version("2.1.257"), Some(Version(2, 1, 257)));
    }

    #[test]
    fn a_prerelease_suffix_does_not_make_a_version_unreadable() {
        assert_eq!(parse_version("2.1.277-beta.1"), Some(Version(2, 1, 277)));
    }

    #[test]
    fn a_version_that_cannot_be_read_is_none_not_an_optimistic_zero() {
        // Une version illisible et une version ancienne ne méritent pas le
        // même message.
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("inconnue"), None);
        assert_eq!(parse_version("2"), None);
    }

    #[test]
    fn the_ordering_is_by_number_not_by_text() {
        // `"2.1.9" > "2.1.277"` en comparaison de chaînes, et c'est exactement
        // l'erreur que trois `u32` rendent impossible.
        assert!(Version(2, 1, 9) < Version(2, 1, 277));
        assert!(Version(2, 2, 0) > MINIMUM);
        assert!(Version(2, 1, 257) < MINIMUM);
        assert!(MINIMUM >= MINIMUM);
    }

    #[test]
    fn the_minimum_is_the_version_where_total_cost_usd_changed_meaning() {
        assert_eq!(MINIMUM, Version(2, 1, 277));
        assert_eq!(MINIMUM.to_string(), "2.1.277");
    }
}
