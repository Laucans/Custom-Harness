//! The harness lends its own skills to the checkout a session runs in.
//!
//! **Why this exists.** A session is spawned with `current_dir` set to the
//! mounted clone (`ClaudeCliFactory::new(workspace.root(), …)`), and
//! `claude` resolves project skills from `.claude/skills` relative to that
//! directory. So a session working a target repo looks for its skills *in
//! that repo* — and would not find them, however many the harness carries.
//!
//! The wrong fix, which the audit used to recommend in so many words, is to
//! commit the skills into every target repository: the same files forked
//! into every project, drifting from the harness that actually runs them,
//! and a `/code` whose instructions depend on which repo you happen to be
//! in. The harness orchestrates; a target repo holds its own code and
//! nothing of the harness.
//!
//! So the launcher lends them instead: every mount copies the harness's
//! `.claude/skills` into the clone and hides the copy from git. The copy is
//! refreshed on every run, so it cannot drift, and `git` never sees it, so
//! it cannot be committed to the target by accident.
//!
//! **All of them, not a list.** Picking which skills to lend would be a
//! third list to keep in step with the loop's table and the audit's copy —
//! and those two had already drifted once. An unused `SKILL.md` sitting in
//! a clone is inert: a session opens a skill because a prompt names it,
//! never because the file is present.

use std::path::Path;

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::disk::Disk;
use harness_core::traces::Logbook;

/// Where skills live, under a repository root.
const SKILLS: &str = ".claude/skills";

/// What goes in the clone's local exclude file, so `git status` stays clean.
///
/// `.git/info/exclude` and not `.gitignore`: the first is per-clone and
/// never committed, which is exactly what a borrowed file wants. A
/// `.gitignore` entry would be a change to the target repository.
const EXCLUDE_LINE: &str = "/.claude/";

/// Copies the harness's skills into this checkout, and hides them from git.
///
/// A no-op when `here` and `root` are the same directory: a run working in
/// place already has them.
///
/// # Errors
/// [`Halt::Failed`] if a directory cannot be created or a file written —
/// a session without its skills would halt on the preflight gate anyway,
/// and later rather than here.
pub fn lend(here: &Path, root: &Path, disk: &dyn Disk, log: &Logbook) -> Outcome<()> {
    if here == root {
        return Ok(());
    }
    let from = here.join(SKILLS);
    if !disk.exists(&from) {
        return Err(Halt::Failed(format!(
            "the harness carries no {SKILLS} to lend — expected it at {}",
            from.display()
        )));
    }
    hide_from_git(root, disk)?;

    let into = root.join(SKILLS);
    let mut lent = 0_usize;
    for name in disk.dir_names(&from) {
        let file = from.join(&name).join("SKILL.md");
        let Some(text) = disk.read_to_string(&file) else {
            // A directory under `skills/` without a `SKILL.md` is not a
            // skill; nothing to lend and nothing to complain about.
            continue;
        };
        let target = into.join(&name);
        disk.create_dir_all(&target)?;
        disk.write_to_string(&target.join("SKILL.md"), &text)?;
        lent += 1;
    }
    log.debug(&format!(
        "skills: lent {lent} from the harness into {}",
        root.display()
    ));
    Ok(())
}

/// Adds the exclude line to the clone's `.git/info/exclude`, once.
///
/// Read-modify-write rather than append: the `Disk` port writes whole
/// files, and this one is a handful of lines.
fn hide_from_git(root: &Path, disk: &dyn Disk) -> Outcome<()> {
    let info = root.join(".git/info");
    let exclude = info.join("exclude");
    let current = disk.read_to_string(&exclude).unwrap_or_default();
    if current.lines().any(|line| line.trim() == EXCLUDE_LINE) {
        return Ok(());
    }
    disk.create_dir_all(&info)?;
    let next = if current.is_empty() {
        format!("{EXCLUDE_LINE}\n")
    } else if current.ends_with('\n') {
        format!("{current}{EXCLUDE_LINE}\n")
    } else {
        format!("{current}\n{EXCLUDE_LINE}\n")
    };
    disk.write_to_string(&exclude, &next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::Outcome as CoreOutcome;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// A disk that remembers what it was asked to write.
    #[derive(Default)]
    struct FakeDisk {
        files: HashMap<PathBuf, String>,
        dirs: HashMap<PathBuf, Vec<String>>,
        wrote: RefCell<Vec<(PathBuf, String)>>,
        created: RefCell<Vec<PathBuf>>,
    }

    impl Disk for FakeDisk {
        fn exists(&self, path: &Path) -> bool {
            self.files.contains_key(path) || self.dirs.contains_key(path)
        }
        fn create_dir_all(&self, path: &Path) -> CoreOutcome<()> {
            self.created.borrow_mut().push(path.to_path_buf());
            Ok(())
        }
        fn remove_dir_all(&self, _path: &Path) -> CoreOutcome<()> {
            Ok(())
        }
        fn dir_names(&self, path: &Path) -> Vec<String> {
            self.dirs.get(path).cloned().unwrap_or_default()
        }
        fn read_to_string(&self, path: &Path) -> Option<String> {
            self.files.get(path).cloned()
        }
        fn write_to_string(&self, path: &Path, content: &str) -> CoreOutcome<()> {
            self.wrote
                .borrow_mut()
                .push((path.to_path_buf(), content.to_string()));
            Ok(())
        }
    }

    fn harness_with(skills: &[&str]) -> FakeDisk {
        let mut disk = FakeDisk::default();
        disk.dirs.insert(
            PathBuf::from("/harness/.claude/skills"),
            skills.iter().map(|s| (*s).to_string()).collect(),
        );
        for skill in skills {
            disk.files.insert(
                PathBuf::from(format!("/harness/.claude/skills/{skill}/SKILL.md")),
                format!("# {skill}\n"),
            );
        }
        disk
    }

    #[test]
    fn every_skill_the_harness_carries_lands_in_the_clone() {
        let disk = harness_with(&["code", "commit"]);
        lend(
            Path::new("/harness"),
            Path::new("/clone"),
            &disk,
            &Logbook::null(),
        )
        .expect("lent");
        let wrote = disk.wrote.borrow();
        assert!(wrote.iter().any(|(path, text)| path
            == Path::new("/clone/.claude/skills/code/SKILL.md")
            && text == "# code\n"));
        assert!(
            wrote
                .iter()
                .any(|(path, _)| path == Path::new("/clone/.claude/skills/commit/SKILL.md"))
        );
    }

    #[test]
    fn the_copy_is_hidden_from_git_so_the_next_mount_does_not_read_it_as_work() {
        // `git status --porcelain` lists untracked files, and the mount
        // guard refuses a workspace that "carries local work". Without this
        // line the second run of any workflow would halt.
        let disk = harness_with(&["code"]);
        lend(
            Path::new("/harness"),
            Path::new("/clone"),
            &disk,
            &Logbook::null(),
        )
        .expect("lent");
        let wrote = disk.wrote.borrow();
        let (_, text) = wrote
            .iter()
            .find(|(path, _)| path == Path::new("/clone/.git/info/exclude"))
            .expect("the exclude file");
        assert_eq!(text, "/.claude/\n");
    }

    #[test]
    fn an_exclude_file_that_already_says_it_is_left_alone() {
        let mut disk = harness_with(&["code"]);
        disk.files.insert(
            PathBuf::from("/clone/.git/info/exclude"),
            "# stuff\n/.claude/\n".to_string(),
        );
        lend(
            Path::new("/harness"),
            Path::new("/clone"),
            &disk,
            &Logbook::null(),
        )
        .expect("lent");
        assert!(
            !disk
                .wrote
                .borrow()
                .iter()
                .any(|(path, _)| path == Path::new("/clone/.git/info/exclude")),
            "rewritten for nothing"
        );
    }

    #[test]
    fn an_existing_exclude_file_keeps_what_it_had() {
        let mut disk = harness_with(&["code"]);
        disk.files.insert(
            PathBuf::from("/clone/.git/info/exclude"),
            "build/\n".to_string(),
        );
        lend(
            Path::new("/harness"),
            Path::new("/clone"),
            &disk,
            &Logbook::null(),
        )
        .expect("lent");
        let wrote = disk.wrote.borrow();
        let (_, text) = wrote
            .iter()
            .find(|(path, _)| path == Path::new("/clone/.git/info/exclude"))
            .expect("the exclude file");
        assert_eq!(text, "build/\n/.claude/\n");
    }

    #[test]
    fn a_directory_without_a_skill_file_is_not_a_skill() {
        let mut disk = harness_with(&["code"]);
        disk.dirs.insert(
            PathBuf::from("/harness/.claude/skills"),
            vec!["code".to_string(), "notes".to_string()],
        );
        lend(
            Path::new("/harness"),
            Path::new("/clone"),
            &disk,
            &Logbook::null(),
        )
        .expect("lent");
        assert!(
            !disk
                .wrote
                .borrow()
                .iter()
                .any(|(path, _)| path.to_string_lossy().contains("notes")),
            "a directory with no SKILL.md was copied"
        );
    }

    #[test]
    fn a_run_working_in_place_lends_nothing_to_itself() {
        let disk = harness_with(&["code"]);
        lend(
            Path::new("/harness"),
            Path::new("/harness"),
            &disk,
            &Logbook::null(),
        )
        .expect("nothing to do");
        assert!(disk.wrote.borrow().is_empty());
    }

    #[test]
    fn a_harness_without_a_skills_directory_says_so_rather_than_lending_nothing() {
        let disk = FakeDisk::default();
        let err = lend(
            Path::new("/harness"),
            Path::new("/clone"),
            &disk,
            &Logbook::null(),
        )
        .expect_err("must fail");
        assert!(err.reason().contains(".claude/skills"));
    }
}
