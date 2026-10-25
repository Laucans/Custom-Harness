//! `scripts/rules-gate.mjs contest` against node and git, on a repository of
//! its own and a stub `gh` on the path: a `CONTEST.md` the change added
//! opens its contest and fails; one the change took from its base is the
//! base's — warned, passed, and no issue opened.
//! Skipped, loudly, when `node` is not on the path.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const GATE: &str = include_str!("../assets/rules-gate.mjs");

/// A `gh` that writes every call down and answers the two the gate makes.
const STUB_GH: &str = r#"#!/bin/sh
echo "$*" >> "$GH_CALLS"
case "$1 $2" in
  "issue list") echo '[]' ;;
  "issue create") echo 'https://github.com/o/r/issues/9' ;;
esac
"#;

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

/// A repository whose `main` carries `CONTEST.md` when `on_base`, and a
/// branch `change` where `edit` was made and committed; the stub `gh` in
/// `bin/`.
fn repo(case: &str, on_base: bool, edit: &dyn Fn(&Path)) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rules-gate-contest-{case}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("root");
    git(&root, &["init", "-q", "-b", "main"]);
    write(&root, "scripts/rules-gate.mjs", GATE);
    write(&root, "src/lib.rs", "// the product\n");
    if on_base {
        write(&root, "CONTEST.md", "R10 cannot hold: slugs in commands\n");
    }
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "change"]);
    edit(&root);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "change", "--allow-empty"]);
    write(&root, "bin/gh", STUB_GH);
    let made = Command::new("chmod")
        .args(["+x", "bin/gh"])
        .current_dir(&root)
        .status()
        .expect("chmod runs");
    assert!(made.success());
    root
}

/// The gate's verdict, what it printed, and the `gh` calls it made.
fn contest(root: &Path, base: Option<&str>) -> (bool, String, String) {
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let calls = root.join("gh-calls.txt");
    let mut command = Command::new("node");
    command
        .arg("scripts/rules-gate.mjs")
        .arg("contest")
        .arg("7")
        .args(base)
        .current_dir(root)
        .env("PATH", path)
        .env("GH_CALLS", &calls);
    let out = command.output().expect("node runs");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        fs::read_to_string(&calls).unwrap_or_default(),
    )
}

fn node_present() -> bool {
    let present = Command::new("node").arg("--version").output().is_ok();
    if !present {
        eprintln!("rules_gate_contest: node is not on the path — skipped");
    }
    present
}

#[test]
fn a_contest_md_the_change_added_opens_its_contest_and_fails() {
    if !node_present() {
        return;
    }
    let root = repo("added", false, &|root| {
        write(root, "CONTEST.md", "R3 cannot hold\n");
    });
    let (passed, said, calls) = contest(&root, Some("main"));
    assert!(!passed, "{said}");
    assert!(said.contains("a rule's test is contested"), "{said}");
    assert!(calls.contains("issue create"), "{calls}");
    assert!(calls.contains("Rules contested: PR #7"), "{calls}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_contest_md_taken_from_the_base_is_warned_and_opens_nothing() {
    if !node_present() {
        return;
    }
    // milestone/28 carried R10's CONTEST.md; PR #531 took it with the base.
    let root = repo("inherited", true, &|root| {
        write(root, "src/lib.rs", "// the product, changed\n");
    });
    let (passed, said, calls) = contest(&root, Some("main"));
    assert!(passed, "{said}");
    assert!(said.contains("::warning::"), "{said}");
    assert!(said.contains("inherited from main"), "{said}");
    assert!(calls.is_empty(), "no issue read nor opened: {calls}");
    // A change that edits the inherited file contests it as its own.
    let edited = repo("edited", true, &|root| {
        write(root, "CONTEST.md", "R10 cannot hold, and R11 neither\n");
    });
    let (passed, _, calls) = contest(&edited, Some("main"));
    assert!(!passed);
    assert!(calls.contains("issue create"), "{calls}");
    // With no base, as before: every CONTEST.md is the change's.
    let (passed, _, calls) = contest(&root, None);
    assert!(!passed);
    assert!(calls.contains("issue list"), "{calls}");
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&edited);
}
