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
//! One exception to the first rule, for the checkouts only the harness works
//! in (`Wanted::drop_uncommitted`): uncommitted changes there are a
//! session's leftovers, dropped and said rather than stopped on — a lane
//! that stops on a leftover stops for every job after. Commits no remote
//! has stay under the rule. The same question is asked at [`Provisioner::release`],
//! when a run gives its checkout back: left clean, or kept whole and said.
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
use crate::domain::{Halt, Outcome, clone_url, same_repo};
use crate::ports::shell::disk::Disk;
use crate::ports::shell::git::{Repo, Repos};
use crate::traces::{Event, Logbook};

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
        let base = source.workspaces_in(&wanted.base);
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
        self.say_what_stays_behind(source, &url, log).await;
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
            Ok(held) => held.said(),
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
            return Ok(clone_url(&run.wanted.url));
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
        // A previous run may have narrowed the tree to its task's closure;
        // everything below — the guard, the digest, the index, the gates —
        // reads the whole checkout, so it is widened first.
        let widened = git.sparse_checkout_off().await?;
        if !widened.ok() {
            return Err(Halt::Halted(format!(
                "cannot widen the workspace {name:?} back to the whole tree: {}",
                widened.why()
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
            if held.work() || (held.dirty > 0 && !run.wanted.drop_uncommitted) {
                return Err(Halt::Halted(format!(
                    "the workspace {name:?} carries {} — look at {}, or \
                     re-run with --force-reset to overwrite it",
                    held.said(),
                    run.source.rel(dest)
                )));
            }
            if held.dirty > 0 {
                // A checkout only the harness works in: the dirt is a
                // session's leftovers, and the guard above saw that nothing
                // under it is unpushed.
                log.say(&format!(
                    "workspace {name}: {} uncommitted change(s) a previous run \
                     left — dropped, nothing on it is unpushed",
                    held.dirty
                ));
            }
            drop_stashes(git.as_ref(), name, held.stashes, log).await;
        }
        self.put_on_branch(git.as_ref(), run.branch, true, log)
            .await
    }

    /// Puts a checkout back as the next run wants to find it, once its run
    /// is done — then [`Self::unmount`].
    ///
    /// A session leaves what it leaves: a merge it began and never finished,
    /// files it never committed, a branch it made. The next run on a
    /// checkout the harness names would stop on all of it, and a lane that
    /// stops on a leftover stops for every job after (`ForgeDesMondes`,
    /// 23:09Z: a repair left a conflicted merge on its lane, and the review
    /// that followed there halted on "2 uncommitted change(s)"). So the
    /// merge is aborted, the branch put back at `origin`'s state, the
    /// untracked files and the scratch branches removed.
    ///
    /// **Unless the checkout holds commits no remote has.** Those are work
    /// — a repair that never got pushed — and are kept whole, said as an
    /// [`Event::WorkKept`]; the next run on this checkout then stops and
    /// names them, which is the right kind of stop. Only the merge is still
    /// aborted: what it merged is on `origin`, and a half-merge over the
    /// commits is noise for whoever comes to look.
    ///
    /// Cannot fail a run, like [`Self::unmount`]: what could not be done is
    /// said, and the next mount says it again.
    pub async fn release(&self, mount: &Mount, log: &Logbook) {
        if let Some(path) = &mount.path
            && !mount.branch.is_empty()
        {
            self.tidy(path, mount, log).await;
        }
        self.unmount(mount, log).await;
    }

    /// The release's work on a mounted checkout with a branch: see
    /// [`Self::release`].
    async fn tidy(&self, path: &Path, mount: &Mount, log: &Logbook) {
        let git = self.repos.at(path);
        abort_merge(git.as_ref(), &mount.name, log).await;
        // Fresh references, so what the run pushed is not read as unpushed.
        let _ = git.fetch().await;
        let held = match self.what_is_held(path, git.as_ref(), &mount.branch).await {
            Ok(held) => held,
            Err(why) => {
                log.warn(&format!(
                    "workspace {}: its state could not be read ({}) — left as it is",
                    mount.name,
                    why.reason()
                ));
                return;
            }
        };
        if let Some(wrong) = held.wrong {
            log.warn(&format!(
                "workspace {}: {wrong} — left as it is",
                mount.name
            ));
            return;
        }
        if held.work() {
            log.event(&Event::WorkKept {
                workspace: mount.name.clone(),
                path: path.display().to_string(),
                branch: mount.branch.clone(),
                branches: held.branches.clone(),
                commits: count(held.commits.len()),
                stashes: count(held.stashes),
            });
            return;
        }
        if held.dirty > 0 {
            log.say(&format!(
                "workspace {}: {} uncommitted change(s) the run left — dropped, \
                 nothing on it is unpushed",
                mount.name, held.dirty
            ));
        }
        drop_stashes(git.as_ref(), &mount.name, held.stashes, log).await;
        match self
            .put_on_branch(git.as_ref(), &mount.branch, true, log)
            .await
        {
            Ok(_) => log.debug(&format!(
                "workspace {}: back on origin/{}, clean",
                mount.name, mount.branch
            )),
            Err(why) => log.warn(&format!(
                "workspace {}: cannot be put back on origin/{} ({}) — the next \
                 run on it will say so",
                mount.name,
                mount.branch,
                why.reason()
            )),
        }
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
    /// merges by squash, so a merged PR leaves a local branch whose commits
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

    /// What this workspace holds that no one else has.
    ///
    /// Empty when there is nothing to lose. **The question asked three
    /// times** — before a `reset --hard`, before deletion, at a release —
    /// and one answer, because all three destroy exactly the same things.
    async fn what_is_held(&self, dest: &Path, git: &dyn Repo, branch: &str) -> Outcome<Held> {
        if let Some(wrong) = self.not_a_workspace(dest) {
            return Ok(Held {
                wrong: Some(wrong),
                ..Held::default()
            });
        }
        let mut held = Held {
            stashes: git.stashes().await?.len(),
            ..Held::default()
        };
        if branch.is_empty() {
            held.commits = git.unpushed().await?;
            if !held.commits.is_empty() {
                let current = git.current_branch().await?;
                held.branches = vec![if current.is_empty() {
                    "HEAD".to_string()
                } else {
                    current
                }];
            }
        } else {
            held.branches = git.branches_at_risk(&format!("origin/{branch}")).await?;
            if !held.branches.is_empty() {
                held.commits = git.unpushed().await?;
            }
        }
        held.dirty = git.dirty_files().await?.len();
        Ok(held)
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
    ///
    /// Only when the workspace clones **this checkout's own repository**: a
    /// harness driving another repository clones that one, and the
    /// checkout's local work was never meant to be in it.
    async fn say_what_stays_behind(&self, source: &Workspace, url: &str, log: &Logbook) {
        let held = self.left_behind(source, url).await;
        if !held.is_empty() {
            log.warn(&format!(
                "your checkout holds {} — the workspace is cloned from origin, \
                 so none of it is in this run",
                held.join(", ")
            ));
        }
    }
}

impl Provisioner {
    /// What the checkout holds that a clone of `url` will not: its
    /// uncommitted changes and its unpushed commits — nothing when `url` is
    /// another repository than the checkout's `origin`.
    async fn left_behind(&self, source: &Workspace, url: &str) -> Vec<String> {
        let git = self.repos.at(source.root());
        let origin = git.remote_url("origin").await.unwrap_or_default();
        if !same_repo(&origin, url) {
            return Vec::new();
        }
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
        held
    }
}

/// What a checkout holds that no one else has — the answer to the question
/// asked before a `reset --hard`, before a deletion, and at a release.
#[derive(Debug, Default)]
struct Held {
    /// Why the folder is no workspace at all; nothing else is read then.
    wrong: Option<&'static str>,
    /// Entries of `stash list`.
    stashes: usize,
    /// The local branches whose patch origin does not have — the current
    /// one when the run has no branch and commits are unpushed.
    branches: Vec<String>,
    /// The commits no remote has, one line each.
    commits: Vec<String>,
    /// Uncommitted changes, one per file.
    dirty: usize,
}

impl Held {
    /// Work a reset or a deletion would lose for good: a commit no remote
    /// has — or a folder that cannot be read, which counts as such. A stash
    /// is none: a session stashed and went, nobody will pop it, and a lane
    /// that held it stopped every read-only job after (`ForgeDesMondes`,
    /// 2026-10-20 03:23Z: a repair's stash from two days before).
    const fn work(&self) -> bool {
        self.wrong.is_some() || !self.branches.is_empty()
    }

    /// In words, for a halt or a warning; empty when nothing is held.
    fn said(&self) -> String {
        if let Some(wrong) = self.wrong {
            return wrong.to_string();
        }
        let mut said = Vec::new();
        if self.stashes > 0 {
            said.push(format!("{} stash entry(ies)", self.stashes));
        }
        if !self.branches.is_empty() {
            said.push(format!(
                "{} unpushed commit(s) on {}",
                self.commits.len(),
                self.branches
                    .iter()
                    .take(3)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.dirty > 0 {
            said.push(format!("{} uncommitted change(s)", self.dirty));
        }
        said.join(", ")
    }
}

/// Drops the stash entries a session left: leftovers, not work (see
/// [`Held::work`]). Nothing to do, nothing said.
async fn drop_stashes(git: &dyn Repo, name: &str, stashes: usize, log: &Logbook) {
    if stashes == 0 {
        return;
    }
    match git.clear_stashes().await {
        Ok(done) if done.ok() => log.say(&format!(
            "workspace {name}: {stashes} stash entry(ies) a session left — dropped"
        )),
        Ok(done) => log.warn(&format!(
            "workspace {name}: cannot drop its {stashes} stash entry(ies) ({})",
            done.why()
        )),
        Err(why) => log.warn(&format!(
            "workspace {name}: cannot drop its {stashes} stash entry(ies) ({})",
            why.reason()
        )),
    }
}

/// Aborts the merge a session began and never finished. Never work: what
/// it merged is still on `origin` and can be merged again, and a conflicted
/// index is what the next run on the checkout would stop on.
async fn abort_merge(git: &dyn Repo, name: &str, log: &Logbook) {
    match git.merge_in_progress().await {
        Ok(false) => {}
        Ok(true) => match git.abort_merge().await {
            Ok(done) if done.ok() => {
                log.say(&format!("workspace {name}: an unfinished merge aborted"));
            }
            Ok(done) => log.warn(&format!(
                "workspace {name}: cannot abort its unfinished merge ({})",
                done.why()
            )),
            Err(why) => log.warn(&format!(
                "workspace {name}: cannot abort its unfinished merge ({})",
                why.reason()
            )),
        },
        Err(why) => log.warn(&format!(
            "workspace {name}: cannot tell whether a merge is unfinished ({})",
            why.reason()
        )),
    }
}

/// A count as an event carries it.
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
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
    use crate::traces::{Sink, Verbosity};
    use async_trait::async_trait;
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    // --- fakes ---------------------------------------------------------------
    //
    // A fake disk and a fake `git`, not a real test folder: this
    // module is the only one in the package that deletes hundreds of megabytes, and
    // the tests that matter here are the ones that prove it **does not** delete.
    // Writing them against a real clone would be too much to ask.

    /// A `git` that responds what we told it, and records what we ask.
    ///
    /// Its tree has a state, as far as a release needs one: a merge in
    /// progress until aborted or reset, dirt until reset or cleaned — so a
    /// release followed by a mount reads what the release left.
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
        /// A merge begun and not finished.
        merging: Cell<bool>,
        /// `dirty` was wiped by a reset or a clean.
        swept: Cell<bool>,
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
            if self.swept.get() {
                return Ok(Vec::new());
            }
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
            let done = self.note(&format!("reset_hard {reference}"));
            if done.ok() {
                self.swept.set(true);
                self.merging.set(false);
            }
            Ok(done)
        }

        async fn clean(&self) -> Outcome<Ran> {
            let done = self.note("clean");
            if done.ok() {
                self.swept.set(true);
            }
            Ok(done)
        }

        async fn merge_in_progress(&self) -> Outcome<bool> {
            Ok(self.merging.get())
        }

        async fn abort_merge(&self) -> Outcome<Ran> {
            let done = self.note("abort_merge");
            if done.ok() {
                self.merging.set(false);
            }
            Ok(done)
        }

        async fn clear_stashes(&self) -> Outcome<Ran> {
            Ok(self.note("clear_stashes"))
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

    /// A journal that keeps its lines and its events, to be read back.
    #[derive(Default)]
    struct Journal {
        lines: RefCell<Vec<String>>,
        events: RefCell<Vec<Event>>,
    }

    impl Sink for Journal {
        fn emit(&self, line: &str) {
            self.lines.borrow_mut().push(line.to_string());
        }

        fn record(&self, event: &Event) {
            self.events.borrow_mut().push(event.clone());
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

    #[tokio::test]
    async fn a_git_that_does_not_say_how_it_removes_a_path_refuses_to() {
        // The port's default: only a real `git` and the fakes that test a
        // catch-up remove a conflicted path.
        let refused = FakeRepo::default().remove("gen.js").await.expect("ran");
        assert!(!refused.ok());
        assert_eq!(refused.stderr, "this git cannot remove gen.js");
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
        mount_logged(git, disk, wanted, dry_run, &Logbook::null()).await
    }

    async fn mount_logged(
        git: &Rc<FakeRepo>,
        disk: &Rc<FakeDisk>,
        wanted: &Wanted,
        dry_run: bool,
        log: &Logbook,
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
                log,
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
        assert!(disk.created.borrow().is_empty(), "expected nothing");
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
        assert_eq!(mount.branch, "");
        assert!(!mount.disposable);
        assert!(git.calls().is_empty(), "no git verbs: nothing checked out");
        assert!(disk.created.borrow().is_empty(), "expected nothing");
        assert_eq!(disk.removed(), [] as [std::path::PathBuf; 0]);
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
        assert_eq!(disk.removed(), [] as [std::path::PathBuf; 0]);
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
            unpushed: vec!["abc1234 the fix".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        let err = mount_with(&git, &disk, &wanted(), false)
            .await
            .expect_err("must stop");
        assert_eq!(
            err.reason(),
            "the workspace \"event_assistant\" carries 1 unpushed commit(s) on fix/something \
             — look at .llocal/agentic_workspaces/event_assistant, or re-run with \
             --force-reset to overwrite it",
            "the branch, how many, and the exit"
        );
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
        // permanent workspace blocks forever on a squashed PR.
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
        assert_eq!(git.calls(), [] as [std::string::String; 0]);
        assert_eq!(disk.removed(), [] as [std::path::PathBuf; 0]);
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
        assert_eq!(disk.removed(), [] as [std::path::PathBuf; 0]);
        assert!(git.calls().is_empty(), "nothing is even asked of it");
    }

    #[tokio::test]
    async fn a_workspace_whose_state_cannot_be_read_is_kept_not_removed() {
        // A read that does not succeed does not mean "nothing to lose".
        let git = repo(clean_repo());
        let disk = Rc::new(FakeDisk::default()); // no `.git`
        unmount_with(&git, &disk, &disposable("main_agent")).await;
        assert_eq!(disk.removed(), [] as [std::path::PathBuf; 0]);
    }

    #[tokio::test]
    async fn local_work_is_only_reported_when_the_clone_is_of_this_repository() {
        let git = Rc::new(FakeRepo {
            remote: "https://github.com/Laucans/Custom-Harness".to_string(),
            unpushed: vec!["abc1234 not on origin".to_string()],
            ..FakeRepo::default()
        });
        let disk = Rc::new(FakeDisk::default());
        let prov = provisioner(&git, disk);
        let source = Workspace::new(Path::new("/src"));
        let other = prov.left_behind(&source, "Laucans/dnd_helper").await;
        assert!(other.is_empty(), "{other:?}");
        let own = prov
            .left_behind(&source, "git@github.com:Laucans/Custom-Harness.git")
            .await;
        assert_eq!(own.len(), 1, "{own:?}");
        assert!(own[0].contains("not pushed"));
    }

    #[tokio::test]
    async fn without_a_branch_the_guard_falls_back_to_unpushed_commits() {
        let git = repo(FakeRepo {
            unpushed: vec!["abc1234 commit to nobody".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_workspace());
        unmount_with(&git, &disk, &disposable("")).await;
        assert_eq!(disk.removed(), [] as [std::path::PathBuf; 0]);
    }

    // --- the release: a checkout given back clean, or kept whole ---------

    /// Where a lane's checkout lives, `.git` included.
    const LANE: &str = "/depot/.llocal/agentic_workspaces/router-lane-1";

    fn disk_with_lane() -> FakeDisk {
        FakeDisk {
            there: [PathBuf::from(LANE), PathBuf::from(LANE).join(".git")]
                .into_iter()
                .collect(),
            ..FakeDisk::default()
        }
    }

    /// A lane's checkout: permanent, named by the harness, on the branch
    /// the run worked on.
    fn lane(branch: &str) -> Mount {
        Mount {
            workspace: source().at(Path::new(DEST)),
            name: "router-lane-1".to_string(),
            path: Some(PathBuf::from(LANE)),
            branch: branch.to_string(),
            disposable: false,
        }
    }

    /// What a read-only tenant of a lane asks for: no `force_reset`, and
    /// the leftovers of a session dropped.
    fn lane_wanted() -> Wanted {
        Wanted {
            id: "router-lane-1".to_string(),
            create_if_missing: true,
            drop_uncommitted: true,
            ..wanted()
        }
    }

    fn journal() -> (Rc<Journal>, Logbook) {
        let journal = Rc::new(Journal::default());
        let log = Logbook::new(Rc::clone(&journal) as Rc<dyn Sink>, Verbosity::Normal);
        (journal, log)
    }

    /// The lane as the repair left it on `ForgeDesMondes` (23:09Z): a merge
    /// of the milestone branch stopped on a conflict, files it added, a
    /// folder it never committed — and nothing unpushed.
    fn conflicted_lane() -> FakeRepo {
        FakeRepo {
            dirty: vec![
                "UU crates/dataguard/server/src/lib.rs".to_string(),
                "A  crates/dataguard/server/src/diffusion.rs".to_string(),
                "?? scratch/".to_string(),
            ],
            merging: Cell::new(true),
            local: vec![
                "feat/diffusion-aggregate".to_string(),
                "milestone/18-campaigns".to_string(),
            ],
            ..clean_repo()
        }
    }

    fn position(calls: &[String], verb: &str) -> usize {
        calls
            .iter()
            .position(|c| c == verb)
            .unwrap_or_else(|| panic!("{verb} was not asked: {calls:?}"))
    }

    #[tokio::test]
    async fn a_run_that_left_a_conflicted_merge_gives_its_checkout_back_clean() {
        // The review that took the lane next halted on "2 uncommitted
        // change(s)": the unmount only forced a checkout, which left the
        // untracked files, and never said what it dropped.
        let git = repo(conflicted_lane());
        let disk = Rc::new(disk_with_lane());
        let (journal, log) = journal();
        provisioner(&git, Rc::clone(&disk))
            .release(&lane("feat/diffusion-aggregate"), &log)
            .await;
        let calls = git.calls();
        let abort = position(&calls, "abort_merge");
        let fetch = position(&calls, "fetch");
        let guard = position(&calls, "branches_at_risk");
        let reset = position(&calls, "reset_hard origin/feat/diffusion-aggregate");
        assert!(
            abort < fetch && fetch < guard && guard < reset,
            "the merge first, fresh refs before the guard, the guard before \
             anything destructive: {calls:?}"
        );
        assert!(calls.contains(&"checkout feat/diffusion-aggregate --force".to_string()));
        assert!(calls.contains(&"clean".to_string()));
        assert!(calls.contains(&"delete_branch milestone/18-campaigns".to_string()));
        assert!(!calls.contains(&"delete_branch feat/diffusion-aggregate".to_string()));
        assert!(!git.merging.get(), "the merge is gone");
        assert!(
            git.dirty_files().await.expect("read").is_empty(),
            "the checkout is clean"
        );
        assert!(journal.events.borrow().is_empty(), "nothing was kept");
        let lines = journal.lines.borrow();
        assert!(
            lines
                .iter()
                .any(|l| l == "workspace router-lane-1: an unfinished merge aborted"),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l
                == "workspace router-lane-1: 3 uncommitted change(s) the run left — \
                    dropped, nothing on it is unpushed"),
            "{lines:?}"
        );
        assert_eq!(
            disk.removed(),
            [] as [PathBuf; 0],
            "permanent: never deleted"
        );
    }

    #[tokio::test]
    async fn the_next_read_only_tenant_mounts_a_lane_a_release_left_clean() {
        let git = repo(conflicted_lane());
        let disk = Rc::new(disk_with_lane());
        provisioner(&git, Rc::clone(&disk))
            .release(&lane("feat/diffusion-aggregate"), &Logbook::null())
            .await;
        let (journal, log) = journal();
        let mount = mount_logged(&git, &disk, &lane_wanted(), false, &log)
            .await
            .expect("the lane is clean: the review mounts it");
        assert_eq!(mount.branch, "main_agent");
        assert_eq!(mount.name, "router-lane-1");
        assert!(
            !journal.lines.borrow().iter().any(|l| l.contains("dropped")),
            "nothing to drop, nothing said: {:?}",
            journal.lines.borrow()
        );
    }

    #[tokio::test]
    async fn a_lane_dirty_with_nothing_unpushed_is_reset_at_mount_and_says_so() {
        // A killed run never releases: the next tenant meets the dirt
        // itself. Nothing under it is unpushed, so it is dropped and said,
        // not halted on — a lane that halts on a leftover halts for every
        // job after.
        let git = repo(conflicted_lane());
        let disk = Rc::new(disk_with_lane());
        let (journal, log) = journal();
        let mount = mount_logged(&git, &disk, &lane_wanted(), false, &log)
            .await
            .expect("mounted");
        assert_eq!(mount.branch, "main_agent");
        let calls = git.calls();
        assert!(calls.contains(&"checkout main_agent --force".to_string()));
        assert!(calls.contains(&"reset_hard origin/main_agent".to_string()));
        assert!(calls.contains(&"clean".to_string()));
        assert!(
            git.dirty_files().await.expect("read").is_empty(),
            "the checkout is clean"
        );
        assert!(
            journal.lines.borrow().iter().any(|l| l
                == "workspace router-lane-1: 3 uncommitted change(s) a previous run \
                    left — dropped, nothing on it is unpushed"),
            "{:?}",
            journal.lines.borrow()
        );
    }

    #[tokio::test]
    async fn a_human_s_workspace_still_halts_on_the_same_dirt() {
        // `drop_uncommitted` is the whole difference: without it, the rule
        // "nothing is overwritten without saying so" stands as it was.
        let git = repo(conflicted_lane());
        let disk = Rc::new(disk_with_lane());
        let asked = Wanted {
            drop_uncommitted: false,
            ..lane_wanted()
        };
        let err = mount_with(&git, &disk, &asked, false)
            .await
            .expect_err("must stop");
        assert!(err.reason().contains("3 uncommitted change(s)"));
        assert!(!git.calls().iter().any(|c| c.starts_with("reset_hard")));
    }

    #[tokio::test]
    async fn a_lane_holding_unpushed_commits_halts_naming_the_branch_and_how_many() {
        // router-lane-1 on ForgeDesMondes, the morning after: a repair's
        // merge commit never pushed. Dropping the dirt would not make that
        // work pushed; the halt names it for whoever comes to look.
        let git = repo(FakeRepo {
            at_risk: vec!["feat/battlemap-instance-aggregate".to_string()],
            unpushed: vec!["3e07a57 Merge milestone/18 into feat/battlemap".to_string()],
            dirty: vec!["?? scratch/".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_lane());
        let err = mount_with(&git, &disk, &lane_wanted(), false)
            .await
            .expect_err("must stop");
        assert_eq!(
            err.reason(),
            "the workspace \"router-lane-1\" carries 1 unpushed commit(s) on \
             feat/battlemap-instance-aggregate, 1 uncommitted change(s) — look at \
             .llocal/agentic_workspaces/router-lane-1, or re-run with --force-reset to \
             overwrite it"
        );
        assert!(
            !git.calls().iter().any(|c| c.starts_with("reset_hard")),
            "nothing was overwritten"
        );
    }

    #[tokio::test]
    async fn a_release_keeps_unpushed_commits_whole_and_says_so_as_an_event() {
        let git = repo(FakeRepo {
            at_risk: vec!["feat/x".to_string()],
            unpushed: vec!["a1 fix".to_string(), "b2 fix again".to_string()],
            dirty: vec![" M src/lib.rs".to_string()],
            merging: Cell::new(true),
            local: vec!["feat/x".to_string(), "scratch".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_lane());
        let (journal, log) = journal();
        provisioner(&git, Rc::clone(&disk))
            .release(&lane("feat/x"), &log)
            .await;
        assert_eq!(
            *journal.events.borrow(),
            [Event::WorkKept {
                workspace: "router-lane-1".to_string(),
                path: LANE.to_string(),
                branch: "feat/x".to_string(),
                branches: vec!["feat/x".to_string()],
                commits: 2,
                stashes: 0,
            }]
        );
        let calls = git.calls();
        assert!(
            calls.contains(&"abort_merge".to_string()),
            "the half-merge is noise"
        );
        assert!(
            !calls.iter().any(|c| c.starts_with("reset_hard")
                || c == "clean"
                || c.starts_with("checkout")
                || c.starts_with("delete_branch")),
            "nothing else is touched: {calls:?}"
        );
        assert_eq!(
            git.dirty_files().await.expect("read").len(),
            1,
            "kept whole"
        );
        assert!(
            journal.lines.borrow().iter().any(|l| l.starts_with(
                "warning: workspace router-lane-1 kept as it is: 2 unpushed commit(s) on feat/x"
            )),
            "{:?}",
            journal.lines.borrow()
        );
    }

    #[tokio::test]
    async fn a_stash_alone_is_a_leftover_the_release_drops() {
        let git = repo(FakeRepo {
            stashes: vec!["stash@{0}: WIP".to_string()],
            ..clean_repo()
        });
        let disk = Rc::new(disk_with_lane());
        let (journal, log) = journal();
        provisioner(&git, Rc::clone(&disk))
            .release(&lane("feat/x"), &log)
            .await;
        assert!(
            journal.events.borrow().is_empty(),
            "no work kept: {:?}",
            journal.events.borrow()
        );
        let calls = git.calls();
        assert!(calls.contains(&"clear_stashes".to_string()), "{calls:?}");
        assert!(
            calls.iter().any(|c| c.starts_with("reset_hard")),
            "{calls:?}"
        );
        assert!(
            journal
                .lines
                .borrow()
                .iter()
                .any(|l| l.contains("1 stash entry(ies) a session left — dropped")),
            "{:?}",
            journal.lines.borrow()
        );
    }

    #[tokio::test]
    async fn a_merge_nobody_began_is_not_aborted_and_a_clean_lane_is_still_put_back() {
        let git = repo(clean_repo());
        let disk = Rc::new(disk_with_lane());
        let (journal, log) = journal();
        provisioner(&git, Rc::clone(&disk))
            .release(&lane("feat/x"), &log)
            .await;
        let calls = git.calls();
        assert!(!calls.contains(&"abort_merge".to_string()), "{calls:?}");
        assert!(calls.contains(&"reset_hard origin/feat/x".to_string()));
        assert!(
            !journal.lines.borrow().iter().any(|l| l.contains("dropped")),
            "nothing to drop, nothing said: {:?}",
            journal.lines.borrow()
        );
    }

    #[tokio::test]
    async fn an_abort_that_fails_is_said_and_the_reset_still_clears_the_merge() {
        let git = repo(FakeRepo {
            failing: vec!["abort_merge"],
            ..conflicted_lane()
        });
        let disk = Rc::new(disk_with_lane());
        let (journal, log) = journal();
        provisioner(&git, Rc::clone(&disk))
            .release(&lane("feat/diffusion-aggregate"), &log)
            .await;
        assert!(
            journal.lines.borrow().iter().any(|l| l.starts_with(
                "warning: workspace router-lane-1: cannot abort its unfinished merge"
            )),
            "{:?}",
            journal.lines.borrow()
        );
        assert!(!git.merging.get(), "`reset --hard` drops MERGE_HEAD too");
    }

    #[tokio::test]
    async fn a_release_without_a_mount_or_a_branch_asks_git_nothing() {
        let git = repo(conflicted_lane());
        let disk = Rc::new(disk_with_lane());
        let prov = provisioner(&git, Rc::clone(&disk));
        prov.release(&Mount::in_place(source()), &Logbook::null())
            .await;
        prov.release(&lane(""), &Logbook::null()).await;
        assert_eq!(git.calls(), [] as [String; 0]);
    }

    #[tokio::test]
    async fn a_disposable_checkout_left_clean_by_its_release_is_removed() {
        // The release leaves nothing to lose, so the unmount that follows
        // finds nothing to keep.
        let git = repo(conflicted_lane());
        let disk = Rc::new(disk_with_workspace());
        provisioner(&git, Rc::clone(&disk))
            .release(&disposable("main_agent"), &Logbook::null())
            .await;
        assert_eq!(disk.removed(), vec![PathBuf::from(DEST)]);
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
