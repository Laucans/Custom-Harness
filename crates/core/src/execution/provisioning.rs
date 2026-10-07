//! Mount a run's workspace, and unmount it.
//!
//! What [`Wanted`] **declares**, this module **executes**: find or clone the
//! folder, return it to `origin` state, and delete it at the end when it was
//! disposable.
//!
//! # Three rules, each against a way to lose work
//!
//! - **Nothing is overwritten without saying so.** A reused workspace that
//!   carries a patch that `origin` doesn't have, a stash or a dirty tree stops
//!   the run by naming it, rather than running `reset --hard` on it.
//!   `force_reset` is the exit, and it's a human who takes it;
//! - **Nothing is deleted without saying so.** Same question at unmount: a
//!   disposable workspace that carries work that no one else has is kept, and
//!   its path printed;
//! - **A dry-run doesn't clone.** It says what it would have mounted, and the
//!   run works in place. Writing half a gigabyte to « call nothing » would be
//!   the most expensive contradiction in the package.
//!
//! # What the immutability of `Settings` removes
//!
//! On the Python side, mounting **rewrote `cfg.workspace`** in place, with the
//! comment that « a second path through which to pass the root would mean two
//! places to keep in sync ». Here it returns a [`Mount`], and the launcher
//! builds the `Settings` **after**: decision #6 means the hack is no longer
//! needed, and no one can rewrite a root under the feet of a stage in flight.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::domain::workspace::{Strategy, Wanted, Workspace};
use crate::domain::{Halt, Outcome, same_repo};
use crate::ports::shell::disk::Disk;
use crate::ports::shell::git::{Repo, Repos};
use crate::traces::Logbook;

/// What a mount gave, and what unmount must do with it.
#[derive(Debug, Clone)]
pub struct Mount {
    /// The run's workspace: its code, and its accounting left in place.
    pub workspace: Workspace,
    /// The folder name. Empty when the run works in place.
    pub name: String,
    /// The mounted folder. `None` when the run works in place — there's then
    /// nothing to unmount, and that's what `None` says here.
    pub path: Option<PathBuf>,
    /// The branch on which this workspace runs.
    ///
    /// Unmount needs it: « does this workspace carry work? » is compared to
    /// `origin/<it>`.
    pub branch: String,
    /// Deleted at unmount.
    ///
    /// False as soon as a human named this workspace or asked to keep it, and
    /// always false in [`Strategy::Permanent`].
    pub disposable: bool,
}

impl Mount {
    /// A run that works in place: nothing was mounted.
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

    /// True if something was mounted.
    #[must_use]
    pub const fn mounted(&self) -> bool {
        self.path.is_some()
    }
}

/// What mounts and unmounts: one `git` per repository, and one disk.
pub struct Provisioner {
    /// What opens a `git` on a given repository — the source, the parent
    /// folder, the clone.
    pub repos: Rc<dyn Repos>,
    /// What creates, lists and deletes.
    pub disk: Rc<dyn Disk>,
}

/// What a mount needs to know about the run that requests it.
pub struct Run<'a> {
    /// The workspace from which the run is launched: the clone source, and the
    /// accounting root.
    pub source: &'a Workspace,
    /// The workspace requested.
    pub wanted: &'a Wanted,
    /// The workflow's integration branch, if it has one.
    pub branch: &'a str,
    /// The run identifier, which names a disposable workspace.
    pub run_id: &'a str,
    /// Executes nothing, clones nothing.
    pub dry_run: bool,
}

impl Provisioner {
    /// The workspace this run demands, mounted.
    ///
    /// # Errors
    ///
    /// [`Halt::Halted`] whenever a human must act: no `origin` to clone, a
    /// workspace name that is a path, an unknown name, a folder that is not
    /// a workspace, a clone of another repo, work to save. [`Halt::Failed`]
    /// when `git` or disk did not respond.
    pub async fn mount(&self, run: &Run<'_>, log: &Logbook) -> Outcome<Mount> {
        let source = run.source;
        let wanted = run.wanted;
        let url = self.url_for(run).await?;
        let base = if wanted.base.is_empty() {
            source.workspaces()
        } else {
            // A relative path is read from the repo, never from the current
            // directory: the loop runs from anywhere — a cron, a hook, another
            // checkout — and a relative base would otherwise place a clone of
            // hundreds of megabytes where the shell happens to be.
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
            // In place, but **in the workspace a real run would use** when that
            // one is already on disk. Nothing is cloned, checked out, reset or
            // deleted — `path` stays `None`, so unmount does nothing, and the
            // "a dry-run doesn't clone" rule stands untouched.
            //
            // What changes is *which repository* everything downstream then
            // speaks about: the branch and CI gates, the installed-dependency
            // gate, the configuration digest, the signature index, and the
            // prompts the dry run writes out. Against the harness's own
            // checkout all six described the wrong repository, which made
            // `--dry-run` useless for the one thing it is for — reading the
            // prompt a real run would send.
            if self.disk.exists(&dest) {
                log.say(&format!(
                    "workspace: would use {} ({}, from {url}) — dry run reads it \
                     in place, mounting nothing",
                    source.rel(&dest),
                    wanted.strategy.as_str()
                ));
                return Ok(Mount::in_place(source.at(&dest)));
            }
            // Said rather than passed over: the gates and the prompt below are
            // then about the harness itself, and a reader who is not told will
            // take them for the target's.
            log.say(&format!(
                "workspace: would clone {} ({}, from {url}) — it is not on disk \
                 yet, so this dry run reads THIS checkout instead: the gates and \
                 prompts below describe the harness, not the target",
                source.rel(&dest),
                wanted.strategy.as_str()
            ));
            return Ok(Mount::in_place(source.clone()));
        }

        let branch = if self.disk.exists(&dest) {
            self.reuse(&dest, run, &url, &name, log).await?
        } else if named && !wanted.create_if_missing {
            // `--use-workspace` finds, it does not create: a typo would
            // otherwise make a fresh clone under a similar name, and the
            // workspace the human targeted would stay intact and unused. Under
            // `Permanent`, the extra clone would not even be deleted at the end,
            // so the typo would stay on disk. `create_if_missing` lifts this
            // for a name the harness chose itself, which no human could have
            // cloned beforehand.
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

        // A generated disposable is what gets deleted; naming a workspace or
        // asking to keep it amounts to the same, and `Permanent` is never deleted.
        let disposable = wanted.strategy == Strategy::Tmp && !named && !wanted.keep;
        // `source.rel`, not that of the mounted workspace: a path relative to
        // itself would be ".".
        log.say(&format!(
            "workspace: {} ({}, {})",
            source.rel(&dest),
            wanted.strategy.as_str(),
            if disposable { "disposable" } else { "kept" }
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

    /// Delete the workspace if it was disposable and carries nothing.
    ///
    /// **Returns nothing and cannot fail a run**: what happened to the
    /// workflow is already decided by the time we get here, and a folder that
    /// resists should not demote a successful run to failure. What could not
    /// be deleted is said, and stays.
    pub async fn unmount(&self, mount: &Mount, log: &Logbook) {
        let Some(path) = &mount.path else {
            return;
        };
        if !mount.disposable {
            return;
        }
        let git = self.repos.at(path);
        // The run may have just merged its own PR: without this `fetch`, the
        // guard judges against references frozen at mount, the round's branch
        // appears to carry work nobody has, and **every** disposable workspace
        // ends up kept — the disk bloat this path exists to prevent. A failed
        // fetch leaves the guard strict, which is the right sense of error.
        if !mount.branch.is_empty() {
            let _ = git.fetch().await;
        }
        let held = match self.what_is_held(path, git.as_ref(), &mount.branch).await {
            Ok(said) => said,
            // A read that does not succeed does not mean "nothing to lose".
            Err(why) => format!("its state could not be read ({})", why.reason()),
        };
        if !held.is_empty() {
            // What not to say here: "pick it back up with `--use-workspace`".
            // Mounting would ask the same question again, see the same work
            // and stop — and `--force-reset` would wipe it. The workspace is
            // kept to be **looked at**.
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

    // --- the three halves of mounting -----

    /// The URL to clone: what was asked, else the `origin` of the source repo.
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

    /// A fresh clone, placed on the branch the workflow works on.
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
                "{} has no parent directory",
                dest.display()
            )));
        };
        self.disk.create_dir_all(parent)?;
        log.say(&format!("cloning {url} -> {name}"));
        let done = self.repos.at(parent).clone_repo(url, name).await?;
        if !done.ok() {
            // A half-written clone is worse than none: the next run would
            // take it for a reusable workspace.
            let _ = self.disk.remove_dir_all(dest);
            return Err(Halt::Halted(format!("cannot clone {url}: {}", done.why())));
        }
        let ready = self
            .put_on_branch(self.repos.at(dest).as_ref(), branch, false, log)
            .await;
        if ready.is_err() {
            // Same reason, and this case is real: an integration branch that
            // does not exist on `origin` left a complete clone behind on each
            // try. Unmounting does not catch it — a failed mount has no `Mount`
            // to return.
            let _ = self.disk.remove_dir_all(dest);
        }
        ready
    }

    /// A workspace that is already there, reset to the state of `origin`.
    ///
    /// **Order matters, and it cost a run to find: `fetch` comes before the
    /// guard.** The guard asks whether a local branch carries a patch `origin`
    /// does not have — a question whose answer depends entirely on how fresh
    /// `origin/<branch>` is. Asking it first asked it against a stale reference,
    /// so a PR merged since the last run still counted as work to save, and
    /// the workspace would block for real.
    ///
    /// `fetch` destroys nothing. What destroys comes after the guard, and
    /// only after.
    async fn reuse(
        &self,
        dest: &Path,
        run: &Run<'_>,
        url: &str,
        name: &str,
        log: &Logbook,
    ) -> Outcome<String> {
        // Before everything else, and before `force_reset`: a folder we cannot
        // read does not reset. On the Python side this guard was *after*
        // `force`, meaning absent on the only destructive path.
        if let Some(wrong) = self.not_a_workspace(dest) {
            return Err(Halt::Halted(format!("the workspace {name:?}: {wrong}")));
        }
        let git = self.repos.at(dest);
        // Two different URLs can carry the same repo name, and it is this
        // name that names a permanent workspace. Without this test, the run
        // would work silently on the wrong repo — and push to it.
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

    /// Workspace on the workflow's branch, at the state of `origin`.
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
                // Returning success without resetting would make the run
                // work on the previous round's state, saying it is clean.
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

    /// The workflow's branch, else `origin/HEAD`, else the current one.
    ///
    /// The current branch as last resort: after a clone, `HEAD` is always
    /// somewhere, and a [`Mount`] without a branch falls back to unmount
    /// asking by SHA — the question that keeps everything.
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

    /// Local branches the previous round left behind, deleted.
    ///
    /// Without this, a permanent workspace blocks forever: the target repo
    /// merges by rebase, so a merged PR leaves a local branch whose commits
    /// are on no remote by SHA. `fetch --prune` then deletes its
    /// `origin/<branch>`, and the "unpushed work" guard starts reporting
    /// work already delivered — every run, with no exit.
    ///
    /// We only reach here **after** the guard: what remained to lose has
    /// already stopped the run, or `force_reset` said to overwrite it.
    async fn prune_branches(&self, git: &dyn Repo, branch: &str, log: &Logbook) -> Outcome<()> {
        for stale in git.local_branches().await? {
            if stale == branch {
                continue;
            }
            if git.delete_branch(&stale).await?.ok() {
                log.debug(&format!("workspace: local branch {stale} deleted"));
            }
        }
        Ok(())
    }

    // --- what we ask the disk before writing to it ---------------------------

    /// Why this folder is not a workspace, or `None`.
    ///
    /// Checked before everything else, including `force_reset`: we don't know
    /// what is in a folder placed there by hand, so we don't touch it. Reading
    /// it as an empty workspace is what we must never do.
    fn not_a_workspace(&self, dest: &Path) -> Option<&'static str> {
        if self.disk.exists(&dest.join(".git")) {
            return None;
        }
        Some("no .git — this is not a workspace this run made")
    }

    /// What this workspace holds that no one else has, stated in words.
    ///
    /// Empty when there is nothing to lose. **The question asked twice** —
    /// before a `reset --hard`, before deletion — and one answer,
    /// because both destroy exactly the same things.
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

    /// Warn that the checkout's local work is not in the workspace.
    ///
    /// The run clones `origin`: what is not pushed there does not exist for it.
    /// While the loop ran in place, the "clean tree" gate said it itself;
    /// it now looks at a clone, clean by construction. Without this line, a human
    /// who forgot to push sees a green run built on a state that is not theirs.
    ///
    /// The current branch only, three calls total. Sweeping all local branches
    /// would ask a question here whose answer depends on how fresh
    /// `origin/…` is — and we are not going to `fetch` in the human's
    /// repo just to write a warning.
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

/// The name of a workspace no one named.
///
/// `Permanent` draws one per target repo — two different URLs do not step on
/// each other, and the same URL finds its folder run after run, which is
/// the entire strategy. `Tmp` adds the run's identifier.
fn generated(wanted: &Wanted, url: &str, run_id: &str) -> Outcome<String> {
    let repo = repo_name(url);
    if wanted.strategy == Strategy::Permanent {
        return Ok(repo);
    }
    if run_id.is_empty() {
        // Without an identifier, two simultaneous disposable runs would target
        // the same folder, and the first to finish would delete the second's workspace.
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

/// One path segment, not one that goes up.
///
/// `base.join("../..")` resolves **outside** the workspaces folder, and
/// everything this module does next is destructive: without this test, a name
/// like `../..` would get `reset --hard`, `clean -fd` and `branch -D` — on
/// the human's checkout, without even needing `force_reset`, since the target
/// folder is indeed a repo and indeed the same `origin`.
fn is_a_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name).components().count() == 1
        && !Path::new(name).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::shell::process::Ran;
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::collections::HashSet;

    // --- fakes ---------------------------------------------------------------
    //
    // A fake disk and a fake `git`, not a real test folder: this
    // module is the only one in the package that deletes hundreds of megabytes, and
    // the tests that matter here are the ones that prove it **does not** delete.
    // Writing them against a real clone would be too much to ask.

    /// A `git` that responds what we told it, and records what we ask.
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
        /// The verbs that come back non-zero.
        failing: Vec<&'static str>,
        /// In call order.
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
                    format!("fatal: {verb} failed")
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

        async fn create_local_branch(&self, name: &str, from: &str) -> Outcome<Ran> {
            Ok(self.note(&format!("create_local_branch {name} {from}")))
        }

        async fn stage_all(&self) -> Outcome<Ran> {
            Ok(self.note("stage_all"))
        }

        async fn commit(&self, message: &str) -> Outcome<Ran> {
            Ok(self.note(&format!("commit {message}")))
        }

        async fn push(&self, branch: &str) -> Outcome<Ran> {
            Ok(self.note(&format!("push {branch}")))
        }
    }

    /// One fake `git` for any repo — what we exercise here is the
    /// policy, not routing.
    struct FakeRepos(Rc<FakeRepo>);

    impl Repos for FakeRepos {
        fn at(&self, _root: &Path) -> Rc<dyn Repo> {
            Rc::clone(&self.0) as Rc<dyn Repo>
        }
    }

    /// A disk in memory. What it deleted can be read back.
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

        fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
            unreachable!("provisioning never writes a file's content")
        }
    }

    // --- setup ---------------------------------------------------------------

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

    /// A disk where the workspace is already there, `.git` included.
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

    // --- rule 3: a dry-run clones nothing ---------------------------------

    #[tokio::test]
    async fn a_dry_run_clones_nothing_and_works_in_place() {
        // Writing half a gigabyte to "call nothing" would be the
        // most expensive contradiction in the package.
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default());
        let mount = mount_with(&git, &disk, &wanted(), true)
            .await
            .expect("mounted");
        assert!(!mount.mounted());
        assert_eq!(mount.workspace.root(), Path::new("/depot"));
        assert!(git.calls().is_empty(), "no git verbs");
        assert!(disk.created.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_dry_run_reads_the_workspace_a_real_run_would_use_when_it_is_there() {
        // The defect this fixes: a dry run read the *harness's own* checkout, so
        // the branch and CI gates, the installed-dependency gate, the
        // configuration digest, the signature index and the prompts it writes
        // all described the wrong repository — which is the one thing
        // `--dry-run` exists to let you read.
        let git = repo(clean_repo());
        let disk = Rc::new(disk_with_workspace());
        let mount = mount_with(&git, &disk, &wanted(), true)
            .await
            .expect("mounted");
        assert_eq!(mount.workspace.root(), Path::new(DEST));
        // Still mounts nothing: `path` is None, so unmount cannot reset or
        // delete a workspace a dry run only read.
        assert!(!mount.mounted());
        assert!(mount.branch.is_empty());
        assert!(!mount.disposable);
        assert!(git.calls().is_empty(), "no git verbs: nothing checked out");
        assert!(disk.created.borrow().is_empty());
        assert!(disk.removed().is_empty());
        // And the accounting stays home: a dry run must not write a ledger into
        // the target's checkout.
        assert_eq!(mount.workspace.state_root(), Path::new("/depot"));
    }

    // --- the name ----------------------------------------------------------

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
            .expect_err("must stop");
        assert!(err.reason().contains("is not a workspace name"));
        assert!(git.calls().is_empty(), "nothing was done to the repo");
        assert!(disk.removed().is_empty());
    }

    #[tokio::test]
    async fn a_named_workspace_is_found_never_created() {
        // A typo would otherwise make a fresh clone under a similar name, and the
        // target workspace would stay intact and unused beside it.
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
            .expect_err("must stop");
        assert!(err.reason().contains("no workspace named"));
        assert!(
            err.reason().contains("event_assistant"),
            "list what is there"
        );
        assert!(git.calls().is_empty(), "no clone");
    }

    #[tokio::test]
    async fn a_name_the_harness_chose_itself_is_cloned_when_it_is_missing() {
        // The other side of the same rule: a fixed id a workflow reserves for
        // its own checkout cannot pre-exist on a fresh machine, so refusing
        // would make it unreachable forever rather than catch a typo.
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default());
        let asked = Wanted {
            id: "router-readonly".to_string(),
            create_if_missing: true,
            ..wanted()
        };
        let mount = mount_with(&git, &disk, &asked, false)
            .await
            .expect("mounted");
        assert!(mount.workspace.root().ends_with("router-readonly"));
        assert!(
            git.calls().iter().any(|call| call.starts_with("clone")),
            "it clones instead of refusing: {:?}",
            git.calls()
        );
    }

    // --- rule 1: nothing is overwritten without saying so ----------------

    #[tokio::test]
    async fn a_reused_workspace_carrying_work_halts_instead_of_resetting() {
        let git = repo(FakeRepo {
            at_risk: vec!["fix/something".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let err = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect_err("must stop");
        assert!(err.reason().contains("fix/something"));
        assert!(err.reason().contains("--force-reset"), "name the exit");
        let calls = git.calls();
        assert!(
            !calls.iter().any(|c| c.starts_with("reset_hard")),
            "nothing was overwritten: {calls:?}"
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
            .expect_err("must stop");
        assert!(err.reason().contains("1 uncommitted change(s)"));
    }

    #[tokio::test]
    async fn the_fetch_comes_before_the_guard_so_it_judges_against_fresh_refs() {
        // Found at the cost of a run: the guard placed first judged against a
        // stale reference, so a PR merged since the last run still counted as
        // work to save — and the workspace blocked for real.
        let git = repo(FakeRepo {
            at_risk: vec!["feat/already-merged".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let _ = mount_with(&git, &disk, &wanted(), false).await;
        let calls = git.calls();
        let fetch = calls.iter().position(|c| c == "fetch").expect("a fetch");
        let guard = calls
            .iter()
            .position(|c| c == "branches_at_risk")
            .expect("the guard");
        assert!(fetch < guard, "{calls:?}");
    }

    #[tokio::test]
    async fn force_reset_overwrites_and_puts_the_workspace_back_on_origin() {
        let git = repo(FakeRepo {
            at_risk: vec!["fix/lost".to_string()],
            local: vec!["main_agent".to_string(), "fix/lost".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let asked = Wanted {
            force_reset: true,
            ..wanted()
        };
        let mount = mount_with(&git, &disk, &asked, false)
            .await
            .expect("mounted");
        assert_eq!(mount.branch, "main_agent");
        let calls = git.calls();
        assert!(calls.contains(&"checkout main_agent --force".to_string()));
        assert!(calls.contains(&"reset_hard origin/main_agent".to_string()));
        assert!(calls.contains(&"clean".to_string()));
        // The round's branch survives, the others are deleted — else a
        // permanent workspace blocks forever on a rebased PR.
        assert!(calls.contains(&"delete_branch fix/lost".to_string()));
        assert!(!calls.contains(&"delete_branch main_agent".to_string()));
    }

    #[tokio::test]
    async fn a_directory_without_git_is_refused_even_with_force_reset() {
        // The guard was behind `force` on the Python side, meaning it was absent
        // on the only destructive path. We don't know what is in a
        // folder placed there by hand, so we don't touch it.
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
            .expect_err("must stop");
        assert!(err.reason().contains("no .git"));
        assert!(git.calls().is_empty());
        assert!(disk.removed().is_empty());
    }

    #[tokio::test]
    async fn a_workspace_cloned_from_another_repository_is_refused() {
        // Without this test, the run would work silently on the wrong repo —
        // and push to it.
        let git = repo(FakeRepo {
            remote: "git@github.com:someone/event_assistant.git".to_string(),
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        // The target repo is named: two different URLs carry the same
        // repo name, and it is that name that names a permanent workspace.
        let asked = Wanted {
            url: ORIGIN.to_string(),
            ..wanted()
        };
        let err = mount_with(&git, &disk, &asked, false)
            .await
            .expect_err("must stop");
        assert!(err.reason().contains("is a clone of"));
        assert!(
            git.calls().is_empty(),
            "nothing before identifying the repo"
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

    // --- the clone ---------------------------------------------------------

    #[tokio::test]
    async fn a_fresh_clone_lands_on_the_integration_branch() {
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default());
        let mount = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect("mounted");
        assert_eq!(mount.path.as_deref(), Some(Path::new(DEST)));
        assert_eq!(mount.branch, "main_agent");
        assert!(!mount.disposable, "permanent is never deleted");
        let calls = git.calls();
        assert!(calls.contains(&"clone event_assistant".to_string()));
        // Without `--force`: there is nothing to overwrite in a fresh clone.
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
            .expect_err("must stop");
        assert!(err.reason().contains("cannot clone"));
        assert_eq!(disk.removed(), vec![PathBuf::from(DEST)]);
    }

    #[tokio::test]
    async fn a_clone_that_cannot_reach_the_branch_leaves_nothing_behind() {
        // The case is real: an integration branch missing from `origin`
        // left a complete clone behind it every try.
        let git = repo(FakeRepo {
            failing: vec!["checkout"],
            ..clean_repo()
        });
        let disk = Rc::new(FakeDisk::default());
        let err = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect_err("must stop");
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
            .expect_err("must stop");
        assert!(err.reason().contains("no `origin` remote"));
    }

    // --- rule 2: nothing is deleted without saying so ---------------------

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
        assert!(disk.removed().is_empty(), "kept to be looked at");
    }

    #[tokio::test]
    async fn the_unmount_fetches_first_so_a_just_merged_pr_is_not_mistaken_for_work() {
        // Without this fetch, **every** disposable workspace ends up kept —
        // the disk bloat this path exists to prevent.
        let git = repo(clean_repo());
        let disk = Rc::new(disk_with_workspace());
        unmount_with(&git, &disk, &disposable("main_agent")).await;
        let calls = git.calls();
        let fetch = calls.iter().position(|c| c == "fetch").expect("a fetch");
        let guard = calls
            .iter()
            .position(|c| c == "branches_at_risk")
            .expect("the guard");
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
        assert!(git.calls().is_empty(), "nothing is even asked of it");
    }

    #[tokio::test]
    async fn a_workspace_whose_state_cannot_be_read_is_kept_not_removed() {
        // A read that does not succeed does not mean "nothing to lose".
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default()); // no `.git`
        unmount_with(&git, &disk, &disposable("main_agent")).await;
        assert!(disk.removed().is_empty());
    }

    #[tokio::test]
    async fn without_a_branch_the_guard_falls_back_to_unpushed_commits() {
        let git = repo(FakeRepo {
            unpushed: vec!["abc1234 commit to nobody".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        unmount_with(&git, &disk, &disposable("")).await;
        assert!(disk.removed().is_empty());
    }

    // --- pure functions ---------------------------------------------------

    #[test]
    fn a_workspace_name_is_a_name_not_a_path() {
        // What this test prevents: `reset --hard` and `clean -fd` on the
        // human's checkout, without even a --force-reset.
        assert!(is_a_name("event_assistant"));
        assert!(!is_a_name("../.."));
        assert!(!is_a_name(".."));
        assert!(!is_a_name("."));
        assert!(!is_a_name(""));
        assert!(!is_a_name("a/b"));
        assert!(!is_a_name("/absolute"));
        assert!(!is_a_name("a\\b"));
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
            generated(&permanent, "git@github.com:o/r.git", "").expect("a name"),
            "r"
        );
        let disposable = Wanted {
            strategy: Strategy::Tmp,
            ..Wanted::default()
        };
        assert_eq!(
            generated(&disposable, "git@github.com:o/r.git", "20261002-1").expect("a name"),
            "r-20261002-1"
        );
        // Without an identifier, two simultaneous disposable runs would target the
        // same folder, and the first to finish would delete the second's workspace.
        assert!(generated(&disposable, "git@github.com:o/r.git", "").is_err());
    }
}
