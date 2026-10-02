//! The checkout a run works against, and where its accounting lands.
//!
//! **Two roots, not one**, since a run can work elsewhere than in the repo
//! from which it is launched: `root` carries **the code** — what sessions
//! edit, what `git` and `gh` see — and `state_root` carries **the accounting**:
//! logs, cost register, resume point. The two coincide as long as nobody asks
//! for a workspace.
//!
//! The gap between the two is what makes a workspace disposable: the code
//! folder disappears at the end of the run, and the cost register still answers.
//!
//! **Nothing here mounts anything: this module describes.** Cloning, resetting,
//! and deletion live in [`crate::execution::provisioning`], because they call
//! `git`.

use std::path::{Path, PathBuf};

/// Where workspaces land when no one decides otherwise.
///
/// Under `.llocal/`, so gitignored: a clone of the repo *within* the repo must
/// not show up in `git status`, or else the "clean tree" gate would refuse
/// every run from the second one on.
///
/// The name was **not** renamed with the rest (`pipeline:*` → `harness:*`): a
/// permanent workspace weighs hundreds of megabytes, and renaming it would
/// orphan one already on disk.
pub const DEFAULT_BASE: &str = ".llocal/agentic_workspaces";

/// How long a run's workspace survives the run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Strategy {
    /// One workspace per target repo, reused run to run and never deleted.
    /// Reset to `origin` before each run, so it does not drift.
    ///
    /// **The default, and it is a constraint, not a preference.** A clone carries
    /// only what git tracks: neither npm dependencies nor a venv, which are
    /// gitignored. Under [`Strategy::Tmp`] the workspace is deleted at every run
    /// end, so there is no moment to install them — and every stage check command
    /// would fail on a repo without dependencies. A permanent workspace keeps
    /// them: `reset --hard` and `clean -fd` return the code to `origin` without
    /// touching ignored files.
    #[default]
    Permanent,
    /// Disposable clone, deleted at the end of the run.
    Tmp,
}

impl Strategy {
    /// The strategy of this text, or `None` — **never a silent default**.
    ///
    /// A `WORKSPACE_STRATEGY=permanant` that fell back to `Tmp` would delete,
    /// once per run and without saying anything, the workspace the human thought
    /// they were keeping.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "permanent" => Some(Self::Permanent),
            "tmp" => Some(Self::Tmp),
            _ => None,
        }
    }

    /// How the strategy is written — in a variable, in a journal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Permanent => "permanent",
            Self::Tmp => "tmp",
        }
    }

    /// The accepted values, for an error message that helps.
    #[must_use]
    pub const fn known() -> &'static str {
        "permanent|tmp"
    }
}

/// The workspace a workflow asks for. A **declaration**, not a mount.
#[derive(Debug, Clone, Default)]
pub struct Wanted {
    /// What becomes of the folder.
    pub strategy: Strategy,
    /// The URL to clone. Empty: `origin` of the repo from which the run is launched.
    pub url: String,
    /// The folder name.
    ///
    /// Empty, it is derived from the repo in `Permanent` (one workspace per
    /// target, without anyone having to name it) and generated in `Tmp`. Filled,
    /// it is a human's `--use-workspace`: the workspace is then **found**, never
    /// recreated, and never deleted — naming a workspace to work in is a way of
    /// saying you do not want to lose it.
    pub id: String,
    /// The folder containing workspaces. Empty: [`DEFAULT_BASE`].
    pub base: String,
    /// `--keep-workspace`: keep a `Tmp` generated that the run would delete.
    pub keep: bool,
    /// `--force-reset`: overwrite local work of a reused workspace, instead of
    /// stopping and naming it.
    pub force_reset: bool,
}

/// A run's checkout, and where its accounting lands.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    state_root: PathBuf,
}

impl Workspace {
    /// Both roots at the same place — what a run without a workspace means.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            state_root: root.to_path_buf(),
        }
    }

    /// The same workspace, code taken elsewhere.
    ///
    /// The accounting stays where it was, and that is the whole point: that is what
    /// makes a code folder disposable.
    #[must_use]
    pub fn at(&self, root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            state_root: self.state_root.clone(),
        }
    }

    /// The code: what sessions edit, what `git` and `gh` see.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The accounting: logs, ledger, resume point.
    #[must_use]
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    /// The loop folder, under the accounting root.
    #[must_use]
    pub fn loop_dir(&self) -> PathBuf {
        self.state_root.join(".llocal/agent-loop")
    }

    /// The resume pointer, two lines.
    #[must_use]
    pub fn pointer(&self) -> PathBuf {
        self.loop_dir().join("state")
    }

    /// The cost ledger, whose header is frozen.
    #[must_use]
    pub fn ledger(&self) -> PathBuf {
        self.loop_dir().join("costs.tsv")
    }

    /// A review's folder: its locks, logs, artifacts.
    ///
    /// One folder for all reviews — their artifacts are
    /// prefixed by PR number, not by a run id.
    #[must_use]
    pub fn review_dir(&self) -> PathBuf {
        self.state_root.join(".llocal/pr-review")
    }

    /// The review ledger — columns distinct from the rounds one.
    #[must_use]
    pub fn review_ledger(&self) -> PathBuf {
        self.review_dir().join("costs.tsv")
    }

    /// Refinement's folder: its locks, artifacts, one subfolder
    /// per issue.
    #[must_use]
    pub fn refinement_dir(&self) -> PathBuf {
        self.state_root.join(".llocal/refinement")
    }

    /// The refinement ledger — same columns as the rounds one.
    #[must_use]
    pub fn refinement_ledger(&self) -> PathBuf {
        self.refinement_dir().join("costs.tsv")
    }

    /// Where workspaces this repo mounts land.
    #[must_use]
    pub fn workspaces(&self) -> PathBuf {
        self.state_root.join(DEFAULT_BASE)
    }

    /// This path, relative to one of the two roots when possible.
    ///
    /// **Code first, accounting last**, and order matters: the
    /// workspace lives *under* the accounting root when no one has said otherwise. Accounting
    /// first would have made every code path
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
