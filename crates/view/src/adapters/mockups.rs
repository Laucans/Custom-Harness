//! The product's mock-up, read with `git` on every branch and edited in a
//! checkout of the view's own.
//!
//! What a branch carries under `mock/` is read from the shared mirror
//! (`git_units::Mirror`). A session that edits the mock-up needs files on
//! disk: the first time, a **worktree** of the branch is added under
//! `.llocal/view-mockups/<branch slug>/`, detached, and from then on the
//! view serves that folder — so the page shows what the session saves as it
//! saves it. **Save** commits what changed under `mock/` on a new
//! `mockup/<name>` branch and pushes it; **discard** removes the worktree.
//! The worktree is always detached once a save is done, so the mirror's
//! fetch never meets a branch checked out somewhere.
//!
//! Under `--demo`, the sample's folders stand for the branches, and a
//! session edits them in a copy of the yard's ([`seed`]) — never the sample
//! the harness tracks: there is no repository to save to.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use crate::adapters::git_units::{Mirror, run};
use crate::domain::drafts::fingerprint;
use crate::domain::mockup;
use crate::ports::{MockFile, MockPage, MockState, Mockups, PRODUCT_FILES};

/// The mock-up read through `git`, edited in a worktree.
pub struct GitMockups {
    mirror: Arc<Mirror>,
    workspaces: PathBuf,
}

impl GitMockups {
    /// Reads through `mirror`; the worktrees go under `workspaces`.
    #[must_use]
    pub fn new(mirror: Arc<Mirror>, workspaces: &Path) -> Self {
        Self {
            mirror,
            workspaces: workspaces.to_path_buf(),
        }
    }

    fn folder(&self, branch: &str) -> PathBuf {
        self.workspaces.join(mockup::slug(branch))
    }

    /// The branch's worktree, when one was made: a worktree carries a
    /// `.git` file.
    fn worktree(&self, branch: &str) -> Option<PathBuf> {
        let folder = self.folder(branch);
        folder.join(".git").exists().then_some(folder)
    }

    /// Where a discarded workspace of `branch` is kept: a ref beside the
    /// branches, which the mirror's fetch never prunes.
    fn discarded_ref(branch: &str) -> String {
        format!("refs/harness/discarded/{branch}")
    }

    /// The commit a discarded workspace of `branch` was kept as, if any.
    fn discarded(&self, branch: &str) -> Option<String> {
        let out = self
            .mirror
            .git(
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &Self::discarded_ref(branch),
                ],
                None,
            )
            .ok()?;
        let commit = String::from_utf8_lossy(&out).trim().to_string();
        (!commit.is_empty()).then_some(commit)
    }

    fn at(folder: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
        run(Command::new("git").arg("-C").arg(folder).args(args), None)
    }

    /// Something under `mock/` differs from the commit.
    fn dirty(folder: &Path) -> Result<bool, String> {
        let mut paths = vec!["status", "--porcelain", "--", "mock"];
        paths.extend(PRODUCT_FILES);
        let out = Self::at(folder, &paths)?;
        Ok(!String::from_utf8_lossy(&out).trim().is_empty())
    }

    /// What a publish stages: `mock/`, and the creation's files at the root
    /// that are there.
    fn staged(folder: &Path) -> Vec<&'static str> {
        let mut paths = vec!["add", "-A", "--", "mock"];
        paths.extend(PRODUCT_FILES.iter().filter(|f| folder.join(f).exists()));
        paths
    }

    /// The branch checked out, `HEAD` when detached.
    fn current_branch(folder: &Path) -> Result<String, String> {
        let out = Self::at(folder, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// The branch the worktree was last saved to: the newest `mockup/*`
    /// branch that contains its head.
    fn saved_as(folder: &Path) -> String {
        let Ok(out) = Self::at(
            folder,
            &[
                "branch",
                "--list",
                "--points-at",
                "HEAD",
                "--format=%(refname:short)",
                "mockup/*",
            ],
        ) else {
            return String::new();
        };
        String::from_utf8_lossy(&out)
            .lines()
            .map(str::trim)
            .find(|b| b.starts_with("mockup/"))
            .unwrap_or_default()
            .to_string()
    }
}

/// A fingerprint of every file under `dir`: the newest change and the
/// total size — what the page reloads on.
fn stamp_of(dir: &Path) -> String {
    fn walk(at: &Path, newest: &mut u128, size: &mut u64) {
        let Ok(entries) = fs::read_dir(at) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, newest, size);
            } else if let Ok(meta) = entry.metadata() {
                *size = size.saturating_add(meta.len());
                let at = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis());
                *newest = (*newest).max(at);
            }
        }
    }
    let mut newest = 0u128;
    let mut size = 0u64;
    walk(dir, &mut newest, &mut size);
    format!("{newest}-{size}")
}

/// What a manifest's text lists, when there is one.
fn listed(manifest: Option<&str>) -> Result<mockup::Manifest, String> {
    manifest.map_or_else(|| Ok(mockup::Manifest::default()), mockup::manifest_of)
}

/// Writes `dir/mock/<path>`, its folders made first.
fn write_under(dir: &Path, path: &str, bytes: &[u8]) -> Result<(), String> {
    let full = dir.join("mock").join(path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(&full, bytes).map_err(|e| format!("{}: {e}", full.display()))
}

/// The files under `dir/mock`, oldest written first.
fn files_of(dir: &Path) -> Vec<MockFile> {
    fn walk(root: &Path, at: &Path, found: &mut Vec<MockFile>) {
        let Ok(entries) = fs::read_dir(at) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, found);
            } else if let (Ok(meta), Ok(rel)) = (entry.metadata(), path.strip_prefix(root)) {
                let at = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                found.push(MockFile {
                    path: rel.to_string_lossy().replace('\\', "/"),
                    at,
                });
            }
        }
    }
    let root = dir.join("mock");
    let mut found = Vec::new();
    walk(&root, &root, &mut found);
    found.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.path.cmp(&b.path)));
    found
}

/// Reads `dir/mock/<path>`; `None` when it is not there.
fn read_under(dir: &Path, path: &str) -> Result<Option<Vec<u8>>, String> {
    match fs::read(dir.join("mock").join(path)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) if e.kind() == ErrorKind::IsADirectory => Ok(None),
        Err(e) => Err(format!("{}: {e}", dir.join("mock").join(path).display())),
    }
}

/// The state of a mock-up held in a folder: the demo's, or a worktree.
fn state_of_folder(branch: &str, folder: &Path) -> Result<MockState, String> {
    let manifest = fs::read_to_string(folder.join("mock/pages.json")).ok();
    let found = listed(manifest.as_deref())?;
    let pages = found
        .pages
        .into_iter()
        .map(|page| {
            let hash = fs::read(folder.join("mock").join(&page.path))
                .map(|bytes| fingerprint(&bytes))
                .unwrap_or_default();
            MockPage { hash, ..page }
        })
        .collect();
    Ok(MockState {
        branch: branch.to_string(),
        present: manifest.is_some(),
        pages,
        components: found.components,
        edited: false,
        saved: String::new(),
        discarded: false,
        stamp: stamp_of(&folder.join("mock")),
    })
}

impl Mockups for GitMockups {
    fn state(&self, branch: &str) -> Result<MockState, String> {
        if let Some(folder) = self.worktree(branch) {
            let mut state = state_of_folder(branch, &folder)?;
            state.edited = Self::dirty(&folder)?;
            state.saved = Self::saved_as(&folder);
            state.discarded = self.discarded(branch).is_some();
            return Ok(state);
        }
        self.mirror.refresh()?;
        let manifest = self
            .mirror
            .show(branch, "mock/pages.json")?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let found = listed(manifest.as_deref())?;
        let mut pages = Vec::new();
        for page in found.pages {
            let hash = self
                .mirror
                .show(branch, &format!("mock/{}", page.path))?
                .map(|bytes| fingerprint(&bytes))
                .unwrap_or_default();
            pages.push(MockPage { hash, ..page });
        }
        Ok(MockState {
            branch: branch.to_string(),
            present: manifest.is_some(),
            pages,
            components: found.components,
            edited: false,
            saved: String::new(),
            discarded: self.discarded(branch).is_some(),
            stamp: self.mirror.head_of(branch)?.unwrap_or_default(),
        })
    }

    fn file(&self, branch: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
        if !mockup::is_safe_path(path) {
            return Ok(None);
        }
        if let Some(folder) = self.worktree(branch) {
            return read_under(&folder, path);
        }
        self.mirror.refresh()?;
        self.mirror.show(branch, &format!("mock/{path}"))
    }

    fn workspace(&self, branch: &str) -> Result<PathBuf, String> {
        if let Some(folder) = self.worktree(branch) {
            return Ok(folder);
        }
        self.mirror.refresh()?;
        if self.mirror.head_of(branch)?.is_none() {
            return Err(format!("no branch `{branch}` to edit the mock-up of"));
        }
        fs::create_dir_all(&self.workspaces)
            .map_err(|e| format!("{}: {e}", self.workspaces.display()))?;
        let folder = self.folder(branch);
        // A folder left without its `.git` is a stale worktree: start over.
        let _ = fs::remove_dir_all(&folder);
        let _ = self.mirror.git(&["worktree", "prune"], None);
        let at = folder.to_string_lossy().into_owned();
        self.mirror.git(
            &[
                "worktree",
                "add",
                "--detach",
                "--quiet",
                &at,
                &format!("refs/heads/{branch}"),
            ],
            None,
        )?;
        Ok(folder)
    }

    fn save(&self, branch: &str, name: &str) -> Result<String, String> {
        let folder = self.worktree(branch).ok_or_else(|| {
            "nothing to save: no session edited this branch's mock-up".to_string()
        })?;
        if !Self::dirty(&folder)? {
            return Err("nothing to save: the mock-up is as committed".to_string());
        }
        let mut target = mockup::save_branch(name);
        if self.mirror.head_of(&target)?.is_some() {
            let stamp = jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S");
            target = format!("{target}-{stamp}");
        }
        Self::at(&folder, &["checkout", "--quiet", "-b", &target])?;
        Self::at(&folder, &["add", "-A", "--", "mock"])?;
        let message = format!("mockup: {} (from {branch})", name.trim());
        Self::at(&folder, &["commit", "--quiet", "-m", &message])?;
        let pushed = Self::at(
            &folder,
            &[
                "push",
                "--quiet",
                "origin",
                &format!("HEAD:refs/heads/{target}"),
            ],
        );
        // Detached again: the mirror's fetch never meets a checked-out branch.
        let _ = Self::at(&folder, &["checkout", "--quiet", "--detach"]);
        pushed?;
        let _ = Self::current_branch(&folder);
        Ok(target)
    }

    fn write(&self, branch: &str, path: &str, bytes: &[u8]) -> Result<(), String> {
        if !mockup::is_safe_path(path) {
            return Err(format!("not a path under mock/: {path}"));
        }
        let folder = self.workspace(branch)?;
        write_under(&folder, path, bytes)
    }

    fn write_product(&self, branch: &str, path: &str, bytes: &[u8]) -> Result<(), String> {
        if !PRODUCT_FILES.contains(&path) {
            return Err(format!("{path} is no file the creation writes at the root"));
        }
        let full = self.workspace(branch)?.join(path);
        fs::write(&full, bytes).map_err(|e| format!("{}: {e}", full.display()))
    }

    fn discard(&self, branch: &str) -> Result<(), String> {
        if let Some(folder) = self.worktree(branch) {
            // What the workspace changed is committed aside first, so the
            // discard can be taken back.
            if Self::dirty(&folder)? {
                Self::at(&folder, &["add", "-A", "--", "mock"])?;
                Self::at(
                    &folder,
                    &[
                        "-c",
                        "user.name=harness",
                        "-c",
                        "user.email=harness@localhost",
                        "-c",
                        "commit.gpgsign=false",
                        "commit",
                        "--quiet",
                        "--no-verify",
                        "-m",
                        &format!("mockup: discarded from {branch}"),
                    ],
                )?;
                let head = Self::at(&folder, &["rev-parse", "HEAD"])?;
                let head = String::from_utf8_lossy(&head).trim().to_string();
                self.mirror
                    .git(&["update-ref", &Self::discarded_ref(branch), &head], None)?;
            }
            let at = folder.to_string_lossy().into_owned();
            self.mirror
                .git(&["worktree", "remove", "--force", &at], None)?;
        }
        let _ = self.mirror.git(&["worktree", "prune"], None);
        Ok(())
    }

    fn restore(&self, branch: &str) -> Result<(), String> {
        let kept = self
            .discarded(branch)
            .ok_or_else(|| format!("nothing discarded on `{branch}` to bring back"))?;
        if let Some(folder) = self.worktree(branch) {
            if Self::dirty(&folder)? {
                return Err("the mock-up is being edited: save or discard it first".to_string());
            }
            let at = folder.to_string_lossy().into_owned();
            self.mirror
                .git(&["worktree", "remove", "--force", &at], None)?;
        }
        fs::create_dir_all(&self.workspaces)
            .map_err(|e| format!("{}: {e}", self.workspaces.display()))?;
        let folder = self.folder(branch);
        let _ = fs::remove_dir_all(&folder);
        let _ = self.mirror.git(&["worktree", "prune"], None);
        let at = folder.to_string_lossy().into_owned();
        self.mirror.git(
            &["worktree", "add", "--detach", "--quiet", &at, &kept],
            None,
        )?;
        // Back on the branch, the kept changes uncommitted again.
        Self::at(
            &folder,
            &[
                "reset",
                "--quiet",
                "--mixed",
                &format!("refs/heads/{branch}"),
            ],
        )?;
        self.mirror
            .git(&["update-ref", "-d", &Self::discarded_ref(branch)], None)?;
        Ok(())
    }

    fn remove(&self, branch: &str, path: &str) -> Result<(), String> {
        if !mockup::is_safe_path(path) {
            return Err(format!("not a path under mock/: {path}"));
        }
        let folder = self.workspace(branch)?;
        remove_under(&folder, path)
    }

    fn branches(&self) -> Result<Vec<String>, String> {
        self.mirror.refresh()?;
        self.mirror.branches()
    }

    fn files(&self, branch: &str) -> Result<Vec<MockFile>, String> {
        if let Some(folder) = self.worktree(branch) {
            return Ok(files_of(&folder));
        }
        self.mirror.refresh()?;
        if self.mirror.head_of(branch)?.is_none() {
            return Ok(Vec::new());
        }
        let out = self.mirror.git(
            &[
                "ls-tree",
                "-r",
                "--name-only",
                &format!("refs/heads/{branch}"),
                "--",
                "mock/",
            ],
            None,
        )?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|line| line.strip_prefix("mock/"))
            .map(|path| MockFile {
                path: path.to_string(),
                at: 0,
            })
            .collect())
    }

    fn ensure_branch(&self, branch: &str, from: &str) -> Result<(), String> {
        self.mirror.refresh()?;
        if self.mirror.head_of(branch)?.is_some() {
            return Ok(());
        }
        let branches = self.mirror.branches()?;
        let base = [from, "main", "master"]
            .into_iter()
            .find(|b| branches.iter().any(|have| have == b))
            .or_else(|| branches.first().map(String::as_str))
            .ok_or_else(|| "the product has no branch to start from".to_string())?
            .to_string();
        let spec = format!("refs/heads/{base}");
        self.mirror
            .git(&["branch", "--quiet", branch, &spec], None)?;
        let push = format!("refs/heads/{branch}:refs/heads/{branch}");
        self.mirror
            .git(&["push", "--quiet", "origin", &push], None)?;
        Ok(())
    }

    fn publish(&self, branch: &str, message: &str) -> Result<String, String> {
        let folder = self
            .worktree(branch)
            .ok_or_else(|| "nothing to publish: no workspace".to_string())?;
        if !Self::dirty(&folder)? {
            return Err("nothing to publish: the branch is as committed".to_string());
        }
        Self::at(&folder, &Self::staged(&folder))?;
        let message = if message.trim().is_empty() {
            format!("design: {branch}")
        } else {
            message.trim().chars().take(200).collect()
        };
        Self::at(&folder, &["commit", "--quiet", "-m", &message])?;
        let spec = format!("HEAD:refs/heads/{branch}");
        Self::at(&folder, &["push", "--quiet", "origin", &spec])?;
        // The mirror learns the new head on its next fetch; the worktree
        // already sits on it, detached.
        let out = Self::at(&folder, &["rev-parse", "HEAD"])?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    fn carry(
        &self,
        from: &str,
        to: &str,
        folders: &[&str],
        message: &str,
    ) -> Result<Option<String>, String> {
        self.mirror.refresh()?;
        let mut last = String::new();
        for _ in 0..CARRY_ATTEMPTS {
            let base = self
                .mirror
                .head_of(to)?
                .ok_or_else(|| format!("no branch `{to}` to carry the design system to"))?;
            let Some(commit) = self.spliced(from, &base, folders, message)? else {
                return Ok(None);
            };
            let spec = format!("{commit}:refs/heads/{to}");
            match self.mirror.git(&["push", "--quiet", "origin", &spec], None) {
                Ok(_) => return Ok(Some(commit)),
                Err(why) => {
                    // The branch moved since it was read: read it again, splice again.
                    last = why;
                    let fetch = format!("+refs/heads/{to}:refs/heads/{to}");
                    self.mirror
                        .git(&["fetch", "--quiet", "origin", &fetch], None)?;
                }
            }
        }
        Err(format!("`{to}` kept moving: {last}"))
    }
}

/// How many times a carry is pushed onto a branch that keeps moving.
const CARRY_ATTEMPTS: usize = 3;

impl GitMockups {
    /// The commit on `base` whose `mock/<folder>` are `from`'s, made with
    /// `git` plumbing — no checkout is touched; `None` when `base` already
    /// has them so.
    fn spliced(
        &self,
        from: &str,
        base: &str,
        folders: &[&str],
        message: &str,
    ) -> Result<Option<String>, String> {
        let mock = splice(
            &self.listing(&format!("{base}:mock"))?,
            &self.listing(&format!("{from}:mock"))?,
            folders,
        );
        let root = self.listing(&format!("{base}^{{tree}}"))?;
        let root = if mock.is_empty() {
            splice(&root, &[], &["mock"])
        } else {
            let tree = self.mktree(&mock)?;
            splice(
                &root,
                format!("040000 tree {tree}\tmock\0").as_bytes(),
                &["mock"],
            )
        };
        let tree = self.mktree(&root)?;
        let before = self
            .mirror
            .git(&["rev-parse", &format!("{base}^{{tree}}")], None)?;
        if String::from_utf8_lossy(&before).trim() == tree {
            return Ok(None);
        }
        let out = self
            .mirror
            .git(&["commit-tree", &tree, "-p", base, "-m", message], None)?;
        Ok(Some(String::from_utf8_lossy(&out).trim().to_string()))
    }

    /// `git ls-tree -z` of a tree, empty when there is no such tree.
    fn listing(&self, tree: &str) -> Result<Vec<u8>, String> {
        match self.mirror.git(&["ls-tree", "-z", tree], None) {
            Ok(bytes) => Ok(bytes),
            Err(why)
                if why.contains("ot a valid object name")
                    || why.contains("nvalid object name")
                    || why.contains("does not exist") =>
            {
                Ok(Vec::new())
            }
            Err(why) => Err(why),
        }
    }

    /// The tree made of a `ls-tree -z` listing.
    fn mktree(&self, listing: &[u8]) -> Result<String, String> {
        let out = self.mirror.git(&["mktree", "-z"], Some(listing))?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}

/// A `ls-tree -z` listing of `base` whose entries named in `names` are
/// those of `source` instead — absent when `source` has none.
fn splice(base: &[u8], source: &[u8], names: &[&str]) -> Vec<u8> {
    let replaced = |entry: &&[u8]| {
        entry
            .iter()
            .position(|b| *b == b'\t')
            .and_then(|tab| entry.get(tab.saturating_add(1)..))
            .is_some_and(|name| names.iter().any(|n| n.as_bytes() == name))
    };
    let entries = |listing: &'_ [u8]| -> Vec<Vec<u8>> {
        listing
            .split(|b| *b == 0)
            .filter(|entry| !entry.is_empty())
            .map(<[u8]>::to_vec)
            .collect()
    };
    let mut out = Vec::new();
    let kept = entries(base)
        .into_iter()
        .filter(|e| !replaced(&e.as_slice()));
    let taken = entries(source)
        .into_iter()
        .filter(|e| replaced(&e.as_slice()));
    for entry in kept.chain(taken) {
        out.extend_from_slice(&entry);
        out.push(0);
    }
    out
}

/// Removes `dir/mock/<path>`; absence is fine.
fn remove_under(dir: &Path, path: &str) -> Result<(), String> {
    let full = dir.join("mock").join(path);
    match fs::remove_file(&full) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", full.display())),
    }
}

/// Copies the sample at `from` to `to` once, when `to` is absent; a copy
/// cut short is never left behind as a whole one.
///
/// # Errors
///
/// A file of the sample unreadable, or the copy unwritable.
pub fn seed(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.exists() {
        return Ok(());
    }
    let partial = to.with_extension("partial");
    match fs::remove_dir_all(&partial) {
        Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    copy_tree(from, &partial)?;
    fs::rename(&partial, to)
}

/// Copies the folder `from` into `to`, recursively.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// The demo's mock-ups: one folder per branch under `dir` (a `/` in a
/// branch name is a `__` in the folder's), each with its `mock/`.
pub struct DirMockups {
    dir: PathBuf,
}

impl DirMockups {
    /// The sample at `dir`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn folder(&self, branch: &str) -> PathBuf {
        self.dir.join(mockup::slug(branch))
    }
}

impl Mockups for DirMockups {
    fn state(&self, branch: &str) -> Result<MockState, String> {
        state_of_folder(branch, &self.folder(branch))
    }

    fn file(&self, branch: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
        if !mockup::is_safe_path(path) {
            return Ok(None);
        }
        read_under(&self.folder(branch), path)
    }

    fn workspace(&self, branch: &str) -> Result<PathBuf, String> {
        let folder = self.folder(branch);
        if folder.is_dir() {
            Ok(folder)
        } else {
            Err(format!("the sample has no branch `{branch}`"))
        }
    }

    fn save(&self, _branch: &str, _name: &str) -> Result<String, String> {
        Err("the sample has no repository to save a mock-up to".to_string())
    }

    fn write(&self, branch: &str, path: &str, bytes: &[u8]) -> Result<(), String> {
        if !mockup::is_safe_path(path) {
            return Err(format!("not a path under mock/: {path}"));
        }
        write_under(&self.folder(branch), path, bytes)
    }

    fn write_product(&self, branch: &str, path: &str, bytes: &[u8]) -> Result<(), String> {
        if !PRODUCT_FILES.contains(&path) {
            return Err(format!("{path} is no file the creation writes at the root"));
        }
        let full = self.folder(branch).join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&full, bytes).map_err(|e| format!("{}: {e}", full.display()))
    }

    fn discard(&self, _branch: &str) -> Result<(), String> {
        Err("the sample is not a checkout: nothing to discard".to_string())
    }

    fn restore(&self, _branch: &str) -> Result<(), String> {
        Err("the sample is not a checkout: nothing to restore".to_string())
    }

    fn remove(&self, branch: &str, path: &str) -> Result<(), String> {
        if !mockup::is_safe_path(path) {
            return Err(format!("not a path under mock/: {path}"));
        }
        remove_under(&self.folder(branch), path)
    }

    fn branches(&self) -> Result<Vec<String>, String> {
        let entries =
            fs::read_dir(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let mut found: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| mockup::branch_of(&e.file_name().to_string_lossy()))
            .collect();
        found.sort();
        Ok(found)
    }

    fn files(&self, branch: &str) -> Result<Vec<MockFile>, String> {
        Ok(files_of(&self.folder(branch)))
    }

    fn ensure_branch(&self, branch: &str, _from: &str) -> Result<(), String> {
        // The sample is tracked: a branch is a folder it already has, never
        // one made on a visit.
        if self.folder(branch).is_dir() {
            Ok(())
        } else {
            Err(format!("the sample has no branch `{branch}`"))
        }
    }

    fn publish(&self, _branch: &str, _message: &str) -> Result<String, String> {
        Err("the sample has no repository to publish to".to_string())
    }

    fn carry(
        &self,
        _from: &str,
        _to: &str,
        _folders: &[&str],
        _message: &str,
    ) -> Result<Option<String>, String> {
        Err("the sample has no repository to carry the design system to".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real thing: a workspace's files are listed with their time, the
    /// committed ones without, a branch that is not there with none.
    #[test]
    fn a_workspace_s_files_are_listed_with_their_time() {
        let dir = std::env::temp_dir().join(format!("harness-view-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let work = dir.join("work");
        fs::create_dir_all(work.join("mock")).expect("dir");
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&work)
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "-b", "main_agent"]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "empty",
        ]);
        fs::write(work.join("mock/pages.json"), "{}").expect("write");
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "base",
        ]);
        git(&["checkout", "-q", "--detach"]);
        let mockups = GitMockups::new(Arc::new(Mirror::local(&work)), &dir.join("ws"));
        assert_eq!(
            mockups.files("main_agent").expect("listed"),
            vec![MockFile {
                path: "pages.json".to_string(),
                at: 0
            }],
            "as committed: no time"
        );
        assert!(mockups.files("nowhere").expect("listed").is_empty());
        mockups
            .write("main_agent", "pages/home.html", b"<main>home</main>")
            .expect("written");
        let edited = mockups.files("main_agent").expect("listed");
        let page = edited
            .iter()
            .find(|f| f.path == "pages/home.html")
            .expect("the page written is listed");
        assert!(page.at > 0, "a workspace's file has its time");
        assert!(edited.iter().any(|f| f.path == "pages.json"));
        let _ = fs::remove_dir_all(&dir);
    }

    /// The real thing: a workspace's changes discarded are kept aside, and
    /// brought back uncommitted; a workspace being edited refuses it.
    #[test]
    fn a_discarded_workspace_is_brought_back() {
        let dir = std::env::temp_dir().join(format!("harness-view-restore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let work = dir.join("work");
        fs::create_dir_all(work.join("mock/pages")).expect("dir");
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&work)
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "-b", "main_agent"]);
        for (key, value) in [
            ("user.name", "t"),
            ("user.email", "t@t"),
            ("commit.gpgsign", "false"),
        ] {
            git(&["config", key, value]);
        }
        fs::write(work.join("mock/pages.json"), r#"{"version":1,"pages":[]}"#).expect("write");
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "base"]);
        git(&["checkout", "-q", "--detach"]);
        let mockups = GitMockups::new(Arc::new(Mirror::local(&work)), &dir.join("ws"));

        assert!(
            mockups.restore("main_agent").is_err(),
            "nothing discarded yet"
        );
        mockups
            .write("main_agent", "pages/home.html", b"<main>home</main>")
            .expect("written");
        mockups.discard("main_agent").expect("discarded");
        let gone = mockups.state("main_agent").expect("state");
        assert!(gone.discarded && !gone.edited);
        assert_eq!(
            mockups.file("main_agent", "pages/home.html").expect("read"),
            None
        );

        mockups.workspace("main_agent").expect("a clean workspace");
        mockups
            .restore("main_agent")
            .expect("brought back over a clean workspace");
        let back = mockups.state("main_agent").expect("state");
        assert!(back.edited && !back.discarded);
        assert_eq!(
            mockups
                .file("main_agent", "pages/home.html")
                .expect("read")
                .as_deref(),
            Some(&b"<main>home</main>"[..])
        );
        mockups.discard("main_agent").expect("discarded again");
        mockups
            .write("main_agent", "pages/other.html", b"other")
            .expect("written");
        assert!(
            mockups.restore("main_agent").is_err(),
            "a workspace being edited"
        );
        mockups
            .discard("main_agent")
            .expect("the edit discarded, kept in its turn");
        mockups
            .restore("main_agent")
            .expect("the last discard comes back");
        assert!(
            mockups
                .file("main_agent", "pages/other.html")
                .expect("read")
                .is_some()
        );
        assert_eq!(
            mockups.file("main_agent", "pages/home.html").expect("read"),
            None,
            "only the last discard is kept"
        );
        mockups.discard("main_agent").expect("clean up");
        let clean = mockups.workspace("main_agent").expect("workspace");
        mockups
            .discard("main_agent")
            .expect("a clean workspace keeps nothing aside");
        assert!(!clean.join(".git").exists());
        assert!(
            mockups.state("main_agent").expect("state").discarded,
            "the earlier discard stays kept"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The real thing, against throwaway repositories: the design system's
    /// folders carried onto an integration branch that moved since the
    /// fork, the rest of it untouched, a branch moved again caught up.
    #[test]
    fn the_design_system_is_carried_onto_the_integration_branch_alone() {
        let dir = std::env::temp_dir().join(format!("harness-view-carry-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let origin = dir.join("origin.git");
        let work = dir.join("work");
        fs::create_dir_all(work.join("mock/kit")).expect("dir");
        let git = |at: &Path, args: &[&str]| -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(at)
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(
            &dir,
            &["init", "-q", "--bare", "-b", "main_agent", "origin.git"],
        );
        git(&work, &["init", "-q", "-b", "main_agent"]);
        for (key, value) in [
            ("user.name", "t"),
            ("user.email", "t@t"),
            ("commit.gpgsign", "false"),
        ] {
            git(&work, &["config", key, value]);
        }
        git(
            &work,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        fs::write(work.join("app.txt"), "base").expect("write");
        fs::write(work.join("mock/pages.json"), "{}").expect("write");
        fs::write(work.join("mock/kit/Old.html"), "old").expect("write");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "base"]);
        git(&work, &["checkout", "-q", "-b", "design-system"]);
        fs::remove_file(work.join("mock/kit/Old.html")).expect("rm");
        fs::create_dir_all(work.join("mock/design")).expect("dir");
        fs::write(work.join("mock/kit/Button.html"), "button").expect("write");
        fs::write(work.join("mock/design/tokens.css"), ":root{}").expect("write");
        fs::write(work.join("app.txt"), "not carried").expect("write");
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "design"]);
        let published = git(&work, &["rev-parse", "HEAD"]);
        git(&work, &["checkout", "-q", "main_agent"]);
        fs::write(work.join("other.txt"), "moved on").expect("write");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "meanwhile"]);
        git(
            &work,
            &["push", "-q", "origin", "main_agent", "design-system"],
        );
        let before = git(&work, &["rev-parse", "main_agent"]);
        git(&work, &["checkout", "-q", "--detach"]);

        let mockups = GitMockups::new(Arc::new(Mirror::local(&work)), &dir.join("ws"));
        let carried = mockups
            .carry(
                &published,
                "main_agent",
                &["design", "kit"],
                "design: carried",
            )
            .expect("carried")
            .expect("a commit");
        let show = |path: &str| git(&origin, &["show", &format!("main_agent:{path}")]);
        assert_eq!(git(&origin, &["rev-parse", "main_agent"]), carried);
        assert_eq!(git(&origin, &["rev-parse", "main_agent^"]), before);
        assert_eq!(show("mock/kit/Button.html"), "button");
        assert_eq!(show("mock/design/tokens.css"), ":root{}");
        assert_eq!(
            show("app.txt"),
            "base",
            "only the design system's folders travel"
        );
        assert_eq!(show("other.txt"), "moved on");
        assert_eq!(show("mock/pages.json"), "{}");
        let kit = git(
            &origin,
            &["ls-tree", "--name-only", "main_agent", "mock/kit/"],
        );
        assert!(!kit.contains("Old.html"), "a fragment gone goes: {kit}");
        // The local branch is behind the remote now: the push is refused,
        // the branch read again, and nothing is left to carry.
        let again = mockups.carry(&published, "main_agent", &["design", "kit"], "again");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(again.expect("caught up"), None);
    }

    #[test]
    fn the_creation_s_files_at_the_root_are_written_and_published_beside_the_mock_up() {
        let dir = std::env::temp_dir().join(format!("harness-view-product-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let origin = dir.join("origin.git");
        let work = dir.join("work");
        fs::create_dir_all(work.join("mock")).expect("dir");
        let git = |at: &Path, args: &[&str]| -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(at)
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(
            &dir,
            &["init", "-q", "--bare", "-b", "main_agent", "origin.git"],
        );
        git(&work, &["init", "-q", "-b", "main_agent"]);
        for (key, value) in [
            ("user.name", "t"),
            ("user.email", "t@t"),
            ("commit.gpgsign", "false"),
        ] {
            git(&work, &["config", key, value]);
        }
        git(
            &work,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        fs::write(work.join("mock/pages.json"), "{}").expect("write");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "base"]);
        git(&work, &["push", "-q", "origin", "main_agent"]);
        git(&work, &["checkout", "-q", "--detach"]);

        let mockups = GitMockups::new(Arc::new(Mirror::local(&work)), &dir.join("ws"));
        let folder = mockups.workspace("main_agent").expect("a workspace");
        assert_eq!(
            GitMockups::staged(&folder),
            vec!["add", "-A", "--", "mock"],
            "no ports.json yet: mock/ alone"
        );
        assert_eq!(
            mockups.write_product("main_agent", "notes.txt", b"x"),
            Err("notes.txt is no file the creation writes at the root".to_string())
        );
        assert!(!folder.join("notes.txt").exists());
        mockups
            .write_product("main_agent", "ports.json", b"{\"version\": 1}")
            .expect("written");
        assert_eq!(
            GitMockups::staged(&folder),
            vec!["add", "-A", "--", "mock", "ports.json"]
        );
        let published = mockups.publish("main_agent", "ports").expect("published");
        let on_origin = git(&origin, &["rev-parse", "main_agent"]);
        let shown = git(&origin, &["show", "main_agent:ports.json"]);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(published, on_origin);
        assert_eq!(shown, "{\"version\": 1}");
    }

    #[test]
    fn a_splice_takes_the_named_entries_from_the_source() {
        let base = b"040000 tree a\tdesign\x00100644 blob b\tpages.json\x00040000 tree c\tkit\x00";
        let source = b"040000 tree d\tdesign\x00100644 blob e\tpages.json\x00";
        let out = String::from_utf8(splice(base, source, &["design", "kit"])).expect("utf8");
        assert_eq!(
            out,
            "100644 blob b\tpages.json\x00040000 tree d\tdesign\x00"
        );
    }

    #[test]
    fn a_folder_writes_only_the_creation_s_files_at_its_root() {
        let dir = std::env::temp_dir().join(format!("harness-dir-product-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mockups = DirMockups::new(&dir);
        assert_eq!(
            mockups.write_product("main_agent", "mock/x.html", b"x"),
            Err("mock/x.html is no file the creation writes at the root".to_string())
        );
        mockups
            .write_product("main_agent", "ports.json", b"{}")
            .expect("written");
        let written = fs::read_to_string(dir.join(mockup::slug("main_agent")).join("ports.json"));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(written.expect("there"), "{}");
    }

    #[test]
    fn the_demo_edits_a_copy_seeded_once() {
        let dir = std::env::temp_dir().join(format!("seed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let sample = dir.join("sample");
        fs::create_dir_all(sample.join("main/mock")).expect("sample");
        fs::write(sample.join("main/mock/pages.json"), "[]").expect("write");
        let copy = dir.join("copy");
        seed(&sample, &copy).expect("seed");
        assert_eq!(
            fs::read_to_string(copy.join("main/mock/pages.json")).expect("copied"),
            "[]"
        );
        fs::write(copy.join("main/mock/pages.json"), "edited").expect("edit");
        seed(&sample, &copy).expect("again");
        assert_eq!(
            fs::read_to_string(copy.join("main/mock/pages.json")).expect("kept"),
            "edited"
        );
        assert_eq!(
            fs::read_to_string(sample.join("main/mock/pages.json")).expect("sample"),
            "[]"
        );
        assert!(!dir.join("copy.partial").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_without_pages_json_has_no_mockup_and_a_stamp_follows_the_files() {
        let dir = std::env::temp_dir().join(format!("harness-mockups-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("main/mock/pages")).expect("temp dir");
        let sample = DirMockups::new(&dir);
        assert!(sample.restore("main").is_err(), "a sample keeps no discard");
        fs::write(dir.join("main/mock/pages/a.html"), "a").expect("page");
        assert_eq!(
            sample
                .files("main")
                .expect("listed")
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["pages/a.html"]
        );
        fs::remove_file(dir.join("main/mock/pages/a.html")).expect("rm");
        let none = sample.state("main").expect("read");
        assert!(!none.present && none.pages.is_empty(), "expected something");
        fs::write(
            dir.join("main/mock/pages.json"),
            r#"{"version":1,"pages":[{"path":"pages/home.html","title":"Home","objective":"o"}]}"#,
        )
        .expect("manifest");
        fs::write(dir.join("main/mock/pages/home.html"), "<h1>hi</h1>").expect("page");
        let state = sample.state("main").expect("read");
        assert!(state.present);
        assert_eq!(state.pages[0].title, "Home");
        assert_eq!(
            state.pages[0].hash.len(),
            16,
            "a page carries its fingerprint"
        );
        sample
            .write("main", "drafts/home.json", b"{}")
            .expect("written");
        assert_eq!(
            sample.file("main", "drafts/home.json").expect("read"),
            Some(b"{}".to_vec())
        );
        assert!(sample.write("main", "../x", b"").is_err());
        assert_eq!(
            sample.file("main", "pages/home.html").expect("read"),
            Some(b"<h1>hi</h1>".to_vec())
        );
        assert_eq!(sample.file("main", "../pages.json").expect("refused"), None);
        assert_eq!(
            sample.file("main", "pages/none.html").expect("absent"),
            None
        );
        assert!(
            sample.state("gone").expect("no folder").pages.is_empty(),
            "expected nothing"
        );
        assert!(sample.save("main", "x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
