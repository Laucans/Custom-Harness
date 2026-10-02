//! Monter le workspace d'un run, et le démonter.
//!
//! Ce que [`Wanted`] **déclare**, ce module l'**exécute** : trouver ou cloner
//! le dossier, le remettre à l'état d'`origin`, et le supprimer à la fin quand
//! il était jetable.
//!
//! # Trois règles, chacune contre une façon de perdre du travail
//!
//! - **rien n'est écrasé sans le dire.** Un workspace réutilisé qui porte un
//!   patch qu'`origin` n'a pas, un stash ou un arbre sale arrête le run en le
//!   nommant, plutôt que de passer un `reset --hard` dessus. `force_reset` est
//!   la sortie, et c'est un humain qui la prend ;
//! - **rien n'est supprimé sans le dire.** Même question au démontage : un
//!   workspace jetable qui porte du travail que personne d'autre n'a est
//!   gardé, et son chemin imprimé ;
//! - **un dry-run ne clone pas.** Il dit ce qu'il aurait monté, et le run
//!   travaille sur place. Écrire un demi-gigaoctet pour « n'appeler rien »
//!   serait la contradiction la plus chère du paquet.
//!
//! # Ce que l'immuabilité des `Settings` supprime
//!
//! Côté Python, le montage **réécrivait `cfg.workspace`** en place, avec le
//! commentaire qu'« un second chemin par où passer la racine ferait deux
//! endroits à tenir d'accord ». Ici il rend un [`Mount`], et le lanceur
//! construit les `Settings` **après** : la décision n°6 fait que le hack n'a
//! plus d'objet, et personne ne peut plus réécrire une racine sous les pieds
//! d'une stage en vol.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::adapters::shell::disk::Disk;
use crate::adapters::shell::git::{Repo, Repos};
use crate::domain::workspace::{Strategy, Wanted, Workspace};
use crate::domain::{Halt, Outcome};
use crate::traces::Logbook;

/// Ce qu'un montage a donné, et ce que le démontage doit en faire.
#[derive(Debug, Clone)]
pub struct Mount {
    /// Le workspace du run : son code, et sa comptabilité restée en place.
    pub workspace: Workspace,
    /// Le nom du dossier. Vide quand le run travaille sur place.
    pub name: String,
    /// Le dossier monté. `None` quand le run travaille sur place — il n'y a
    /// alors rien à démonter, et c'est ce que `None` dit ici.
    pub path: Option<PathBuf>,
    /// La branche sur laquelle ce workspace tourne.
    ///
    /// Le démontage en a besoin : « ce workspace porte-t-il du travail ? » se
    /// compare à `origin/<elle>`.
    pub branch: String,
    /// Supprimé au démontage.
    ///
    /// Faux dès qu'un humain a nommé ce workspace ou demandé de le garder, et
    /// toujours faux en [`Strategy::Permanent`].
    pub disposable: bool,
}

impl Mount {
    /// Un run qui travaille sur place : rien n'a été monté.
    #[must_use]
    pub const fn in_place(workspace: Workspace) -> Self {
        Self {
            workspace,
            name: String::new(),
            path: None,
            branch: String::new(),
            disposable: false,
        }
    }

    /// Vrai si quelque chose a été monté.
    #[must_use]
    pub const fn mounted(&self) -> bool {
        self.path.is_some()
    }
}

/// Ce qui monte et démonte : un `git` par dépôt, et un disque.
pub struct Provisioner {
    /// De quoi ouvrir un `git` sur un dépôt donné — la source, le dossier
    /// parent, le clone.
    pub repos: Rc<dyn Repos>,
    /// De quoi créer, lister et supprimer.
    pub disk: Rc<dyn Disk>,
}

/// Ce qu'un montage a besoin de savoir du run qui le demande.
pub struct Run<'a> {
    /// Le workspace d'où le run est lancé : la source du clone, et la racine de
    /// comptabilité.
    pub source: &'a Workspace,
    /// Le workspace demandé.
    pub wanted: &'a Wanted,
    /// La branche d'intégration du workflow, si elle en a une.
    pub branch: &'a str,
    /// L'identifiant du run, qui nomme un workspace jetable.
    pub run_id: &'a str,
    /// N'exécute rien, ne clone rien.
    pub dry_run: bool,
}

impl Provisioner {
    /// Le workspace que ce run demande, monté.
    ///
    /// # Errors
    ///
    /// [`Halt::Halted`] à chaque fois qu'un humain a un geste à faire : pas
    /// d'`origin` à cloner, un nom de workspace qui est un chemin, un nom
    /// introuvable, un dossier qui n'est pas un workspace, un clone d'un autre
    /// dépôt, du travail à sauver. [`Halt::Failed`] quand `git` ou le disque
    /// n'ont pas répondu.
    pub async fn mount(&self, run: &Run<'_>, log: &Logbook) -> Outcome<Mount> {
        let source = run.source;
        let wanted = run.wanted;
        let url = self.url_for(run).await?;
        let base = if wanted.base.is_empty() {
            source.workspaces()
        } else {
            // Un chemin relatif se lit depuis le dépôt, jamais depuis le
            // répertoire courant : la boucle est lancée de n'importe où — un
            // cron, un hook, un autre checkout — et une base relative
            // déposerait sinon un clone de plusieurs centaines de mégaoctets là
            // où le shell se trouvait.
            source.root().join(&wanted.base)
        };
        let named = !wanted.id.is_empty();
        let name = if named {
            wanted.id.clone()
        } else {
            generated(wanted, &url, run.run_id)?
        };
        if !is_a_name(&name) {
            return Err(Halt::Halted(format!(
                "{name:?} is not a workspace name — give a single name, not a \
                 path: workspaces live in {}",
                source.rel(&base)
            )));
        }
        let dest = base.join(&name);

        if run.dry_run {
            log.say(&format!(
                "workspace: would use {} ({}, from {url}) — dry run works in \
                 place",
                source.rel(&dest),
                wanted.strategy.as_str()
            ));
            return Ok(Mount::in_place(source.clone()));
        }

        let branch = if self.disk.exists(&dest) {
            self.reuse(&dest, run, &url, &name, log).await?
        } else if named {
            // `--use-workspace` retrouve, il ne crée pas : un nom mal tapé
            // ferait sinon un clone frais sous un nom voisin, et le workspace
            // que l'humain visait resterait intact et inutilisé à côté. Sous
            // `Permanent` le clone de trop ne serait même pas supprimé à la
            // fin, donc la faute de frappe resterait sur le disque.
            let kept = self.disk.dir_names(&base).join(" ");
            return Err(Halt::Halted(format!(
                "no workspace named {name:?} under {} — kept workspaces: {}",
                source.rel(&base),
                if kept.is_empty() { "(none)" } else { &kept }
            )));
        } else {
            self.clone_fresh(&url, &dest, &name, run.branch, log)
                .await?
        };

        // Un jetable engendré est ce qui se supprime ; nommer un workspace ou
        // demander de le garder revient au même, et `Permanent` ne se supprime
        // jamais.
        let disposable = wanted.strategy == Strategy::Tmp && !named && !wanted.keep;
        // `source.rel` et non celui du workspace monté : un chemin relatif à
        // lui-même vaudrait « . ».
        log.say(&format!(
            "workspace: {} ({}, {})",
            source.rel(&dest),
            wanted.strategy.as_str(),
            if disposable { "jetable" } else { "gardé" }
        ));
        self.say_what_stays_behind(source, log).await;
        Ok(Mount {
            workspace: source.at(&dest),
            name,
            path: Some(dest),
            branch,
            disposable,
        })
    }

    /// Supprime le workspace s'il était jetable, et s'il ne porte rien.
    ///
    /// **Ne rend rien et ne peut pas faire échouer un run** : ce qui est arrivé
    /// au workflow est déjà décidé quand on arrive ici, et un dossier qui
    /// résiste ne doit pas requalifier un run réussi en panne. Ce qui n'a pas pu
    /// être supprimé est dit, et reste.
    pub async fn unmount(&self, mount: &Mount, log: &Logbook) {
        let Some(path) = &mount.path else {
            return;
        };
        if !mount.disposable {
            return;
        }
        let git = self.repos.at(path);
        // Le run vient peut-être de merger sa propre PR : sans ce `fetch`, la
        // garde juge contre des références figées au montage, la branche du
        // round paraît porter du travail que personne n'a, et **tout** workspace
        // jetable finit gardé — le remplissage de disque que ce chemin existe
        // pour éviter. Un fetch qui échoue laisse la garde sévère, ce qui est le
        // bon sens de l'erreur.
        if !mount.branch.is_empty() {
            let _ = git.fetch().await;
        }
        let held = match self.what_is_held(path, git.as_ref(), &mount.branch).await {
            Ok(said) => said,
            // Une lecture qui n'aboutit pas ne vaut pas « rien à perdre ».
            Err(why) => format!("son état n'a pas pu être lu ({})", why.reason()),
        };
        if !held.is_empty() {
            // Ce qu'il ne faut pas dire ici : « reprends-le avec
            // `--use-workspace` ». Le montage reposerait la même question,
            // verrait le même travail et s'arrêterait — et `--force-reset`
            // l'effacerait. Le workspace est gardé pour être **regardé**.
            log.warn(&format!(
                "workspace {} kept: {held} — it is at {}; salvage what you \
                 need, then re-run with --use-workspace {} --force-reset",
                mount.name,
                path.display(),
                mount.name
            ));
            return;
        }
        match self.disk.remove_dir_all(path) {
            Ok(()) => log.debug(&format!("workspace {} removed", mount.name)),
            Err(why) => log.warn(&format!(
                "workspace {} could not be removed ({})",
                mount.name,
                why.reason()
            )),
        }
    }

    // --- les trois moitiés du montage -------------------------------------

    /// L'URL à cloner : celle demandée, sinon l'`origin` du dépôt source.
    async fn url_for(&self, run: &Run<'_>) -> Outcome<String> {
        if !run.wanted.url.is_empty() {
            return Ok(run.wanted.url.clone());
        }
        let url = self
            .repos
            .at(run.source.root())
            .remote_url("origin")
            .await?;
        if url.is_empty() {
            return Err(Halt::Halted(
                "no repository to clone: no workspace URL was given and this \
                 checkout has no `origin` remote — set one, or run without a \
                 workspace"
                    .to_string(),
            ));
        }
        Ok(url)
    }

    /// Un clone frais, posé sur la branche que le workflow travaille.
    async fn clone_fresh(
        &self,
        url: &str,
        dest: &Path,
        name: &str,
        branch: &str,
        log: &Logbook,
    ) -> Outcome<String> {
        let Some(parent) = dest.parent() else {
            return Err(Halt::Failed(format!(
                "{} n'a pas de dossier parent",
                dest.display()
            )));
        };
        self.disk.create_dir_all(parent)?;
        log.say(&format!("cloning {url} -> {name}"));
        let done = self.repos.at(parent).clone_repo(url, name).await?;
        if !done.ok() {
            // Un clone à moitié écrit est pire qu'aucun : le run suivant le
            // prendrait pour un workspace réutilisable.
            let _ = self.disk.remove_dir_all(dest);
            return Err(Halt::Halted(format!("cannot clone {url}: {}", done.why())));
        }
        let ready = self
            .put_on_branch(self.repos.at(dest).as_ref(), branch, false, log)
            .await;
        if ready.is_err() {
            // Même raison, et le cas est réel : une branche d'intégration qui
            // n'existe pas sur `origin` laissait un clone complet derrière elle
            // à chaque essai. Le démontage ne le rattrape pas — un montage qui
            // échoue n'a pas de `Mount` à rendre.
            let _ = self.disk.remove_dir_all(dest);
        }
        ready
    }

    /// Un workspace déjà là, remis à l'état d'`origin`.
    ///
    /// **L'ordre compte, et il a coûté un run pour être trouvé : le `fetch`
    /// passe avant la garde.** Celle-ci demande si une branche locale porte un
    /// patch qu'`origin` n'a pas — question dont la réponse dépend entièrement
    /// de la fraîcheur d'`origin/<branche>`. La poser d'abord la posait contre
    /// une référence périmée, donc une PR fusionnée depuis le dernier run
    /// comptait encore comme du travail à sauver, et le workspace se bloquait
    /// pour de bon.
    ///
    /// `fetch` ne détruit rien. Ce qui détruit vient après la garde, et
    /// seulement après.
    async fn reuse(
        &self,
        dest: &Path,
        run: &Run<'_>,
        url: &str,
        name: &str,
        log: &Logbook,
    ) -> Outcome<String> {
        // Avant tout le reste, et avant `force_reset` : un dossier qu'on ne
        // sait pas lire ne se remet pas à zéro. Côté Python cette garde était
        // *derrière* le `force`, c'est-à-dire absente sur le seul chemin
        // destructeur.
        if let Some(wrong) = self.not_a_workspace(dest) {
            return Err(Halt::Halted(format!("the workspace {name:?}: {wrong}")));
        }
        let git = self.repos.at(dest);
        // Deux URL différentes peuvent porter le même nom de dépôt, et c'est ce
        // nom qui nomme un workspace permanent. Sans ce test, le run
        // travaillerait en silence sur le mauvais dépôt — et y pousserait.
        let here = git.remote_url("origin").await?;
        if !here.is_empty() && !same_repo(&here, url) {
            return Err(Halt::Halted(format!(
                "the workspace {name:?} is a clone of {here}, not of {url} — \
                 give it another name, or remove it"
            )));
        }
        log.debug(&format!("workspace {name}: fetch + reset on origin"));
        let fetched = git.fetch().await?;
        if !fetched.ok() {
            return Err(Halt::Halted(format!(
                "cannot fetch in the workspace {name:?}: {}",
                fetched.why()
            )));
        }
        if !run.wanted.force_reset {
            let guard_branch = if run.branch.is_empty() {
                git.default_branch().await?
            } else {
                run.branch.to_string()
            };
            let held = self.what_is_held(dest, git.as_ref(), &guard_branch).await?;
            if !held.is_empty() {
                return Err(Halt::Halted(format!(
                    "the workspace {name:?} carries {held} — look at {}, or \
                     re-run with --force-reset to overwrite it",
                    run.source.rel(dest)
                )));
            }
        }
        self.put_on_branch(git.as_ref(), run.branch, true, log)
            .await
    }

    /// Le workspace sur la branche du workflow, à l'état d'`origin`.
    async fn put_on_branch(
        &self,
        git: &dyn Repo,
        wanted: &str,
        reset: bool,
        log: &Logbook,
    ) -> Outcome<String> {
        let branch = self.which_branch(git, wanted).await?;
        if branch.is_empty() {
            if reset {
                // Rendre un succès sans avoir remis à zéro ferait tourner le
                // run sur l'état du round précédent, en le disant propre.
                return Err(Halt::Halted(
                    "cannot tell which branch this workspace should be on: \
                     neither an integration branch nor origin/HEAD resolves"
                        .to_string(),
                ));
            }
            return Ok(String::new());
        }
        let done = git.checkout(&branch, reset).await?;
        if !done.ok() {
            return Err(Halt::Halted(format!(
                "the workspace has no branch {branch}: {}",
                done.why()
            )));
        }
        if !reset {
            return Ok(branch);
        }
        let back = git.reset_hard(&format!("origin/{branch}")).await?;
        if !back.ok() {
            return Err(Halt::Halted(format!(
                "cannot reset the workspace on origin/{branch}: {}",
                back.why()
            )));
        }
        git.clean().await?;
        self.prune_branches(git, &branch, log).await?;
        Ok(branch)
    }

    /// La branche du workflow, sinon celle d'`origin/HEAD`, sinon la courante.
    ///
    /// La courante en dernier recours : après un clone, `HEAD` est toujours
    /// quelque part, et un [`Mount`] sans branche fait retomber le démontage
    /// sur la question par sha — celle qui garde tout.
    async fn which_branch(&self, git: &dyn Repo, wanted: &str) -> Outcome<String> {
        if !wanted.is_empty() {
            return Ok(wanted.to_string());
        }
        let default = git.default_branch().await?;
        if !default.is_empty() {
            return Ok(default);
        }
        git.current_branch().await
    }

    /// Les branches locales que le round précédent a laissées, effacées.
    ///
    /// Sans ça un workspace permanent se bloque, définitivement : le dépôt
    /// cible fusionne en rebase, donc une PR fusionnée laisse une branche
    /// locale dont les commits ne sont sur aucun remote par sha. Le
    /// `fetch --prune` efface ensuite son `origin/<branche>`, et la garde
    /// « travail non poussé » se met à signaler du travail déjà livré — à
    /// chaque run, sans issue.
    ///
    /// On n'arrive ici qu'**après** la garde : ce qui restait à perdre a déjà
    /// arrêté le run, ou `force_reset` a dit de l'écraser.
    async fn prune_branches(&self, git: &dyn Repo, branch: &str, log: &Logbook) -> Outcome<()> {
        for stale in git.local_branches().await? {
            if stale == branch {
                continue;
            }
            if git.delete_branch(&stale).await?.ok() {
                log.debug(&format!("workspace: branche locale {stale} effacée"));
            }
        }
        Ok(())
    }

    // --- ce qu'on demande au disque avant d'écrire dessus ------------------

    /// Pourquoi ce dossier n'est pas un workspace, ou `None`.
    ///
    /// Vérifié avant tout le reste, `force_reset` compris : on ne sait pas ce
    /// qu'il y a dans un dossier posé là à la main, donc on n'y touche pas. Le
    /// lire comme un workspace vide est ce qu'il ne faut jamais faire.
    fn not_a_workspace(&self, dest: &Path) -> Option<&'static str> {
        if self.disk.exists(&dest.join(".git")) {
            return None;
        }
        Some("no .git — this is not a workspace this run made")
    }

    /// Ce que ce workspace porte et que personne d'autre n'a, dit en mots.
    ///
    /// Le vide quand il n'y a rien à perdre. **La question posée deux fois** —
    /// avant un `reset --hard`, avant une suppression — et une seule réponse,
    /// parce que les deux détruisent exactement la même chose.
    async fn what_is_held(&self, dest: &Path, git: &dyn Repo, branch: &str) -> Outcome<String> {
        if let Some(wrong) = self.not_a_workspace(dest) {
            return Ok(wrong.to_string());
        }
        let mut said = Vec::new();
        let stashed = git.stashes().await?;
        if !stashed.is_empty() {
            said.push(format!("{} stash entry(ies)", stashed.len()));
        }
        if branch.is_empty() {
            let commits = git.unpushed().await?;
            if let Some(first) = commits.first() {
                said.push(format!("{} unpushed commit(s) ({first})", commits.len()));
            }
        } else {
            let at_risk = git.branches_at_risk(&format!("origin/{branch}")).await?;
            if !at_risk.is_empty() {
                said.push(format!(
                    "work on {} branch(es) origin does not have ({})",
                    at_risk.len(),
                    at_risk
                        .iter()
                        .take(3)
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        let dirty = git.dirty_files().await?;
        if !dirty.is_empty() {
            said.push(format!("{} uncommitted change(s)", dirty.len()));
        }
        Ok(said.join(", "))
    }

    /// Prévenir que le travail local du checkout n'est pas dans le workspace.
    ///
    /// Le run clone `origin` : ce qui n'y est pas poussé n'existe pas pour lui.
    /// Tant que la boucle tournait sur place, la porte « arbre propre » le
    /// disait d'elle-même ; elle regarde désormais un clone, propre par
    /// construction. Sans cette ligne, un humain qui a oublié de pousser voit
    /// un run vert bâti sur un état qui n'est pas le sien.
    ///
    /// La branche courante seulement, et trois appels en tout. Balayer toutes
    /// les branches locales poserait ici une question dont la réponse dépend de
    /// la fraîcheur d'`origin/…` — et on ne va pas faire un `fetch` dans le
    /// dépôt de l'humain pour écrire un avertissement.
    async fn say_what_stays_behind(&self, source: &Workspace, log: &Logbook) {
        let git = self.repos.at(source.root());
        let mut held = Vec::new();
        if let Ok(dirty) = git.dirty_files().await
            && !dirty.is_empty()
        {
            held.push(format!("{} uncommitted change(s)", dirty.len()));
        }
        if let Ok(ahead) = git.unpushed().await
            && !ahead.is_empty()
        {
            let branch = git.current_branch().await.unwrap_or_default();
            let where_ = if branch.is_empty() { "HEAD" } else { &branch };
            held.push(format!("{} commit(s) not pushed on {where_}", ahead.len()));
        }
        if !held.is_empty() {
            log.warn(&format!(
                "your checkout holds {} — the workspace is cloned from origin, \
                 so none of it is in this run",
                held.join(", ")
            ));
        }
    }
}

/// Le nom d'un workspace que personne n'a nommé.
///
/// `Permanent` en tire un par dépôt cible — deux URL différentes ne se marchent
/// pas dessus, et la même URL retrouve son dossier d'un run à l'autre, ce qui
/// est toute la stratégie. `Tmp` y ajoute l'identifiant du run.
fn generated(wanted: &Wanted, url: &str, run_id: &str) -> Outcome<String> {
    let repo = repo_name(url);
    if wanted.strategy == Strategy::Permanent {
        return Ok(repo);
    }
    if run_id.is_empty() {
        // Sans identifiant, deux runs jetables simultanés viseraient le même
        // dossier, et le premier à finir supprimerait le workspace du second.
        return Err(Halt::Failed(
            "a disposable workspace needs a run id to be named — the launcher \
             always sets one"
                .to_string(),
        ));
    }
    Ok(format!("{repo}-{run_id}"))
}

/// `git@github.com:Laucans/event_assistant.git` → `event_assistant`.
fn repo_name(url: &str) -> String {
    let tail = url.trim_end_matches('/');
    let tail = tail.rsplit(['/', ':']).next().unwrap_or(tail);
    let tail = tail.strip_suffix(".git").unwrap_or(tail);
    if tail.is_empty() {
        "workspace".to_string()
    } else {
        tail.to_string()
    }
}

/// Un seul segment de chemin, et pas un qui remonte.
///
/// `base.join("../..")` se résout **hors** du dossier des workspaces, et tout
/// ce que ce module fait ensuite est destructeur : sans ce test, un nom comme
/// `../..` recevait le `reset --hard`, le `clean -fd` et le `branch -D` — sur
/// le checkout de l'humain, sans même avoir à passer `force_reset`, puisque le
/// dossier visé est bien un dépôt et bien le même `origin`.
fn is_a_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name).components().count() == 1
        && !Path::new(name).is_absolute()
}

/// Deux URL qui désignent le même dépôt, aux formes d'écriture près.
///
/// `git@github.com:o/r.git` et `https://github.com/o/r` sont le même dépôt ;
/// les comparer caractère pour caractère ferait refuser un workspace
/// parfaitement valide.
fn same_repo(one: &str, other: &str) -> bool {
    key(one) == key(other)
}

fn key(url: &str) -> String {
    let mut text = url.trim_end_matches('/');
    text = text.strip_suffix(".git").unwrap_or(text);
    for prefix in ["https://", "http://", "ssh://", "git://"] {
        text = text.strip_prefix(prefix).unwrap_or(text);
    }
    let text = text.replace(':', "/");
    text.strip_prefix("git@").unwrap_or(&text).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shell::process::Ran;
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::collections::HashSet;

    // --- les faux ----------------------------------------------------------
    //
    // Un faux disque et un faux `git`, et non un vrai dossier de test : ce
    // module est le seul du paquet qui supprime des centaines de mégaoctets, et
    // les tests qui comptent ici sont ceux qui prouvent qu'il **ne** supprime
    // pas. Les écrire contre un vrai clone demanderait d'espérer.

    /// Un `git` qui répond ce qu'on lui a dit, et note ce qu'on lui demande.
    #[derive(Default)]
    struct FakeRepo {
        remote: String,
        default_branch: String,
        current: String,
        local: Vec<String>,
        stashes: Vec<String>,
        at_risk: Vec<String>,
        unpushed: Vec<String>,
        dirty: Vec<String>,
        /// Les verbes qui sortent en non nul.
        failing: Vec<&'static str>,
        /// Dans l'ordre d'appel.
        calls: RefCell<Vec<String>>,
    }

    impl FakeRepo {
        fn note(&self, verb: &str) -> Ran {
            self.calls.borrow_mut().push(verb.to_string());
            let broke = self.failing.iter().any(|f| verb.starts_with(f));
            Ran {
                code: Some(i32::from(broke)),
                stdout: String::new(),
                stderr: if broke {
                    format!("fatal: {verb} a échoué")
                } else {
                    String::new()
                },
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    #[async_trait(?Send)]
    impl Repo for FakeRepo {
        async fn current_branch(&self) -> Outcome<String> {
            Ok(self.current.clone())
        }

        async fn head_sha(&self) -> Outcome<String> {
            Ok("abc1234".to_string())
        }

        async fn dirty_files(&self) -> Outcome<Vec<String>> {
            Ok(self.dirty.clone())
        }

        async fn has_branch(&self, _name: &str) -> Outcome<bool> {
            Ok(true)
        }

        async fn origin_has_branch(&self, _name: &str) -> Outcome<bool> {
            Ok(true)
        }

        async fn remote_url(&self, _remote: &str) -> Outcome<String> {
            Ok(self.remote.clone())
        }

        async fn tracked_files(&self) -> Outcome<Vec<String>> {
            Ok(Vec::new())
        }

        async fn default_branch(&self) -> Outcome<String> {
            Ok(self.default_branch.clone())
        }

        async fn local_branches(&self) -> Outcome<Vec<String>> {
            Ok(self.local.clone())
        }

        async fn stashes(&self) -> Outcome<Vec<String>> {
            Ok(self.stashes.clone())
        }

        async fn unpushed(&self) -> Outcome<Vec<String>> {
            Ok(self.unpushed.clone())
        }

        async fn branches_at_risk(&self, _upstream: &str) -> Outcome<Vec<String>> {
            self.calls.borrow_mut().push("branches_at_risk".to_string());
            Ok(self.at_risk.clone())
        }

        async fn clone_repo(&self, _url: &str, name: &str) -> Outcome<Ran> {
            Ok(self.note(&format!("clone {name}")))
        }

        async fn fetch(&self) -> Outcome<Ran> {
            Ok(self.note("fetch"))
        }

        async fn checkout(&self, branch: &str, force: bool) -> Outcome<Ran> {
            Ok(self.note(&format!(
                "checkout {branch}{}",
                if force { " --force" } else { "" }
            )))
        }

        async fn reset_hard(&self, reference: &str) -> Outcome<Ran> {
            Ok(self.note(&format!("reset_hard {reference}")))
        }

        async fn clean(&self) -> Outcome<Ran> {
            Ok(self.note("clean"))
        }

        async fn delete_branch(&self, name: &str) -> Outcome<Ran> {
            Ok(self.note(&format!("delete_branch {name}")))
        }
    }

    /// Un seul faux `git`, quel que soit le dépôt — ce qu'on exerce ici est la
    /// politique, pas le routage.
    struct FakeRepos(Rc<FakeRepo>);

    impl Repos for FakeRepos {
        fn at(&self, _root: &Path) -> Rc<dyn Repo> {
            Rc::clone(&self.0) as Rc<dyn Repo>
        }
    }

    /// Un disque en mémoire. Ce qu'il a supprimé se relit.
    #[derive(Default)]
    struct FakeDisk {
        there: HashSet<PathBuf>,
        names: Vec<String>,
        removed: RefCell<Vec<PathBuf>>,
        created: RefCell<Vec<PathBuf>>,
    }

    impl FakeDisk {
        fn removed(&self) -> Vec<PathBuf> {
            self.removed.borrow().clone()
        }
    }

    impl Disk for FakeDisk {
        fn exists(&self, path: &Path) -> bool {
            self.there.contains(path)
        }

        fn create_dir_all(&self, path: &Path) -> Outcome<()> {
            self.created.borrow_mut().push(path.to_path_buf());
            Ok(())
        }

        fn remove_dir_all(&self, path: &Path) -> Outcome<()> {
            self.removed.borrow_mut().push(path.to_path_buf());
            Ok(())
        }

        fn dir_names(&self, _path: &Path) -> Vec<String> {
            self.names.clone()
        }

        fn read_to_string(&self, _path: &Path) -> Option<String> {
            None
        }
    }

    // --- le décor ----------------------------------------------------------

    const ORIGIN: &str = "git@github.com:Laucans/event_assistant.git";
    const DEST: &str = "/depot/.llocal/agentic_workspaces/event_assistant";

    fn source() -> Workspace {
        Workspace::new(Path::new("/depot"))
    }

    fn repo(fake: FakeRepo) -> Rc<FakeRepo> {
        Rc::new(fake)
    }

    fn clean_repo() -> FakeRepo {
        FakeRepo {
            remote: ORIGIN.to_string(),
            default_branch: "main".to_string(),
            current: "main_agent".to_string(),
            ..FakeRepo::default()
        }
    }

    /// Un disque où le workspace est déjà là, `.git` compris.
    fn disk_with_workspace() -> FakeDisk {
        FakeDisk {
            there: [PathBuf::from(DEST), PathBuf::from(DEST).join(".git")]
                .into_iter()
                .collect(),
            ..FakeDisk::default()
        }
    }

    fn provisioner(git: &Rc<FakeRepo>, disk: Rc<FakeDisk>) -> Provisioner {
        Provisioner {
            repos: Rc::new(FakeRepos(Rc::clone(git))),
            disk,
        }
    }

    fn wanted() -> Wanted {
        Wanted {
            strategy: Strategy::Permanent,
            ..Wanted::default()
        }
    }

    async fn mount_with(
        git: &Rc<FakeRepo>,
        disk: &Rc<FakeDisk>,
        wanted: &Wanted,
        dry_run: bool,
    ) -> Outcome<Mount> {
        let here = source();
        provisioner(git, Rc::clone(disk))
            .mount(
                &Run {
                    source: &here,
                    wanted,
                    branch: "main_agent",
                    run_id: "20261002-1",
                    dry_run,
                },
                &Logbook::null(),
            )
            .await
    }

    // --- règle 3 : un dry-run ne clone pas ---------------------------------

    #[tokio::test]
    async fn a_dry_run_clones_nothing_and_works_in_place() {
        // Écrire un demi-gigaoctet pour « n'appeler rien » serait la
        // contradiction la plus chère du paquet.
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default());
        let mount = mount_with(&git, &disk, &wanted(), true)
            .await
            .expect("monté");
        assert!(!mount.mounted());
        assert_eq!(mount.workspace.root(), Path::new("/depot"));
        assert!(git.calls().is_empty(), "aucun verbe de git");
        assert!(disk.created.borrow().is_empty());
    }

    // --- le nom ------------------------------------------------------------

    #[tokio::test]
    async fn a_name_that_is_a_path_is_refused_before_anything_is_touched() {
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default());
        let asked = Wanted {
            id: "../..".to_string(),
            ..wanted()
        };
        let err = mount_with(&git, &disk, &asked, false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("is not a workspace name"));
        assert!(git.calls().is_empty(), "rien n'a été fait au dépôt");
        assert!(disk.removed().is_empty());
    }

    #[tokio::test]
    async fn a_named_workspace_is_found_never_created() {
        // Un nom mal tapé ferait sinon un clone frais sous un nom voisin, et le
        // workspace visé resterait intact et inutilisé à côté.
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk {
            names: vec!["event_assistant".to_string()],
            ..FakeDisk::default()
        });
        let asked = Wanted {
            id: "event_assistan".to_string(),
            ..wanted()
        };
        let err = mount_with(&git, &disk, &asked, false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("no workspace named"));
        assert!(
            err.reason().contains("event_assistant"),
            "lister ce qui est là"
        );
        assert!(git.calls().is_empty(), "aucun clone");
    }

    // --- règle 1 : rien n'est écrasé sans le dire --------------------------

    #[tokio::test]
    async fn a_reused_workspace_carrying_work_halts_instead_of_resetting() {
        let git = repo(FakeRepo {
            at_risk: vec!["fix/quelque-chose".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let err = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("fix/quelque-chose"));
        assert!(err.reason().contains("--force-reset"), "nommer la sortie");
        let calls = git.calls();
        assert!(
            !calls.iter().any(|c| c.starts_with("reset_hard")),
            "rien n'a été écrasé : {calls:?}"
        );
        assert!(!calls.iter().any(|c| c == "clean"));
    }

    #[tokio::test]
    async fn an_uncommitted_change_in_a_reused_workspace_also_halts() {
        let git = repo(FakeRepo {
            dirty: vec![" M src/main.rs".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let err = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("1 uncommitted change(s)"));
    }

    #[tokio::test]
    async fn the_fetch_comes_before_the_guard_so_it_judges_against_fresh_refs() {
        // Trouvé au prix d'un run : la garde posée d'abord jugeait contre une
        // référence périmée, donc une PR fusionnée depuis le dernier run
        // comptait encore comme du travail à sauver — et le workspace se
        // bloquait pour de bon.
        let git = repo(FakeRepo {
            at_risk: vec!["feat/deja-mergee".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let _ = mount_with(&git, &disk, &wanted(), false).await;
        let calls = git.calls();
        let fetch = calls.iter().position(|c| c == "fetch").expect("un fetch");
        let guard = calls
            .iter()
            .position(|c| c == "branches_at_risk")
            .expect("la garde");
        assert!(fetch < guard, "{calls:?}");
    }

    #[tokio::test]
    async fn force_reset_overwrites_and_puts_the_workspace_back_on_origin() {
        let git = repo(FakeRepo {
            at_risk: vec!["fix/perdu".to_string()],
            local: vec!["main_agent".to_string(), "fix/perdu".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let asked = Wanted {
            force_reset: true,
            ..wanted()
        };
        let mount = mount_with(&git, &disk, &asked, false).await.expect("monté");
        assert_eq!(mount.branch, "main_agent");
        let calls = git.calls();
        assert!(calls.contains(&"checkout main_agent --force".to_string()));
        assert!(calls.contains(&"reset_hard origin/main_agent".to_string()));
        assert!(calls.contains(&"clean".to_string()));
        // La branche du round survit, les autres sont effacées — sinon un
        // workspace permanent se bloque définitivement sur une PR rebasée.
        assert!(calls.contains(&"delete_branch fix/perdu".to_string()));
        assert!(!calls.contains(&"delete_branch main_agent".to_string()));
    }

    #[tokio::test]
    async fn a_directory_without_git_is_refused_even_with_force_reset() {
        // La garde était derrière le `force` côté Python, c'est-à-dire absente
        // sur le seul chemin destructeur. On ne sait pas ce qu'il y a dans un
        // dossier posé là à la main, donc on n'y touche pas.
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk {
            there: std::iter::once(PathBuf::from(DEST)).collect(),
            ..FakeDisk::default()
        });
        let asked = Wanted {
            force_reset: true,
            ..wanted()
        };
        let err = mount_with(&git, &disk, &asked, false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("no .git"));
        assert!(git.calls().is_empty());
        assert!(disk.removed().is_empty());
    }

    #[tokio::test]
    async fn a_workspace_cloned_from_another_repository_is_refused() {
        // Sans ce test, le run travaillerait en silence sur le mauvais dépôt —
        // et y pousserait.
        let git = repo(FakeRepo {
            remote: "git@github.com:someone/event_assistant.git".to_string(),
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        // Le dépôt visé est nommé : deux URL différentes portent le même nom de
        // dépôt, et c'est ce nom qui nomme un workspace permanent.
        let asked = Wanted {
            url: ORIGIN.to_string(),
            ..wanted()
        };
        let err = mount_with(&git, &disk, &asked, false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("is a clone of"));
        assert!(
            git.calls().is_empty(),
            "rien avant d'avoir identifié le dépôt"
        );
    }

    #[tokio::test]
    async fn two_spellings_of_the_same_remote_do_not_refuse_a_valid_workspace() {
        let git = repo(FakeRepo {
            remote: "https://github.com/Laucans/event_assistant".to_string(),
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let asked = Wanted {
            url: ORIGIN.to_string(),
            ..wanted()
        };
        assert!(mount_with(&git, &disk, &asked, false).await.is_ok());
    }

    // --- le clone ----------------------------------------------------------

    #[tokio::test]
    async fn a_fresh_clone_lands_on_the_integration_branch() {
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default());
        let mount = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect("monté");
        assert_eq!(mount.path.as_deref(), Some(Path::new(DEST)));
        assert_eq!(mount.branch, "main_agent");
        assert!(!mount.disposable, "permanent ne se supprime jamais");
        let calls = git.calls();
        assert!(calls.contains(&"clone event_assistant".to_string()));
        // Sans `--force` : il n'y a rien à écraser dans un clone frais.
        assert!(calls.contains(&"checkout main_agent".to_string()));
        assert!(!calls.iter().any(|c| c.starts_with("reset_hard")));
    }

    #[tokio::test]
    async fn a_half_written_clone_is_removed_rather_than_left_to_be_reused() {
        let git = repo(FakeRepo {
            failing: vec!["clone"],
            ..clean_repo()
        });
        let disk = Rc::new(FakeDisk::default());
        let err = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("cannot clone"));
        assert_eq!(disk.removed(), vec![PathBuf::from(DEST)]);
    }

    #[tokio::test]
    async fn a_clone_that_cannot_reach_the_branch_leaves_nothing_behind() {
        // Le cas est réel : une branche d'intégration absente d'`origin`
        // laissait un clone complet derrière elle à chaque essai.
        let git = repo(FakeRepo {
            failing: vec!["checkout"],
            ..clean_repo()
        });
        let disk = Rc::new(FakeDisk::default());
        let err = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("has no branch main_agent"));
        assert_eq!(disk.removed(), vec![PathBuf::from(DEST)]);
    }

    #[tokio::test]
    async fn no_origin_and_no_url_names_the_gesture_rather_than_guessing() {
        let git = repo(FakeRepo {
            remote: String::new(),
            ..clean_repo()
        });
        let disk = Rc::new(FakeDisk::default());
        let err = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("no `origin` remote"));
    }

    // --- règle 2 : rien n'est supprimé sans le dire ------------------------

    fn disposable(branch: &str) -> Mount {
        Mount {
            workspace: source().at(Path::new(DEST)),
            name: "event_assistant-20261002-1".to_string(),
            path: Some(PathBuf::from(DEST)),
            branch: branch.to_string(),
            disposable: true,
        }
    }

    async fn unmount_with(git: &Rc<FakeRepo>, disk: &Rc<FakeDisk>, mount: &Mount) {
        provisioner(git, Rc::clone(disk))
            .unmount(mount, &Logbook::null())
            .await;
    }

    #[tokio::test]
    async fn a_clean_disposable_workspace_is_removed() {
        let git = repo(clean_repo());
        let disk = Rc::new(disk_with_workspace());
        unmount_with(&git, &disk, &disposable("main_agent")).await;
        assert_eq!(disk.removed(), vec![PathBuf::from(DEST)]);
    }

    #[tokio::test]
    async fn a_disposable_workspace_holding_work_is_kept() {
        let git = repo(FakeRepo {
            stashes: vec!["stash@{0}: WIP".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        unmount_with(&git, &disk, &disposable("main_agent")).await;
        assert!(disk.removed().is_empty(), "gardé pour être regardé");
    }

    #[tokio::test]
    async fn the_unmount_fetches_first_so_a_just_merged_pr_is_not_mistaken_for_work() {
        // Sans ce fetch, **tout** workspace jetable finit gardé — le
        // remplissage de disque que ce chemin existe pour éviter.
        let git = repo(clean_repo());
        let disk = Rc::new(disk_with_workspace());
        unmount_with(&git, &disk, &disposable("main_agent")).await;
        let calls = git.calls();
        let fetch = calls.iter().position(|c| c == "fetch").expect("un fetch");
        let guard = calls
            .iter()
            .position(|c| c == "branches_at_risk")
            .expect("la garde");
        assert!(fetch < guard, "{calls:?}");
    }

    #[tokio::test]
    async fn a_permanent_workspace_is_never_removed() {
        let git = repo(clean_repo());
        let disk = Rc::new(disk_with_workspace());
        let kept = Mount {
            disposable: false,
            ..disposable("main_agent")
        };
        unmount_with(&git, &disk, &kept).await;
        assert!(disk.removed().is_empty());
        assert!(git.calls().is_empty(), "rien ne lui est même demandé");
    }

    #[tokio::test]
    async fn a_workspace_whose_state_cannot_be_read_is_kept_not_removed() {
        // Une lecture qui n'aboutit pas ne vaut pas « rien à perdre ».
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default()); // pas de `.git`
        unmount_with(&git, &disk, &disposable("main_agent")).await;
        assert!(disk.removed().is_empty());
    }

    #[tokio::test]
    async fn without_a_branch_the_guard_falls_back_to_unpushed_commits() {
        let git = repo(FakeRepo {
            unpushed: vec!["abc1234 un commit à personne".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        unmount_with(&git, &disk, &disposable("")).await;
        assert!(disk.removed().is_empty());
    }

    // --- les fonctions pures -----------------------------------------------

    #[test]
    fn a_workspace_name_is_a_name_not_a_path() {
        // Ce que ce test empêche : `reset --hard` et `clean -fd` sur le
        // checkout de l'humain, sans même un --force-reset.
        assert!(is_a_name("event_assistant"));
        assert!(!is_a_name("../.."));
        assert!(!is_a_name(".."));
        assert!(!is_a_name("."));
        assert!(!is_a_name(""));
        assert!(!is_a_name("a/b"));
        assert!(!is_a_name("/absolu"));
        assert!(!is_a_name("a\\b"));
    }

    #[test]
    fn two_spellings_of_the_same_remote_are_the_same_repository() {
        assert!(same_repo(
            "git@github.com:Laucans/event_assistant.git",
            "https://github.com/Laucans/event_assistant"
        ));
        assert!(same_repo(
            "https://github.com/o/r/",
            "https://github.com/o/r"
        ));
    }

    #[test]
    fn two_repositories_with_the_same_tail_are_not_the_same_remote() {
        // Le mode de panne que ça évite : travailler, et pousser, sur le
        // mauvais dépôt, parce que c'est le nom du dépôt qui nomme un
        // workspace permanent.
        assert!(!same_repo(
            "git@github.com:someone/event_assistant.git",
            "git@github.com:Laucans/event_assistant.git"
        ));
    }

    #[test]
    fn the_workspace_of_a_repository_is_named_after_it() {
        assert_eq!(
            repo_name("git@github.com:Laucans/event_assistant.git"),
            "event_assistant"
        );
        assert_eq!(repo_name("https://github.com/o/r/"), "r");
        assert_eq!(repo_name(""), "workspace");
    }

    #[test]
    fn a_permanent_workspace_needs_no_run_id_and_a_disposable_one_does() {
        let permanent = Wanted::default();
        assert_eq!(
            generated(&permanent, "git@github.com:o/r.git", "").expect("un nom"),
            "r"
        );
        let disposable = Wanted {
            strategy: Strategy::Tmp,
            ..Wanted::default()
        };
        assert_eq!(
            generated(&disposable, "git@github.com:o/r.git", "20261002-1").expect("un nom"),
            "r-20261002-1"
        );
        // Sans identifiant, deux runs jetables simultanés viseraient le même
        // dossier, et le premier à finir supprimerait le workspace du second.
        assert!(generated(&disposable, "git@github.com:o/r.git", "").is_err());
    }
}
