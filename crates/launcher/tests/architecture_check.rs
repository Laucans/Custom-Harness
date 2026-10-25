//! `harness architecture-check`, the closure check `scripts/gates-local.sh`
//! runs through `HARNESS_BIN`, against the real binary, git and disk: a
//! file changed outside the unit's closure fails naming it, a change inside
//! passes, and a violation already on the base is said as older, never
//! failed on.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn git(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const DECLARATION: &str =
    "unit: micro-ui\nsystem: credit\nproduces: RiskBadge\nneeds: credit.calculateRisk\n";

/// A product under the architecture: a Capability, a Micro-UI that needs
/// it, mocked; committed as the base. The session facts beside it.
fn product(case: &str) -> (Dir, PathBuf, String) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let dir = Dir(std::env::temp_dir().join(format!(
        "architecture-check-{case}-{}-{stamp}",
        std::process::id()
    )));
    let root = dir.0.join("work");
    fs::create_dir_all(&root).expect("root");
    git(&root, &["init", "-q", "-b", "main"]);
    let query = "# fields: a.b\nquery Q { a }";
    for (path, text) in [
        ("contracts/README.md", "contracts"),
        (
            "crates/credit/capabilities/calculate-risk/capability.json",
            r#"{"capability":"credit.calculateRisk","system":"credit","reads":["q.graphql"]}"#,
        ),
        (
            "crates/credit/capabilities/calculate-risk/Cargo.toml",
            "[package]\n",
        ),
        ("crates/credit/capabilities/calculate-risk/q.graphql", query),
        (
            "crates/dataguard/registry/Cargo.toml",
            "[package.metadata.cargo-machete]\nignored = [\"calculate-risk\"]\n\n[dependencies]\ncalculate-risk = { path = \"../../credit/capabilities/calculate-risk\" }\n",
        ),
        (
            "apps/credit/risk-badge/micro-ui.json",
            r#"{"microUi":"RiskBadge","system":"credit","needs":["credit.calculateRisk"]}"#,
        ),
        (
            "mock/pages.json",
            r#"{"version":1,"pages":[{"path":"pages/credit.html","title":"Credit","objective":"o","components":["RiskBadge"]}],"components":[{"name":"RiskBadge","system":"credit","description":"d","fragment":"components/RiskBadge.html","needs":["credit.calculateRisk"],"actions":[]}]}"#,
        ),
        (
            "mock/components/RiskBadge.html",
            r#"<div data-reads="credit.calculateRisk"></div>"#,
        ),
        (
            "mock/pages/credit.html",
            r#"<div data-component="RiskBadge"></div>"#,
        ),
    ] {
        write(&root, path, text);
    }
    write(
        &root,
        "queries/registry.json",
        &format!(
            r#"{{"queries":[{{"query":"Q","capability":"credit.calculateRisk","file":"crates/credit/capabilities/calculate-risk/q.graphql","sha256":"{}","fields":["a.b"]}}]}}"#,
            sha256(query)
        ),
    );
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    write(
        &dir.0,
        "facts.json",
        &serde_json::json!({
            "task": "34",
            "base": "origin/main",
            "branch": "feat/risk-badge",
            "architecture": DECLARATION,
            "skeleton": [],
        })
        .to_string(),
    );
    (dir, root, base)
}

/// The SHA-256 of `text`, as the registry writes it, through `shasum`.
fn sha256(text: &str) -> String {
    let mut child = Command::new("shasum")
        .args(["-a", "256"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("shasum runs");
    std::io::Write::write_all(child.stdin.as_mut().expect("stdin"), text.as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("shasum ends");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Commits what `edit` changed and runs the check from `base` to it.
fn check(dir: &Dir, root: &Path, base: &str, edit: &dyn Fn(&Path)) -> (i32, String) {
    edit(root);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "work", "--allow-empty"]);
    let facts = dir.0.join("facts.json");
    let out = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args([
            "architecture-check",
            "--facts",
            &facts.to_string_lossy(),
            "--tree",
            &root.to_string_lossy(),
            "--repo",
            &root.to_string_lossy(),
            "--from",
            base,
            "--to",
            "HEAD",
        ])
        .current_dir(root)
        .output()
        .expect("the harness runs");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn a_file_outside_the_closure_fails_naming_it() {
    let (dir, root, base) = product("outside");
    let (code, said) = check(&dir, &root, &base, &|root| {
        write(
            root,
            "crates/credit/capabilities/calculate-risk/src/lib.rs",
            "pub fn x() {}\n",
        );
    });
    assert_eq!(code, 1, "{said}");
    assert!(
        said.contains("crates/credit/capabilities/calculate-risk/src/lib.rs: changed by this round, outside the closure of the micro-ui"),
        "{said}"
    );
}

#[test]
fn a_change_inside_the_closure_holds() {
    let (dir, root, base) = product("inside");
    let (code, said) = check(&dir, &root, &base, &|root| {
        write(root, "apps/credit/risk-badge/index.ts", "export {};\n");
    });
    assert_eq!(code, 0, "{said}");
    assert!(said.starts_with("closure: ok"), "{said}");
}

#[test]
fn a_violation_already_on_the_base_is_said_and_not_failed_on() {
    let (dir, root, _) = product("before");
    write(
        &root,
        "apps/ledger/balance/micro-ui.json",
        r#"{"microUi":"Balance","system":"ledger","needs":["ledger.readBalance"]}"#,
    );
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "a neighbour's violation"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let (code, said) = check(&dir, &root, &base, &|root| {
        write(root, "apps/credit/risk-badge/index.ts", "export {};\n");
    });
    assert_eq!(code, 0, "{said}");
    assert!(said.starts_with("closure: ok"), "{said}");
    assert!(said.contains("(before this round, not yours)"), "{said}");
    assert!(said.contains("ledger.readBalance"), "{said}");
}

#[test]
fn without_a_task_it_cannot_judge_and_says_so() {
    let (_dir, root, base) = product("no-task");
    let out = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args([
            "architecture-check",
            "--tree",
            &root.to_string_lossy(),
            "--repo",
            &root.to_string_lossy(),
            "--from",
            &base,
        ])
        .current_dir(&root)
        .output()
        .expect("the harness runs");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).contains("closure: cannot judge"));
}
