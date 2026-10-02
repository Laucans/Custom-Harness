//! Le checkout contre lequel un run travaille, et où sa comptabilité tombe.
//!
//! **Deux racines et non une**, depuis qu'un run peut travailler ailleurs que
//! dans le dépôt d'où il est lancé : `root` porte **le code** — ce que les
//! sessions éditent, ce que `git` et `gh` voient — et `state_root` porte **la
//! comptabilité** : journaux, registre des coûts, point de reprise. Les deux
//! coïncident tant que personne ne demande de workspace.
//!
//! L'écart entre les deux est ce qui rend un workspace jetable : le dossier de
//! code disparaît à la fin du run, et le registre des coûts répond toujours.
//!
//! **Rien ici ne monte quoi que ce soit : ce module décrit.** Le clone, la
//! remise à zéro et la suppression vivent dans
//! [`crate::execution::provisioning`], parce qu'ils appellent `git`.

use std::path::{Path, PathBuf};

/// Où tombent les workspaces quand personne n'en décide autrement.
///
/// Sous `.llocal/`, donc gitignoré : un clone du dépôt *dans* le dépôt ne doit
/// pas se voir dans `git status`, sans quoi la porte « arbre propre »
/// refuserait tout run dès le second.
///
/// Le nom n'a **pas** été renommé avec le reste (`pipeline:*` → `harness:*`) :
/// un workspace permanent pèse quelques centaines de mégaoctets, et le
/// renommer en orphelinerait un qui est déjà sur le disque.
pub const DEFAULT_BASE: &str = ".llocal/agentic_workspaces";

/// Combien de temps le workspace d'un run survit au run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Strategy {
    /// Un workspace par dépôt cible, réutilisé d'un run à l'autre et jamais
    /// supprimé. Remis à l'état d'`origin` avant chaque run, donc il ne dérive
    /// pas.
    ///
    /// **Le défaut, et c'est une contrainte, pas une préférence.** Un clone ne
    /// porte que ce que git suit : ni les dépendances npm, ni un venv, qui sont
    /// gitignorés. Sous [`Strategy::Tmp`] le workspace est supprimé à chaque
    /// fin de run, donc il n'existe aucun moment où les installer — et chaque
    /// commande de vérification d'un stage échouerait sur un dépôt sans
    /// dépendances. Un workspace permanent les garde : `reset --hard` et
    /// `clean -fd` remettent le code à l'état d'`origin` sans toucher aux
    /// fichiers ignorés.
    #[default]
    Permanent,
    /// Clone jetable, supprimé à la fin du run.
    Tmp,
}

impl Strategy {
    /// La stratégie de ce texte, ou `None` — **jamais un défaut silencieux**.
    ///
    /// Un `WORKSPACE_STRATEGY=permanant` qui retomberait sur `Tmp` ferait
    /// supprimer, une fois par run et sans rien dire, le workspace que l'humain
    /// croyait garder.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "permanent" => Some(Self::Permanent),
            "tmp" => Some(Self::Tmp),
            _ => None,
        }
    }

    /// Comment la stratégie s'écrit — dans une variable, dans un journal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Permanent => "permanent",
            Self::Tmp => "tmp",
        }
    }

    /// Les valeurs acceptées, pour un message d'erreur qui aide.
    #[must_use]
    pub const fn known() -> &'static str {
        "permanent|tmp"
    }
}

/// Le workspace qu'un workflow demande. Une **déclaration**, pas un montage.
#[derive(Debug, Clone, Default)]
pub struct Wanted {
    /// Ce qu'il advient du dossier.
    pub strategy: Strategy,
    /// L'URL à cloner. Vide : l'`origin` du dépôt d'où le run est lancé.
    pub url: String,
    /// Le nom du dossier.
    ///
    /// Vide, il est dérivé du dépôt en `Permanent` (un workspace par cible,
    /// sans que personne ait à le nommer) et engendré en `Tmp`. Rempli, c'est
    /// le `--use-workspace` d'un humain : le workspace est alors **retrouvé**,
    /// jamais recréé, et jamais supprimé — nommer un workspace pour y
    /// travailler est une façon de dire qu'on ne veut pas le perdre.
    pub id: String,
    /// Le dossier qui contient les workspaces. Vide : [`DEFAULT_BASE`].
    pub base: String,
    /// `--keep-workspace` : garder un `Tmp` engendré que le run supprimerait.
    pub keep: bool,
    /// `--force-reset` : écraser le travail local d'un workspace réutilisé, au
    /// lieu de s'arrêter en le nommant.
    pub force_reset: bool,
}

/// Le checkout d'un run, et où sa comptabilité tombe.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    state_root: PathBuf,
}

impl Workspace {
    /// Les deux racines au même endroit — ce qu'un run sans workspace veut
    /// dire.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            state_root: root.to_path_buf(),
        }
    }

    /// Le même workspace, le code pris ailleurs.
    ///
    /// La comptabilité reste où elle était, et c'est tout l'intérêt : c'est ce
    /// qui rend un dossier de code jetable.
    #[must_use]
    pub fn at(&self, root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            state_root: self.state_root.clone(),
        }
    }

    /// Le code : ce que les sessions éditent, ce que `git` et `gh` voient.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// La comptabilité : journaux, registre, point de reprise.
    #[must_use]
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    /// Le dossier de la boucle, sous la racine de comptabilité.
    #[must_use]
    pub fn loop_dir(&self) -> PathBuf {
        self.state_root.join(".llocal/agent-loop")
    }

    /// Le pointeur de reprise, deux lignes.
    #[must_use]
    pub fn pointer(&self) -> PathBuf {
        self.loop_dir().join("state")
    }

    /// Le registre des coûts, dont l'en-tête est gelé.
    #[must_use]
    pub fn ledger(&self) -> PathBuf {
        self.loop_dir().join("costs.tsv")
    }

    /// Le dossier d'une revue : ses verrous, son journal, ses artefacts.
    ///
    /// Un seul dossier pour toutes les revues — leurs artefacts sont
    /// préfixés par le numéro de la PR, pas par un identifiant de run.
    #[must_use]
    pub fn review_dir(&self) -> PathBuf {
        self.state_root.join(".llocal/pr-review")
    }

    /// Le registre des revues — colonnes distinctes de celui des rounds.
    #[must_use]
    pub fn review_ledger(&self) -> PathBuf {
        self.review_dir().join("costs.tsv")
    }

    /// Le dossier du raffinage : ses verrous, ses artefacts, un sous-dossier
    /// par issue.
    #[must_use]
    pub fn refinement_dir(&self) -> PathBuf {
        self.state_root.join(".llocal/refinement")
    }

    /// Le registre du raffinage — mêmes colonnes que celui des rounds.
    #[must_use]
    pub fn refinement_ledger(&self) -> PathBuf {
        self.refinement_dir().join("costs.tsv")
    }

    /// Où tombent les workspaces que ce dépôt monte.
    #[must_use]
    pub fn workspaces(&self) -> PathBuf {
        self.state_root.join(DEFAULT_BASE)
    }

    /// Ce chemin, relatif à l'une des deux racines quand c'est possible.
    ///
    /// **Le code d'abord, la comptabilité ensuite**, et l'ordre compte : le
    /// workspace vit *sous* la racine de comptabilité quand personne n'a dit le
    /// contraire. La comptabilité d'abord rendait donc tout chemin de code
    /// précédé de `.llocal/agentic_workspaces/<nom>/`, et la racine du code
    /// n'était jamais atteinte.
    #[must_use]
    pub fn rel(&self, path: &Path) -> String {
        for base in [&self.root, &self.state_root] {
            if let Ok(inside) = path.strip_prefix(base) {
                return inside.display().to_string();
            }
        }
        path.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_misspelled_strategy_is_none_rather_than_a_silent_default() {
        // Retomber sur Tmp ferait supprimer, une fois par run et sans rien
        // dire, le workspace que l'humain croyait garder.
        assert_eq!(Strategy::parse("permanant"), None);
        assert_eq!(Strategy::parse(""), None);
        assert_eq!(Strategy::parse(" TMP "), Some(Strategy::Tmp));
        assert_eq!(Strategy::parse("permanent"), Some(Strategy::Permanent));
    }

    #[test]
    fn permanent_is_the_default_because_it_deletes_nothing() {
        assert_eq!(Strategy::default(), Strategy::Permanent);
        assert_eq!(Strategy::default().as_str(), "permanent");
    }

    #[test]
    fn without_a_workspace_both_roots_are_the_same_place() {
        let here = Workspace::new(Path::new("/depot"));
        assert_eq!(here.root(), Path::new("/depot"));
        assert_eq!(here.state_root(), Path::new("/depot"));
    }

    #[test]
    fn moving_the_code_leaves_the_bookkeeping_where_it_was() {
        // C'est ce qui rend un workspace jetable : le dossier de code
        // disparaît, `costs.tsv` répond toujours.
        let moved = Workspace::new(Path::new("/depot")).at(Path::new("/depot/.llocal/w/clone"));
        assert_eq!(moved.root(), Path::new("/depot/.llocal/w/clone"));
        assert_eq!(
            moved.ledger(),
            Path::new("/depot/.llocal/agent-loop/costs.tsv")
        );
    }

    #[test]
    fn a_code_path_is_relative_to_the_code_root_not_to_the_state_root() {
        // Le mode de panne que ça évite : tout chemin de code affiché précédé
        // de `.llocal/agentic_workspaces/<nom>/`.
        let moved = Workspace::new(Path::new("/depot")).at(Path::new("/depot/.llocal/w/clone"));
        assert_eq!(
            moved.rel(Path::new("/depot/.llocal/w/clone/src/main.rs")),
            "src/main.rs"
        );
    }

    #[test]
    fn review_and_refinement_live_under_their_own_llocal_dirs() {
        let here = Workspace::new(Path::new("/depot"));
        assert_eq!(here.review_dir(), Path::new("/depot/.llocal/pr-review"));
        assert_eq!(
            here.review_ledger(),
            Path::new("/depot/.llocal/pr-review/costs.tsv")
        );
        assert_eq!(
            here.refinement_dir(),
            Path::new("/depot/.llocal/refinement")
        );
        assert_eq!(
            here.refinement_ledger(),
            Path::new("/depot/.llocal/refinement/costs.tsv")
        );
    }

    #[test]
    fn a_bookkeeping_path_is_still_relative_to_the_state_root() {
        let moved = Workspace::new(Path::new("/depot")).at(Path::new("/clone"));
        assert_eq!(moved.rel(&moved.ledger()), ".llocal/agent-loop/costs.tsv");
    }

    #[test]
    fn a_path_under_neither_root_comes_back_as_it_is() {
        let here = Workspace::new(Path::new("/depot"));
        assert_eq!(here.rel(Path::new("/ailleurs/x")), "/ailleurs/x");
    }
}
