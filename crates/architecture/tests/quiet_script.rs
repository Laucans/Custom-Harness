//! `scripts/quiet.sh` against real output: a pass is one line, a failure
//! keeps every line of its errors — the panic, the `left`/`right` of an
//! assertion, the list of failures, a clippy error whole — and loses only
//! the lines that say something progressed or passed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const SCRIPT: &str = include_str!("../assets/quiet.sh");

struct Dir {
    root: PathBuf,
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn dir(tag: &str) -> Dir {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("harness-quiet-{tag}-{nanos}"));
    fs::create_dir_all(&root).expect("mkdir");
    fs::write(root.join("quiet.sh"), SCRIPT).expect("script");
    Dir { root }
}

/// Runs the script on a command that prints `text` and exits with `code`.
fn quiet(root: &Path, text: &str, code: i32) -> Output {
    fs::write(root.join("output.txt"), text).expect("output");
    Command::new("bash")
        .arg(root.join("quiet.sh"))
        .args([
            "bash",
            "-c",
            &format!("cat '{}'; exit {code}", root.join("output.txt").display()),
        ])
        .env("TMPDIR", root)
        .output()
        .expect("bash runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The log path the last line names.
fn log_of(text: &str) -> PathBuf {
    let last = text.lines().last().expect("a last line");
    PathBuf::from(last.split("full log: ").nth(1).expect("a log path"))
}

const FAILED_TESTS: &str = "   Compiling ledger v0.1.0 (/repo/crates/ledger)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 2.31s
     Running unittests src/lib.rs (target/debug/deps/billing-1a2b)

running 2 tests
test invoices::tests::lists ... ok
test invoices::tests::totals ... ignored

test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src/lib.rs (target/debug/deps/ledger-3c4d)

running 3 tests
test lines::tests::adds ... ok
test lines::tests::sums ... FAILED
test lines::tests::rounds ... ok

failures:

---- lines::tests::sums stdout ----

thread 'lines::tests::sums' panicked at src/lines.rs:42:9:
assertion `left == right` failed
  left: 3
 right: 4
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    lines::tests::sums

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

error: test failed, to rerun pass `-p ledger --lib`
";

#[test]
fn a_pass_is_one_line_and_leaves_no_log() {
    let d = dir("pass");
    let output = quiet(&d.root, FAILED_TESTS, 0);
    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.starts_with("ok · bash -c "), "{text}");
    assert!(text.trim_end().ends_with('s'), "{text}");
    let logs = fs::read_dir(&d.root)
        .expect("dir")
        .filter(|e| {
            e.as_ref().is_ok_and(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with("quiet.") && name != "quiet.sh"
            })
        })
        .count();
    assert_eq!(logs, 0, "a pass keeps no log");
}

#[test]
fn a_failed_test_keeps_every_line_of_its_failure_and_drops_what_passed() {
    let d = dir("tests");
    let output = quiet(&d.root, FAILED_TESTS, 101);
    assert_eq!(output.status.code(), Some(101), "the command's exit code");
    let text = stdout(&output);
    // Every line of the failure, as it came.
    for line in [
        "     Running unittests src/lib.rs (target/debug/deps/ledger-3c4d)",
        "running 3 tests",
        "test lines::tests::sums ... FAILED",
        "---- lines::tests::sums stdout ----",
        "thread 'lines::tests::sums' panicked at src/lines.rs:42:9:",
        "assertion `left == right` failed",
        "  left: 3",
        " right: 4",
        "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
        "    lines::tests::sums",
        "test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s",
        "error: test failed, to rerun pass `-p ledger --lib`",
    ] {
        assert!(text.lines().any(|l| l == line), "lost {line:?} in:\n{text}");
    }
    // The failure's lines stay in their order.
    let at = |needle: &str| text.find(needle).expect(needle);
    assert!(at("panicked at") < at("  left: 3") && at("  left: 3") < at(" right: 4"));
    // What only says it progressed or passed is gone.
    for gone in [
        "Compiling",
        "Finished",
        "billing-1a2b",
        "invoices::tests::lists",
        "invoices::tests::totals",
        "test result: ok.",
        "lines::tests::adds ... ok",
        "lines::tests::rounds ... ok",
    ] {
        assert!(!text.contains(gone), "kept {gone:?} in:\n{text}");
    }
    let last = text.lines().last().expect("a last line");
    assert!(last.starts_with("failed (exit 101) · bash -c "), "{last}");
    // The full log is kept, word for word.
    let log = log_of(&text);
    assert_eq!(fs::read_to_string(log).expect("the log"), FAILED_TESTS);
}

#[test]
fn a_clippy_error_is_printed_whole() {
    let d = dir("clippy");
    let error = "    Checking ledger v0.1.0 (/repo/crates/ledger)
error: this function has too many lines (166/100)
   --> crates/ledger/src/lines.rs:824:5
    |
824 | /     async fn drawn(
825 | |         &self,
    | |_____^
    |
    = help: for further information visit https://rust-lang.github.io/rust-clippy/index.html#too_many_lines
    = note: `-D clippy::too-many-lines` implied by `-D warnings`

error: could not compile `ledger` (lib) due to 1 previous error
";
    let output = quiet(&d.root, error, 101);
    assert_eq!(output.status.code(), Some(101));
    let text = stdout(&output);
    let printed: Vec<&str> = text.lines().collect();
    let expected: Vec<&str> = error.lines().skip(1).collect();
    assert_eq!(
        printed[..printed.len() - 1],
        expected[..],
        "every line but `Checking`, in order"
    );
}

#[test]
fn a_failure_the_filter_would_empty_is_printed_raw() {
    let d = dir("raw");
    let text = "   Compiling ledger v0.1.0\n     Running unittests src/lib.rs (x)\n\nrunning 1 test\ntest a ... ok\n\ntest result: ok. 1 passed\n";
    let output = quiet(&d.root, text, 1);
    assert_eq!(output.status.code(), Some(1));
    let printed = stdout(&output);
    assert!(printed.starts_with(text), "the whole output:\n{printed}");
    assert!(printed.contains("failed (exit 1)"));
}

#[test]
fn without_a_command_it_says_how_to_call_it() {
    let d = dir("usage");
    let output = Command::new("bash")
        .arg(d.root.join("quiet.sh"))
        .output()
        .expect("bash runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}
