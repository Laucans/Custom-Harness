//! A new factory made outside the view: its repository through `gh`, its
//! folder a `git worktree` of the harness checkout the view runs from, and
//! `harness init-repo` run in it.
//!
//! A worktree, not an empty folder: the watch lends the harness's
//! `.claude/skills` to every clone it mounts, from the checkout it runs in,
//! and the view finds its factory's root by its `.git`. Detached, so no
//! branch of the harness is made for it.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ports::{Foundry, Initialized};

/// The program's output, or why it failed — its own words.
fn output(command: &mut Command, what: &str) -> Result<String, String> {
    let out = command
        .output()
        .map_err(|e| format!("{what} cannot start: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let said = if err.trim().is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        err.into_owned()
    };
    Err(format!("{what} failed: {}", said.trim()))
}

/// `gh`, `git` and the harness binary, from the harness checkout.
pub struct Cli {
    /// The harness checkout the view runs from: what a factory's folder is
    /// a worktree of.
    checkout: PathBuf,
    /// Where the harness binary may be, in order of preference.
    harness: Vec<PathBuf>,
    /// The GitHub CLI.
    gh: PathBuf,
}

impl Cli {
    /// A foundry working from `checkout`, starting the first of `harness`
    /// that exists.
    #[must_use]
    pub fn new(checkout: PathBuf, harness: Vec<PathBuf>) -> Self {
        Self {
            checkout,
            harness,
            gh: PathBuf::from("gh"),
        }
    }

    /// The harness binary, by absolute path: the factory's `.env.local`
    /// names it, and the factory runs from another folder.
    fn harness(&self) -> Result<PathBuf, String> {
        self.harness
            .iter()
            .find(|path| path.is_file())
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
            .ok_or_else(|| {
                "no harness binary to run init-repo with: build it (`cargo build -p harness-launcher`) \
                 or name it with --harness-bin"
                    .to_string()
            })
    }
}

impl Foundry for Cli {
    fn owners(&self) -> Result<Vec<String>, String> {
        let login = output(
            Command::new(&self.gh).args(["api", "user", "--jq", ".login"]),
            "gh api user",
        )?;
        let orgs = output(
            Command::new(&self.gh).args(["api", "user/orgs", "--jq", ".[].login"]),
            "gh api user/orgs",
        )
        .unwrap_or_default();
        Ok(std::iter::once(login.as_str())
            .chain(orgs.lines())
            .map(str::trim)
            .filter(|owner| !owner.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn create_repository(
        &self,
        owner: &str,
        name: &str,
        private: bool,
        description: &str,
    ) -> Result<String, String> {
        let slug = format!("{owner}/{name}");
        let mut command = Command::new(&self.gh);
        command.args([
            "repo",
            "create",
            &slug,
            if private { "--private" } else { "--public" },
            "--add-readme",
        ]);
        if !description.trim().is_empty() {
            command.args(["--description", description.trim()]);
        }
        output(&mut command, "gh repo create")?;
        Ok(slug)
    }

    fn make_folder(&self, root: &Path) -> Result<(), String> {
        if root.exists() {
            return Err(format!("{} exists already", root.display()));
        }
        if let Some(parent) = root.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("{} cannot be made: {e}", parent.display()))?;
        }
        output(
            Command::new("git")
                .arg("-C")
                .arg(&self.checkout)
                .args(["worktree", "add", "--detach"])
                .arg(root)
                .arg("HEAD"),
            "git worktree add",
        )
        .map(|_| ())
    }

    fn init_repo(&self, root: &Path, slug: &str, branch: &str) -> Result<Initialized, String> {
        let harness = self.harness()?;
        let out = Command::new(&harness)
            .args(["init-repo", slug, "--branch", branch])
            .current_dir(root)
            // The factory's own link is what init-repo writes; the one this
            // view runs on is another factory's.
            .env_remove("TARGET_REPO_URL")
            .env("INTEGRATION_BRANCH", branch)
            .output()
            .map_err(|e| format!("{} cannot start: {e}", harness.display()))?;
        let report = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let ready = match out.status.code() {
            Some(0) => true,
            // Gaps remain, and the report names them.
            Some(1) if !report.is_empty() => false,
            _ => {
                return Err(format!(
                    "harness init-repo failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        };
        let bin = harness.display();
        let mut lines = String::from(
            "\n# --- the factory view, written when the view created this factory ---\n",
        );
        let _ = writeln!(lines, "HARNESS_BIN={bin}");
        let _ = writeln!(lines, "HARNESS_WATCH_COMMAND={bin} watch --force-reset");
        let _ = writeln!(lines, "HARNESS_TRIAL_COMMAND={bin} trial");
        let env = root.join(".env.local");
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&env)
            .and_then(|mut file| file.write_all(lines.as_bytes()))
            .map_err(|e| format!("{} cannot be written: {e}", env.display()))?;
        Ok(Initialized { ready, report })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    /// A folder of its own for a test, emptied.
    fn folder(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("harness-foundry-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    /// An executable `name` in `dir` that runs `body` (sh).
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let bin = dir.join(name);
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).expect("script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("mode");
        bin
    }

    fn sh(body: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", body]);
        command
    }

    #[test]
    fn a_program_answers_its_output_or_its_own_words_when_it_fails() {
        assert_eq!(output(&mut sh("echo ' hi '"), "x"), Ok("hi".to_string()));
        assert_eq!(
            output(&mut sh("echo out; echo err >&2; exit 1"), "x"),
            Err("x failed: err".to_string())
        );
        assert_eq!(
            output(&mut sh("echo out; exit 1"), "x"),
            Err("x failed: out".to_string())
        );
        assert!(output(&mut Command::new("/nowhere/at/all"), "x").is_err());
    }

    #[test]
    fn the_owners_are_the_login_then_the_organisations() {
        let dir = folder("owners");
        let gh = script(
            &dir,
            "gh",
            "case \"$2\" in user) echo me ;; user/orgs) printf 'org1\\n\\n org2 \\n' ;; esac",
        );
        let cli = Cli {
            gh,
            ..Cli::new(dir.clone(), Vec::new())
        };
        assert_eq!(
            cli.owners(),
            Ok(vec![
                "me".to_string(),
                "org1".to_string(),
                "org2".to_string()
            ])
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_repository_is_created_with_a_readme_its_visibility_and_its_description() {
        let dir = folder("create");
        let said = dir.join("said");
        let gh = script(&dir, "gh", &format!("echo \"$@\" >> {}", said.display()));
        let cli = Cli {
            gh,
            ..Cli::new(dir.clone(), Vec::new())
        };
        assert_eq!(
            cli.create_repository("me", "app", true, " a ledger "),
            Ok("me/app".to_string())
        );
        assert_eq!(
            cli.create_repository("me", "pub", false, "  "),
            Ok("me/pub".to_string())
        );
        let lines = std::fs::read_to_string(&said).expect("said");
        assert_eq!(
            lines,
            "repo create me/app --private --add-readme --description a ledger\nrepo create me/pub --public --add-readme\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn init_repo_runs_in_the_folder_says_whether_gaps_remain_and_links_the_binary() {
        let dir = folder("init");
        let ready = script(
            &dir,
            "ready",
            "pwd; echo \"$@\"; echo \"branch=$INTEGRATION_BRANCH\"",
        );
        let cli = Cli::new(dir.clone(), vec![dir.join("missing"), ready.clone()]);
        let done = cli.init_repo(&dir, "me/app", "main_agent").expect("ran");
        assert!(done.ready);
        assert!(
            done.report.contains("init-repo me/app --branch main_agent"),
            "{}",
            done.report
        );
        assert!(done.report.contains("branch=main_agent"), "{}", done.report);
        assert!(
            done.report.contains("harness-foundry-init"),
            "{}",
            done.report
        );
        let env = std::fs::read_to_string(dir.join(".env.local")).expect("written");
        let bin = ready.canonicalize().expect("real");
        assert!(
            env.contains(&format!("HARNESS_BIN={}\n", bin.display())),
            "{env}"
        );
        assert!(
            env.contains(&format!(
                "HARNESS_WATCH_COMMAND={} watch --force-reset\n",
                bin.display()
            )),
            "{env}"
        );
        assert!(
            env.contains(&format!("HARNESS_TRIAL_COMMAND={} trial\n", bin.display())),
            "{env}"
        );

        let gaps = script(&dir, "gaps", "echo 'one gap'; exit 1");
        let left = Cli::new(dir.clone(), vec![gaps])
            .init_repo(&dir, "me/app", "main_agent")
            .expect("ran");
        assert_eq!((left.ready, left.report.as_str()), (false, "one gap"));
        let silent = script(&dir, "silent", "echo 'broken' >&2; exit 1");
        let why = Cli::new(dir.clone(), vec![silent])
            .init_repo(&dir, "me/app", "main_agent")
            .expect_err("failed");
        assert!(why.contains("broken"), "{why}");
        let halted = script(&dir, "halted", "echo report; exit 2");
        assert!(
            Cli::new(dir.clone(), vec![halted])
                .init_repo(&dir, "me/app", "main_agent")
                .is_err()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_factorys_folder_is_a_detached_worktree_of_the_checkout() {
        let dir = folder("worktree");
        let checkout = dir.join("harness");
        std::fs::create_dir_all(&checkout).expect("dir");
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&checkout)
                .args(args)
                .output()
                .expect("git");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "first",
        ]);
        let root = dir.join("factories").join("app");
        Cli::new(checkout, Vec::new())
            .make_folder(&root)
            .expect("made");
        assert!(root.join(".git").is_file(), "a worktree's .git is a file");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_harness_binary_is_said_before_anything_runs() {
        let cli = Cli::new(
            PathBuf::from("/nowhere"),
            vec![PathBuf::from("/nowhere/harness")],
        );
        let why = cli
            .init_repo(Path::new("/nowhere/f"), "me/f", "main_agent")
            .expect_err("no binary");
        assert!(why.contains("no harness binary"), "{why}");
    }

    #[test]
    fn a_folder_that_exists_is_never_made_again() {
        let here = std::env::temp_dir();
        let why = Cli::new(here.clone(), Vec::new())
            .make_folder(&here)
            .expect_err("exists");
        assert!(why.contains("exists already"), "{why}");
    }
}
