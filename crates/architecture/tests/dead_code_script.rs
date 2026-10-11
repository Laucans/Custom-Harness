//! `scripts/dead-code.sh`, as `init-repo` installs it, run for real against a
//! throwaway repository: which `pub` items it reports, and which it lets be.
//!
//! Hermetic: nothing is compiled, the script reads sources. Needs `bash` and
//! `git` on the path — what running this suite already needs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::FILES;

/// A repository with the script installed, removed on drop.
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

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repository holding `files`, every one of them tracked.
fn repo(case: &str, files: &[(&str, &str)]) -> Repo {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("dead-code-{case}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).expect("mkdir");
    let script = FILES
        .iter()
        .find(|asset| asset.path == "scripts/dead-code.sh")
        .expect("init-repo installs the script");
    write(&root, "scripts/dead-code.sh", script.content);
    for (path, text) in files {
        write(&root, path, text);
    }
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    Repo { root }
}

/// The script's verdict: its exit code and what it printed on stdout.
fn run(repo: &Repo) -> (i32, String) {
    let out = Command::new("bash")
        .arg("scripts/dead-code.sh")
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    (
        out.status.code().expect("an exit code"),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

#[test]
fn a_pub_item_written_nowhere_else_fails_and_is_named() {
    let repo = repo(
        "unused",
        &[(
            "crates/a/src/lib.rs",
            "pub fn used() {}\npub fn forgotten() {}\npub struct Orphan;\nfn caller() { used(); }\n",
        )],
    );
    let (code, out) = run(&repo);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("crates/a/src/lib.rs:2: pub forgotten"),
        "{out}"
    );
    assert!(out.contains("crates/a/src/lib.rs:3: pub Orphan"), "{out}");
    assert!(!out.contains("used"), "{out}");
}

#[test]
fn a_use_in_another_crate_or_in_a_test_counts() {
    let repo = repo(
        "used",
        &[
            (
                "crates/a/src/lib.rs",
                "pub fn called() {}\npub const TESTED: u8 = 1;\npub(crate) fn inner() {}\n",
            ),
            ("crates/b/src/lib.rs", "fn f() { a::called(); }\n"),
            (
                "crates/a/tests/it.rs",
                "#[test] fn t() { assert_eq!(a::TESTED, 1); }\n",
            ),
            ("crates/a/src/other.rs", "fn g() { super::inner(); }\n"),
        ],
    );
    let (code, out) = run(&repo);
    assert_eq!(code, 0, "{out}");
    assert!(out.is_empty(), "{out}");
}

#[test]
fn an_export_or_an_allowed_item_is_left_alone() {
    let repo = repo(
        "kept",
        &[(
            "crates/a/src/lib.rs",
            "/// The page calls it.\n#[wasm_bindgen(js_name = setView)]\npub fn set_view() {}\n\
             #[no_mangle]\npub extern \"C\" fn hook() {}\n\
             // A downstream repository calls it.\n#[allow(dead_code)]\n/// Documented.\npub fn api() {}\n\
             #[allow(dead_code)]\nfn private() {}\npub fn after_the_allowance() {}\n",
        )],
    );
    let (code, out) = run(&repo);
    assert_eq!(code, 1, "{out}");
    assert_eq!(
        out.lines().count(),
        1,
        "only the item after the allowance, which it does not cover: {out}"
    );
    assert!(out.contains("pub after_the_allowance"), "{out}");
}

#[test]
fn a_binary_s_items_are_the_compiler_s_to_judge() {
    let repo = repo(
        "binary",
        &[
            (
                "crates/app/src/main.rs",
                "pub fn only_here() {}\nfn main() {}\n",
            ),
            (
                "crates/app/src/bin/tool.rs",
                "pub fn also_only_here() {}\nfn main() {}\n",
            ),
        ],
    );
    let (code, out) = run(&repo);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_repository_without_rust_passes() {
    let repo = repo("empty", &[("README.md", "nothing yet\n")]);
    let (code, out) = run(&repo);
    assert_eq!(code, 0, "{out}");
}
