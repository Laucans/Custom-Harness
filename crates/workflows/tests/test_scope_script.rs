//! `scripts/test-scope.sh`, as `init-repo` installs it, run for real against a
//! throwaway Cargo workspace laid out like a target repository: which tests a
//! change gets.
//!
//! Hermetic: the workspace has path dependencies only, so `cargo tree` needs
//! no network, and `TEST_SCOPE_DRY_RUN` prints the `cargo test` line instead
//! of compiling anything. Needs `bash`, `git` and `cargo` on the path — what
//! running this suite already needs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_workflows::init_repo::data::install::FILES;

/// A target repository on `task`, branched from `main_agent`, removed on drop.
struct Repo {
    root: PathBuf,
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn krate(root: &Path, dir: &str, name: &str, dependencies: &str) {
    write(
        root,
        &format!("{dir}/Cargo.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{dependencies}"
        ),
    );
    write(root, &format!("{dir}/src/lib.rs"), "");
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(status.status.success(), "git {args:?}: {status:?}");
}

const MEMBERS: &str = r#""crates/credit/capabilities/risk", "crates/credit/capabilities/score", "crates/credit/server", "crates/dataguard/data-capabilities/limit""#;

/// Two Capabilities — `risk` with an infrastructure crate depending on it —
/// and one DataCapability.
fn repo(case: &str) -> Repo {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("test-scope-{case}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).expect("mkdir");
    let script = FILES
        .iter()
        .find(|asset| asset.path == "scripts/test-scope.sh")
        .expect("init-repo installs the script");
    write(&root, "scripts/test-scope.sh", script.content);
    write(
        &root,
        "Cargo.toml",
        &format!("[workspace]\nresolver = \"2\"\nmembers = [{MEMBERS}]\n"),
    );
    write(&root, ".gitignore", "target\n");
    krate(&root, "crates/credit/capabilities/risk", "risk", "");
    krate(&root, "crates/credit/capabilities/score", "score", "");
    krate(
        &root,
        "crates/credit/server",
        "server",
        "risk = { path = \"../capabilities/risk\" }\n",
    );
    krate(
        &root,
        "crates/dataguard/data-capabilities/limit",
        "limit",
        "",
    );
    git(&root, &["init", "-q", "-b", "main_agent"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    git(&root, &["checkout", "-qb", "task"]);
    Repo { root }
}

/// The `cargo test` line the script settles on.
fn scope(repo: &Repo, args: &[&str]) -> String {
    let out = Command::new("bash")
        .arg("scripts/test-scope.sh")
        .args(args)
        .env("TEST_SCOPE_DRY_RUN", "1")
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn touch(repo: &Repo, path: &str) {
    write(&repo.root, path, "// changed\n");
}

#[test]
fn a_change_inside_one_capability_runs_its_tests_alone() {
    let repo = repo("one");
    touch(&repo, "crates/credit/capabilities/score/src/lib.rs");
    assert_eq!(
        scope(&repo, &["main_agent"]),
        "cargo test --all-features -p score"
    );
}

#[test]
fn a_crate_depending_on_the_capability_is_tested_with_it() {
    let repo = repo("dependent");
    touch(&repo, "crates/credit/capabilities/risk/src/lib.rs");
    assert_eq!(
        scope(&repo, &["main_agent"]),
        "cargo test --all-features -p risk -p server"
    );
}

#[test]
fn a_data_capability_runs_the_whole_workspace() {
    let repo = repo("data");
    touch(&repo, "crates/credit/capabilities/risk/src/lib.rs");
    touch(&repo, "crates/dataguard/data-capabilities/limit/src/lib.rs");
    assert_eq!(
        scope(&repo, &["main_agent"]),
        "cargo test --workspace --all-features"
    );
}

#[test]
fn anything_outside_the_capabilities_runs_the_whole_workspace() {
    let repo = repo("outside");
    touch(&repo, "crates/credit/server/src/lib.rs");
    assert_eq!(
        scope(&repo, &["main_agent"]),
        "cargo test --workspace --all-features"
    );
    let nested = self::repo("nested");
    touch(&nested, "crates/credit/sub/capabilities/x/src/lib.rs");
    assert_eq!(
        scope(&nested, &["main_agent"]),
        "cargo test --workspace --all-features"
    );
}

#[test]
fn no_base_or_an_unknown_one_runs_the_whole_workspace() {
    let repo = repo("base");
    touch(&repo, "crates/credit/capabilities/score/src/lib.rs");
    assert_eq!(scope(&repo, &[]), "cargo test --workspace --all-features");
    assert_eq!(scope(&repo, &[""]), "cargo test --workspace --all-features");
    assert_eq!(
        scope(&repo, &["origin/nowhere"]),
        "cargo test --workspace --all-features"
    );
}

#[test]
fn a_new_capability_registered_in_the_workspace_stays_scoped() {
    let repo = repo("fresh");
    krate(&repo.root, "crates/credit/capabilities/fresh", "fresh", "");
    write(
        &repo.root,
        "Cargo.toml",
        &format!(
            "[workspace]\nresolver = \"2\"\nmembers = [{MEMBERS},\n  \"crates/credit/capabilities/fresh\",\n]\n"
        ),
    );
    assert_eq!(
        scope(&repo, &["main_agent"]),
        "cargo test --all-features -p fresh"
    );
}

#[test]
fn any_other_change_to_the_root_manifest_runs_the_whole_workspace() {
    let repo = repo("root");
    touch(&repo, "crates/credit/capabilities/score/src/lib.rs");
    write(
        &repo.root,
        "Cargo.toml",
        &format!(
            "[workspace]\nresolver = \"2\"\nmembers = [{MEMBERS}]\n\n[workspace.dependencies]\nserde = \"1\"\n"
        ),
    );
    assert_eq!(
        scope(&repo, &["main_agent"]),
        "cargo test --workspace --all-features"
    );
}

#[test]
fn prose_alone_runs_nothing_and_arguments_reach_cargo() {
    let repo = repo("prose");
    write(&repo.root, "docs/notes.md", "words\n");
    assert_eq!(scope(&repo, &["main_agent"]), "");
    touch(&repo, "crates/credit/capabilities/score/src/lib.rs");
    assert_eq!(
        scope(&repo, &["main_agent", "--", "one_test"]),
        "cargo test --all-features -p score one_test"
    );
}
