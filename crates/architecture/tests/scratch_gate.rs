//! The `scratch-databases-shared` gate of `gates.yml`, as `init-repo`
//! installs it, run for real against a throwaway repository: a test file a
//! change adds takes its database from `platform-scratch-db`, while the
//! tests written before the guard may still be edited.
//!
//! Hermetic: `bash` and `git` only, what running this suite already needs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::content_of;

/// A repository whose base commit holds a test made before the guard.
struct Repo {
    root: PathBuf,
    base: String,
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const BY_HAND: &str = "#[tokio::test]\nasync fn moved() {\n    client.batch_execute(&format!(\"CREATE DATABASE {name}\")).await.unwrap();\n}\n";
const GUARDED: &str = "#[tokio::test]\nasync fn moved() {\n    let db = platform_scratch_db::ScratchDb::create(\"moved\").await.unwrap();\n}\n";

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo(case: &str) -> Repo {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "scratch-gate-{case}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("mkdir");
    git(&root, &["init", "-q", "-b", "main_agent"]);
    write(
        &root,
        "crates/table/data-capabilities/move-token/tests/postgres.rs",
        BY_HAND,
    );
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    Repo { root, base }
}

/// The gate's `run: |` block, as the step runs it.
fn script() -> String {
    let gates = content_of(".github/workflows/gates.yml").expect("installed");
    let step = gates
        .split("- id: scratch-databases-shared\n")
        .nth(1)
        .expect("the gate");
    step.split("run: |\n")
        .nth(1)
        .expect("its script")
        .lines()
        .take_while(|line| line.starts_with("          ") || line.trim().is_empty())
        .map(|line| line.get(10..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Commits `files` on top of the base, then runs the gate against it as a
/// task's pull request.
fn change(repo: &Repo, files: &[(&str, &str)]) -> (bool, String) {
    change_from(repo, "task/27-drop-token", files)
}

/// Commits `files` on top of the base, then runs the gate against it as a
/// pull request from `head`.
fn change_from(repo: &Repo, head: &str, files: &[(&str, &str)]) -> (bool, String) {
    for (path, text) in files {
        write(&repo.root, path, text);
    }
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-qm", "change", "--allow-empty"]);
    let out = Command::new("bash")
        .arg("-c")
        .arg(script())
        .env("BASE", &repo.base)
        .env("HEAD_REF", head)
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn a_new_test_creating_its_database_by_hand_fails() {
    let repo = repo("by-hand");
    let (ok, said) = change(
        &repo,
        &[(
            "crates/table/data-capabilities/drop-token/tests/postgres.rs",
            BY_HAND,
        )],
    );
    assert!(!ok, "{said}");
    assert!(
        said.contains("::error file=crates/table/data-capabilities/drop-token/tests/postgres.rs::"),
        "{said}"
    );
}

#[test]
fn a_new_test_through_the_guard_and_an_edited_old_one_pass() {
    let repo = repo("guarded");
    let (ok, said) = change(
        &repo,
        &[
            (
                "crates/table/data-capabilities/drop-token/tests/postgres.rs",
                GUARDED,
            ),
            (
                "crates/table/data-capabilities/move-token/tests/postgres.rs",
                &format!("{BY_HAND}\n// edited\n"),
            ),
        ],
    );
    assert!(ok, "{said}");
}

#[test]
fn a_source_holding_a_test_counts_and_one_holding_none_does_not() {
    let repo = repo("sources");
    let (ok, said) = change(
        &repo,
        &[(
            "crates/data/src/tools.rs",
            "pub const SQL: &str = \"create  database x\";\n",
        )],
    );
    assert!(ok, "no test in it: {said}");
    let (ok, said) = change(
        &repo,
        &[(
            "crates/data/src/fixture.rs",
            "#[cfg(test)]\nmod tests {\n    const SQL: &str = \"create database x\";\n}\n",
        )],
    );
    assert!(!ok, "a test module creating its database: {said}");
}

#[test]
fn a_new_test_reaching_postgres_by_its_own_connection_fails_and_through_the_kit_passes() {
    let first = repo("own-connection");
    let own = "#[tokio::test]\nasync fn moved() {\n    let url = std::env::var(\"DATABASE_URL\").unwrap();\n    let (client, c) = tokio_postgres::connect(&url, NoTls).await.unwrap();\n}\n";
    let (ok, said) = change(
        &first,
        &[(
            "crates/table/data-capabilities/drop-token/tests/postgres.rs",
            own,
        )],
    );
    assert!(!ok, "{said}");
    assert!(said.contains("platform_test_kit::Scratch"), "{said}");
    let repo = repo("kit");
    let kit = "use platform_test_kit::Scratch;\n#[tokio::test]\nasync fn moved() {\n    let scratch = Scratch::new(\"moved\").await;\n    std::env::var(\"DATABASE_URL\").ok();\n}\n";
    let adapter = "pub fn url() -> String { std::env::var(\"DATABASE_URL\").unwrap_or_default() }\n#[cfg(test)]\nmod tests {}\n";
    let (ok, said) = change(
        &repo,
        &[
            (
                "crates/table/data-capabilities/drop-token/tests/postgres.rs",
                kit,
            ),
            ("crates/data/src/url.rs", adapter),
            ("crates/platform/test-kit/tests/own.rs", own),
        ],
    );
    assert!(
        ok,
        "the kit, a source outside tests/, the kit's own tests: {said}"
    );
}

#[test]
fn the_check_reads_code_not_words_comments_and_a_struct_of_the_same_name_aside() {
    let memory = "//! Over memory (`DATABASE_URL=memory`): no connection, no Scratch.\n// std::env::var(\"DATABASE_URL\") is never read here.\n#[tokio::test]\nasync fn moved() {}\n";
    let first = repo("comments");
    let (ok, said) = change(
        &first,
        &[(
            "crates/live/data-capabilities/drop-token/tests/engine.rs",
            memory,
        )],
    );
    assert!(ok, "a comment reaches no database: {said}");
    let word = "// no Scratch needed\n#[tokio::test]\nasync fn moved() {\n    let url = std::env::var(\"DATABASE_URL\").unwrap();\n}\n";
    let own = "struct Scratch { client: Client }\nimpl Scratch {\n    async fn new() -> Self {\n        let (client, c) = tokio_postgres::connect(&url, NoTls).await.unwrap();\n    }\n}\n";
    let repo = repo("names");
    let (ok, said) = change(
        &repo,
        &[
            ("crates/table/data-capabilities/a/tests/postgres.rs", word),
            ("crates/table/data-capabilities/b/tests/postgres.rs", own),
        ],
    );
    assert!(!ok, "{said}");
    for file in ["a/tests/postgres.rs", "b/tests/postgres.rs"] {
        assert!(
            said.contains(&format!(
                "data-capabilities/{file}::a test that reaches PostgreSQL"
            )),
            "{file} in {said}"
        );
    }
}

#[test]
fn the_guard_itself_and_files_outside_crates_are_not_judged() {
    let repo = repo("guard");
    let (ok, said) = change(
        &repo,
        &[
            ("crates/platform/scratch-db/tests/drop.rs", BY_HAND),
            ("tests/rules/business/tests/db.rs", BY_HAND),
            ("scripts/seed.sh", "psql -c 'CREATE DATABASE x'\n"),
        ],
    );
    assert!(ok, "{said}");
}

#[test]
fn no_base_to_compare_with_passes() {
    let repo = repo("no-base");
    let out = Command::new("bash")
        .arg("-c")
        .arg(script())
        .env("BASE", "0000000000000000000000000000000000000000")
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("no base to compare with"));
}

#[test]
fn a_milestone_s_pull_request_is_not_judged_again() {
    // ForgeDesMondes' milestone 28 adds a task's tests/postgres.rs written
    // before the gate: landing it must not turn red.
    let repo = repo("milestone");
    let (ok, said) = change_from(
        &repo,
        "milestone/28",
        &[(
            "crates/session/data-capabilities/close-session/tests/postgres.rs",
            BY_HAND,
        )],
    );
    assert!(ok, "{said}");
    assert!(said.contains("judged on its own"), "{said}");
}

#[test]
fn a_file_the_base_removed_since_the_branch_is_not_one_the_change_adds() {
    // Measured from the merge base: the base deleted a by-hand test after
    // the branch was cut, and the branch, still holding it, adds nothing.
    let repo = repo("moved-base");
    git(&repo.root, &["checkout", "-qb", "task/27"]);
    write(&repo.root, "crates/data/README.md", "notes\n");
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-qm", "task"]);
    git(&repo.root, &["checkout", "-q", "main_agent"]);
    git(
        &repo.root,
        &[
            "rm",
            "-q",
            "crates/table/data-capabilities/move-token/tests/postgres.rs",
        ],
    );
    git(&repo.root, &["commit", "-qm", "moved to the guard"]);
    let moved = git(&repo.root, &["rev-parse", "HEAD"]);
    git(&repo.root, &["checkout", "-q", "task/27"]);
    let out = Command::new("bash")
        .arg("-c")
        .arg(script())
        .env("BASE", &moved)
        .env("HEAD_REF", "task/27")
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn the_gate_runs_on_pull_requests_only() {
    let gates = content_of(".github/workflows/gates.yml").expect("installed");
    let step = gates
        .split("- id: scratch-databases-shared\n")
        .nth(1)
        .expect("the gate");
    assert!(
        step.contains("if: ${{ !cancelled() && github.event_name == 'pull_request' }}"),
        "a push lands what a pull request judged"
    );
    assert!(step.contains("BASE: ${{ github.event.pull_request.base.sha }}\n"));
}
