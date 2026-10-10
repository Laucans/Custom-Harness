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
//! session edits them in place: there is no repository to save to.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use crate::adapters::git_units::{Mirror, run};
use crate::domain::drafts::fingerprint;
use crate::domain::mockup;
use crate::ports::{MockPage, MockState, Mockups};

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

    fn at(folder: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
        run(Command::new("git").arg("-C").arg(folder).args(args), None)
    }

    /// Something under `mock/` differs from the commit.
    fn dirty(folder: &Path) -> Result<bool, String> {
        let out = Self::at(folder, &["status", "--porcelain", "--", "mock"])?;
        Ok(!String::from_utf8_lossy(&out).trim().is_empty())
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
        stamp: stamp_of(&folder.join("mock")),
    })
}

impl Mockups for GitMockups {
    fn state(&self, branch: &str) -> Result<MockState, String> {
        if let Some(folder) = self.worktree(branch) {
            let mut state = state_of_folder(branch, &folder)?;
            state.edited = Self::dirty(&folder)?;
            state.saved = Self::saved_as(&folder);
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

    fn discard(&self, branch: &str) -> Result<(), String> {
        if let Some(folder) = self.worktree(branch) {
            let at = folder.to_string_lossy().into_owned();
            self.mirror
                .git(&["worktree", "remove", "--force", &at], None)?;
        }
        let _ = self.mirror.git(&["worktree", "prune"], None);
        Ok(())
    }
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

    fn discard(&self, _branch: &str) -> Result<(), String> {
        Err("the sample is not a checkout: nothing to discard".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_without_pages_json_has_no_mockup_and_a_stamp_follows_the_files() {
        let dir = std::env::temp_dir().join(format!("harness-mockups-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("main/mock/pages")).expect("temp dir");
        let sample = DirMockups::new(&dir);
        let none = sample.state("main").expect("read");
        assert!(!none.present && none.pages.is_empty());
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
        assert!(sample.state("gone").expect("no folder").pages.is_empty());
        assert!(sample.save("main", "x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
