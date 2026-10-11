//! `scripts/mutants-rotation.sh` and `scripts/test-debt.sh`, as `init-repo`
//! installs them, run for real: which files a turn judges, and what a missed
//! mutant becomes.
//!
//! Hermetic: the candidates come from a file (`MUTANTS_ROTATION_LIST`), and
//! `cargo` and `gh` are fakes put first on the path — scripts that record
//! their arguments and answer what the test tells them to. Needs `bash`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::FILES;

/// A folder with the two scripts and a `bin/` of fakes, removed on drop.
struct Dir {
    root: PathBuf,
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn installed(path: &str) -> &'static str {
    FILES
        .iter()
        .find(|asset| asset.path == path)
        .map(|asset| asset.content)
        .expect("init-repo installs the script")
}

/// A fake that appends its arguments to `log` and exits with `$FAKE_CODE`.
const FAKE_CARGO: &str = r#"#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_LOG"
exit "${FAKE_CODE:-0}"
"#;

/// A fake `gh`: records every call; `issue list` answers the number of the
/// open issue whose title, in `$FAKE_OPEN` (`number<TAB>title` lines), is the
/// one quoted in `--search`.
const FAKE_GH: &str = r#"#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_LOG"
if [ "$1 $2" = "issue list" ]; then
  while [ $# -gt 0 ]; do
    if [ "$1" = "--search" ]; then title=${2#\"}; title=${title%%\"*}; fi
    shift
  done
  [ -f "${FAKE_OPEN:-}" ] && awk -F'\t' -v t="$title" '$2 == t { print $1 }' "$FAKE_OPEN"
fi
exit 0
"#;

fn dir(case: &str) -> Dir {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rotation-{case}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).expect("mkdir");
    for script in ["scripts/mutants-rotation.sh", "scripts/test-debt.sh"] {
        write(&root, script, installed(script));
    }
    write(&root, "bin/cargo", FAKE_CARGO);
    write(&root, "bin/gh", FAKE_GH);
    for fake in ["bin/cargo", "bin/gh"] {
        let status = Command::new("chmod")
            .args(["+x", fake])
            .current_dir(&root)
            .status()
            .expect("chmod runs");
        assert!(status.success());
    }
    Dir { root }
}

/// Runs `script` with the fakes first on the path and `env` set.
fn run(dir: &Dir, script: &str, args: &[&str], env: &[(&str, &str)]) -> Output {
    let path = format!(
        "{}:{}",
        dir.root.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::new("bash");
    command
        .arg(script)
        .args(args)
        .current_dir(&dir.root)
        .env("PATH", path)
        .env("FAKE_LOG", dir.root.join("calls.log"));
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("bash runs")
}

fn calls(dir: &Dir) -> String {
    fs::read_to_string(dir.root.join("calls.log")).unwrap_or_default()
}

/// The files turn `turn` picks out of `files`, dealt into `buckets`.
fn picked(dir: &Dir, files: &[String], turn: u32, buckets: u32) -> BTreeSet<String> {
    write(&dir.root, "files.txt", &files.join("\n"));
    let out = run(
        dir,
        "scripts/mutants-rotation.sh",
        &[&turn.to_string(), &buckets.to_string()],
        &[
            ("MUTANTS_ROTATION_LIST", "files.txt"),
            ("MUTANTS_ROTATION_DRY_RUN", "1"),
        ],
    );
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

fn sources(range: std::ops::Range<u32>) -> Vec<String> {
    range.map(|n| format!("crates/c{n}/src/lib.rs")).collect()
}

#[test]
fn every_file_is_judged_once_in_a_full_round_of_turns() {
    let dir = dir("round");
    let files = sources(0..40);
    let mut seen = Vec::new();
    for turn in 100..108 {
        seen.extend(picked(&dir, &files, turn, 8));
    }
    let distinct: BTreeSet<_> = seen.iter().cloned().collect();
    assert_eq!(seen.len(), files.len(), "no file twice: {seen:?}");
    assert_eq!(distinct, files.iter().cloned().collect());
}

#[test]
fn diverged_branches_still_deal_their_shared_files_alike() {
    // One branch removed c0..c5 and added c40..c50: what both still hold
    // lands in the same bucket, so two runs with different turns never
    // judge the same file.
    let dir = dir("diverged");
    let ours = sources(0..40);
    let theirs = sources(5..50);
    for turn in 0..8 {
        let a = picked(&dir, &ours, turn, 8);
        let b = picked(&dir, &theirs, turn, 8);
        let shared = |set: &BTreeSet<String>| -> BTreeSet<String> {
            set.iter()
                .filter(|f| ours.contains(f) && theirs.contains(f))
                .cloned()
                .collect()
        };
        assert_eq!(shared(&a), shared(&b), "turn {turn}");
        let next = picked(&dir, &theirs, turn + 1, 8);
        assert!(a.is_disjoint(&next), "turns {turn} and {}", turn + 1);
    }
}

#[test]
fn a_rerun_judges_the_same_files_and_bad_arguments_are_refused() {
    let dir = dir("rerun");
    let files = sources(0..40);
    assert_eq!(picked(&dir, &files, 3, 8), picked(&dir, &files, 11, 8));
    for args in [&[][..], &["x"], &["3", "0"]] {
        let out = run(&dir, "scripts/mutants-rotation.sh", args, &[]);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
    }
}

#[test]
fn what_cargo_mutants_finds_never_fails_the_run_but_its_own_error_does() {
    let dir = dir("codes");
    write(&dir.root, "files.txt", &sources(0..4).join("\n"));
    let list = [("MUTANTS_ROTATION_LIST", "files.txt")];
    for (code, expected) in [("0", 0), ("2", 0), ("3", 0), ("4", 0), ("70", 70)] {
        let env = [list[0], ("FAKE_CODE", code)];
        let out = run(
            &dir,
            "scripts/mutants-rotation.sh",
            &["0", "1", "--", "--in-place"],
            &env,
        );
        assert_eq!(
            out.status.code(),
            Some(expected),
            "cargo-mutants exited {code}"
        );
    }
    let last = calls(&dir).lines().last().unwrap_or_default().to_string();
    assert!(
        last.starts_with("mutants --workspace --no-shuffle --file crates/"),
        "{last}"
    );
    assert_eq!(
        last.matches("--file").count(),
        4,
        "one bucket holds them all"
    );
    assert!(
        last.ends_with("--in-place"),
        "the arguments after -- reach cargo: {last}"
    );
}

#[test]
fn a_missed_mutant_opens_one_issue_per_file_or_comments_on_the_open_one() {
    let dir = dir("debt");
    write(
        &dir.root,
        "out/missed.txt",
        "crates/a/src/lib.rs:3:5: replace f -> bool with true\n\
         crates/a/src/lib.rs:9:1: delete match arm 'x' in g\n\
         crates/b/src/lib.rs:2:2: replace == with != in h\n",
    );
    write(
        &dir.root,
        "open.tsv",
        "17\tTest debt: crates/b/src/lib.rs\n",
    );
    let out = run(
        &dir,
        "scripts/test-debt.sh",
        &["out", "run 42"],
        &[("FAKE_OPEN", "open.tsv")],
    );
    assert!(out.status.success(), "{out:?}");
    let log = calls(&dir);
    assert_eq!(
        log.matches("label create harness:test-debt").count(),
        1,
        "{log}"
    );
    assert_eq!(log.matches("issue create").count(), 1, "{log}");
    assert!(
        log.contains("--title Test debt: crates/a/src/lib.rs"),
        "{log}"
    );
    assert!(log.contains("2 mutant(s), found in run 42"), "{log}");
    assert!(log.contains("- line 9: delete match arm 'x' in g"), "{log}");
    assert!(log.contains("issue comment 17"), "{log}");
    assert!(log.contains("- line 2: replace == with != in h"), "{log}");
    // The open issue is found without its label — the workflow took it off
    // as it started — and asked again.
    assert!(
        log.contains("issue edit 17 --add-label harness:test-debt"),
        "{log}"
    );
    let listing = log
        .lines()
        .find(|line| line.starts_with("issue list"))
        .unwrap_or_default();
    assert!(!listing.contains("--label"), "{listing}");
}

#[test]
fn nothing_missed_files_nothing() {
    let dir = dir("clean");
    write(&dir.root, "out/caught.txt", "crates/a/src/lib.rs:3:5: x\n");
    let out = run(&dir, "scripts/test-debt.sh", &["out"], &[]);
    assert!(out.status.success(), "{out:?}");
    assert!(calls(&dir).is_empty());
}
