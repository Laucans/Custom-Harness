//! `scripts/contracts-gate.mjs untouched` against node and git, on a
//! repository of its own: a milestone changing its own contract passes, one
//! changing another milestone's fails naming both, an id the graph does not
//! know is the change's own, and without a graph nothing is judged.
//! Skipped, loudly, when `node` is not on the path.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const GATE: &str = include_str!("../assets/contracts-gate.mjs");

const GRAPH: &str = r#"{"version":1,"milestones":[],"ids":{
  "session.listScenes":{"kind":"capability","producer":20,"state":"mocked","version":1},
  "session.addScene":{"kind":"data-capability","producer":20,"state":"planned","version":1}
}}"#;

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

/// A repository with the graph above on `main`, the gate under `scripts/`,
/// and a branch `change` where `edit` was made and committed.
fn repo(case: &str, with_graph: bool, edit: &dyn Fn(&Path)) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "contracts-gate-{case}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("root");
    git(&root, &["init", "-q", "-b", "main"]);
    write(&root, "scripts/contracts-gate.mjs", GATE);
    if with_graph {
        write(&root, "plan/graph.json", GRAPH);
    }
    write(
        &root,
        "crates/session/capabilities/list-scenes/capability.json",
        r#"{"capability":"session.listScenes","system":"session","output":{"scenes":"[]"}}"#,
    );
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "change"]);
    edit(&root);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "change", "--allow-empty"]);
    root
}

fn gate(root: &Path, extra: &[&str], base_ref: Option<&str>) -> (bool, String) {
    let mut command = Command::new("node");
    command
        .arg("scripts/contracts-gate.mjs")
        .arg("untouched")
        .arg("main")
        .args(extra)
        .current_dir(root)
        .env_remove("GITHUB_BASE_REF");
    if let Some(base) = base_ref {
        command.env("GITHUB_BASE_REF", base);
    }
    let out = command.output().expect("node runs");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// The gate as the CI runs it on a pull request: `GITHUB_BASE_REF` and
/// `GITHUB_HEAD_REF` set as GitHub sets them.
fn gate_on_pr(root: &Path, base_ref: &str, head_ref: &str) -> (bool, String) {
    let out = Command::new("node")
        .arg("scripts/contracts-gate.mjs")
        .arg("untouched")
        .arg("main")
        .current_dir(root)
        .env("GITHUB_BASE_REF", base_ref)
        .env("GITHUB_HEAD_REF", head_ref)
        .output()
        .expect("node runs");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn node_present() -> bool {
    let present = Command::new("node").arg("--version").output().is_ok();
    if !present {
        eprintln!("contracts_gate: node is not on the path — skipped");
    }
    present
}

fn change_list_scenes(root: &Path) {
    write(
        root,
        "crates/session/capabilities/list-scenes/capability.json",
        r#"{"capability":"session.listScenes","system":"session","output":{"scenes":"[]","count":"number"}}"#,
    );
}

#[test]
fn a_milestone_changes_its_own_contract_and_no_other_s() {
    if !node_present() {
        return;
    }
    let root = repo("own", true, &change_list_scenes);
    let (ok, said) = gate(&root, &[], Some("milestone/20-sessions"));
    assert!(ok, "{said}");
    assert!(said.contains("1 manifest(s) changed"), "{said}");
    let (ok, said) = gate(&root, &[], Some("milestone/19-media"));
    assert!(!ok, "{said}");
    assert!(
        said.contains("session.listScenes belongs to milestone #20, not #19"),
        "{said}"
    );
    assert!(
        said.contains("AGENT_LOOP_STOP: contract session.listScenes"),
        "{said}"
    );
    let (ok, said) = gate(&root, &["--milestone", "20"], None);
    assert!(ok, "--milestone names it: {said}");
    let (ok, said) = gate(&root, &[], None);
    assert!(
        !ok,
        "no milestone known: another's contract is refused: {said}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_id_the_graph_does_not_know_is_the_change_s_own_and_no_graph_judges_nothing() {
    if !node_present() {
        return;
    }
    let root = repo("new", true, &|root| {
        write(
            root,
            "crates/session/capabilities/get-session/capability.json",
            r#"{"capability":"session.getSession","system":"session"}"#,
        );
    });
    let (ok, said) = gate(&root, &[], Some("milestone/19-media"));
    assert!(ok, "{said}");
    let _ = fs::remove_dir_all(&root);

    let root = repo("nograph", false, &change_list_scenes);
    let (ok, said) = gate(&root, &[], Some("milestone/19-media"));
    assert!(ok, "{said}");
    assert!(said.contains("no plan/graph.json"), "{said}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_deleted_manifest_and_a_wrong_usage_are_handled() {
    if !node_present() {
        return;
    }
    let root = repo("deleted", true, &|root| {
        fs::remove_file(root.join("crates/session/capabilities/list-scenes/capability.json"))
            .expect("removed");
    });
    let (ok, said) = gate(&root, &[], Some("milestone/19-media"));
    assert!(ok, "a file that is gone has no id to judge: {said}");
    let out = Command::new("node")
        .arg("scripts/contracts-gate.mjs")
        .current_dir(&root)
        .output()
        .expect("node runs");
    assert_eq!(out.status.code(), Some(2));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_milestone_s_own_pull_request_is_named_by_its_head_branch() {
    // ForgeDesMondes' PR #268, milestone/18 into main_agent: the base names
    // no milestone, the head does — every contract of the milestone is its
    // own. A task's PR keeps naming it by the base, which wins over the head.
    if !node_present() {
        return;
    }
    let root = repo("head", true, &change_list_scenes);
    let (ok, said) = gate_on_pr(&root, "main_agent", "milestone/20-sessions");
    assert!(ok, "the head names the milestone: {said}");
    let (ok, said) = gate_on_pr(&root, "main_agent", "milestone/19-media");
    assert!(!ok, "{said}");
    assert!(
        said.contains("session.listScenes belongs to milestone #20, not #19"),
        "{said}"
    );
    let (ok, said) = gate_on_pr(&root, "milestone/19-media", "milestone/20-sessions");
    assert!(!ok, "the base wins over the head: {said}");
    assert!(said.contains("not #19"), "{said}");
    let (ok, said) = gate_on_pr(&root, "main_agent", "feat/list-scenes");
    assert!(
        !ok,
        "neither names a milestone: another's contract is refused: {said}"
    );
    assert!(!said.contains(", not #"), "{said}");
    let _ = fs::remove_dir_all(&root);
}
