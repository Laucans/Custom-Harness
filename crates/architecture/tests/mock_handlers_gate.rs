//! The `mock-handlers` gate of `gates.yml`, as `init-repo` installs it, run
//! for real against a throwaway repository: no handler id is defined twice
//! — in one literal, or in its file and a legacy file — a handler file
//! defines its own id only, and once `mock/handlers/` holds a handler file
//! (its README and `_lib/` do not count) a pull request adds no handler to
//! the legacy files, whose handlers stay editable.
//!
//! Needs `bash`, `git` and `node`; skipped, loudly, without node.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::content_of;

struct Repo {
    root: PathBuf,
    base: String,
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const LEGACY: &str = "export const COMMANDS = {\n  // live.move: the row moves.\n  'live.move': async () => undefined,\n};\n";

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

/// A repository whose base commit holds the gate's scripts and `files`.
fn repo(case: &str, files: &[(&str, &str)]) -> Option<Repo> {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("mock_handlers_gate: node is not on the path, skipped");
        return None;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "mock-handlers-gate-{case}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("mkdir");
    git(&root, &["init", "-q", "-b", "main_agent"]);
    for path in [
        "scripts/mock-handlers-gate.mjs",
        "scripts/split-handlers.mjs",
    ] {
        write(&root, path, content_of(path).expect("installed"));
    }
    for (path, text) in files {
        write(&root, path, text);
    }
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    Some(Repo { root, base })
}

/// The gate's `run: |` block, as the step runs it.
fn script() -> String {
    let gates = content_of(".github/workflows/gates.yml").expect("installed");
    let step = gates
        .split("- id: mock-handlers\n")
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

/// Commits `files` on top of the base (an empty text deletes), then runs
/// the gate: on a pull request against the base when `pull_request`, on a
/// push otherwise.
fn change(repo: &Repo, files: &[(&str, &str)], pull_request: bool) -> (bool, String) {
    for (path, text) in files {
        if text.is_empty() {
            let _ = fs::remove_file(repo.root.join(path));
        } else {
            write(&repo.root, path, text);
        }
    }
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-qm", "change", "--allow-empty"]);
    let out = Command::new("bash")
        .arg("-c")
        .arg(script())
        .env("BASE", if pull_request { repo.base.as_str() } else { "" })
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
fn a_product_without_handlers_passes_and_a_key_twice_in_a_literal_fails() {
    let Some(repo) = repo("twice-in-literal", &[]) else {
        return;
    };
    let (ok, said) = change(&repo, &[], true);
    assert!(ok, "{said}");
    assert!(said.contains("0 handler file(s), 0 id(s)"), "{said}");
    let twice = LEGACY.replace(
        "};\n",
        "  'live.move': async () => ['the later one silently wins'],\n};\n",
    );
    let (ok, said) = change(&repo, &[("mock/src/commands.js", &twice)], false);
    assert!(!ok, "{said}");
    assert!(
        said.contains("::error file=mock/src/commands.js::live.move is defined twice in COMMANDS (lines 3 and 4)"),
        "{said}"
    );
}

#[test]
fn an_id_in_its_file_and_a_legacy_file_or_in_another_id_s_file_fails() {
    let Some(repo) = repo("twice-across", &[("mock/src/commands.js", LEGACY)]) else {
        return;
    };
    let (ok, said) = change(
        &repo,
        &[
            (
                "mock/handlers/live.move.js",
                "export const commands = {\n  'live.move': async () => undefined,\n};\n",
            ),
            (
                "mock/handlers/live.close.js",
                "export const commands = { 'live.close': async () => undefined, 'live.open': async () => undefined };\n",
            ),
            ("mock/handlers/live.empty.js", "export const nothing = 1;\n"),
            (
                "mock/handlers/live.twice.js",
                "export const reads = { 'live.twice': 1, 'live.twice': 2 };\n",
            ),
            ("mock/handlers/live.broken.js", "export const reads = {;\n"),
            (
                "mock/handlers/_lib/live.js",
                "export const commands = { 'not.a.handler': 1 };\n",
            ),
            ("mock/handlers/README.md", "the convention"),
        ],
        false,
    );
    assert!(!ok, "{said}");
    for finding in [
        "::error file=mock/handlers/live.move.js::live.move is defined twice: mock/handlers/live.move.js and mock/src/commands.js — keep one",
        "::error file=mock/handlers/live.close.js::defines live.open: a handler file defines its own id only — live.open belongs in mock/handlers/live.open.js",
        "::error file=mock/handlers/live.empty.js::exports neither",
        "::error file=mock/handlers/live.twice.js::live.twice is defined twice in reads (lines 1 and 1)",
        "::error file=mock/handlers/live.broken.js::cannot be read",
        "mock-handlers: 5 finding(s)",
    ] {
        assert!(said.contains(finding), "{finding}: {said}");
    }
    assert!(
        !said.contains("_lib"),
        "the helpers are not handlers: {said}"
    );
    assert!(!said.contains("live.close is defined"), "{said}");
}

#[test]
fn once_handlers_exist_a_pull_request_adds_no_legacy_handler_but_edits_one() {
    let with_handlers = [
        ("mock/src/commands.js", LEGACY),
        (
            "mock/handlers/live.close.js",
            "export const commands = {\n  'live.close': async () => undefined,\n};\n",
        ),
    ];
    let Some(repo) = repo("new-legacy-key", &with_handlers) else {
        return;
    };
    let edited = LEGACY.replace("async () => undefined", "async () => ['edited']");
    let (ok, said) = change(&repo, &[("mock/src/commands.js", &edited)], true);
    assert!(ok, "an existing legacy handler stays editable: {said}");
    let (ok, said) = change(
        &repo,
        &[(
            "mock/handlers/live.resize.js",
            "export const commands = { 'live.resize': async () => undefined };\n",
        )],
        true,
    );
    assert!(ok, "a new handler in its own file: {said}");
    assert!(said.contains("2 handler file(s), 3 id(s)"), "{said}");
    let added = edited.replace("};\n", "  \"live.hide\": async () => undefined,\n};\n");
    let (ok, said) = change(&repo, &[("mock/src/commands.js", &added)], true);
    assert!(!ok, "{said}");
    assert!(
        said.contains("::error file=mock/src/commands.js::line 4 adds live.hide to mock/src/commands.js: a new handler goes in its own file, mock/handlers/live.hide.js"),
        "{said}"
    );
    assert!(!said.contains("adds live.move"), "{said}");
    let (ok, said) = change(&repo, &[], false);
    assert!(ok, "a push is not a pull request: {said}");
}

#[test]
fn before_any_handler_file_the_legacy_files_still_grow() {
    // The upgrade seeds the folder's README before any split: no handler yet.
    let Some(repo) = repo(
        "before-split",
        &[
            ("mock/src/commands.js", LEGACY),
            ("mock/handlers/README.md", "the convention"),
            (
                "mock/handlers/_lib/live.js",
                "export const helper = () => 1;\n",
            ),
        ],
    ) else {
        return;
    };
    let added = LEGACY.replace("};\n", "  'live.hide': async () => undefined,\n};\n");
    let (ok, said) = change(&repo, &[("mock/src/commands.js", &added)], true);
    assert!(ok, "{said}");
    let (ok, said) = change(
        &repo,
        &[(
            "mock/src/reads.js",
            "export const READS = { 'live.row': async () => 1 };\n",
        )],
        true,
    );
    assert!(ok, "{said}");
}

#[test]
fn a_legacy_file_recreated_after_the_split_adds_every_key() {
    let Some(repo) = repo(
        "recreated",
        &[(
            "mock/handlers/live.close.js",
            "export const commands = { 'live.close': async () => undefined };\n",
        )],
    ) else {
        return;
    };
    let (ok, said) = change(&repo, &[("mock/src/commands.js", LEGACY)], true);
    assert!(!ok, "{said}");
    assert!(said.contains("line 3 adds live.move"), "{said}");
    let unknown = Command::new("bash")
        .arg("-c")
        .arg(script())
        .env("BASE", "0123456789abcdef0123456789abcdef01234567")
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    let said = String::from_utf8_lossy(&unknown.stdout);
    assert!(unknown.status.success(), "{said}");
    assert!(
        said.contains("no base 0123456789abcdef0123456789abcdef01234567 to compare with"),
        "{said}"
    );
}
