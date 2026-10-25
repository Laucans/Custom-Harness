//! `node scripts/rules-gate.mjs status`, as `init-repo` installs it, on a
//! throwaway repository: each rule's state from what its tests wrote, a
//! mis-mounted test said apart — after every other rule, with what to do,
//! its rule's red tests listed under it — and failing always, a rule marked
//! `conform` too, whatever its other tests said. Skipped, loudly, naming the
//! node that answered, without node 18; red instead when `CI` is set.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::content_of;
use serde_json::Value;

struct Repo(PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

const RULES: &str = r#"{"version": 1, "rules": [
  {"id": "R1", "title": "a", "family": "business", "criterion": {"given": "g", "when": "w", "then": "t"}},
  {"id": "R2", "title": "b", "family": "business", "criterion": {"given": "g", "when": "w", "then": "t"}},
  {"id": "R3", "title": "c", "family": "journey", "criterion": {"given": "g", "when": "w", "then": "t"}},
  {"id": "R4", "title": "d", "family": "business", "state": "conform", "criterion": {"given": "g", "when": "w", "then": "t"}},
  {"id": "R5", "title": "e", "family": "business", "criterion": {"given": "g", "when": "w", "then": "t"}}
]}"#;

/// What the tests wrote: R2 red by one test and mis-mounted by another, R3
/// mis-mounted, R4 (conform) mis-mounted, R5 pending.
const STATUS: &str = r#"{"rule": "R1", "family": "business", "test": "r1", "state": "green", "why": ""}
{"rule": "R2", "family": "business", "test": "r2a", "state": "red", "why": "still listed"}
{"rule": "R2", "family": "business", "test": "r2b", "state": "mis-mounted", "why": "campaign.listEntityVersions has no input \"id\"; its capability.json takes entity (crates/campaign/capabilities/list-entity-versions/capability.json)"}
{"rule": "R3", "family": "journey", "test": "R3 c", "state": "mis-mounted", "why": "opens screen:campagne, which its needs do not name: a journey lists every screen it walks through"}
{"rule": "R4", "family": "business", "test": "r4", "state": "mis-mounted", "why": "reaches capability:a.b, which its needs do not name: a test lists every unit it uses"}
{"rule": "R5", "family": "business", "test": "r5", "state": "pending", "why": "waiting for capability:a.c"}
"#;

/// Which node answered and from where: its version, its path, the PATH
/// searched — what a skip or a CI failure says.
fn which_node() -> String {
    let version = Command::new("node")
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map_or_else(
            || "none".to_string(),
            |out| String::from_utf8_lossy(&out.stdout).trim().to_string(),
        );
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path)
        .map(|dir| dir.join("node"))
        .find(|node| node.is_file())
        .map_or_else(
            || "not found".to_string(),
            |node| node.display().to_string(),
        );
    format!(
        "node {version} ({found}) on PATH={}",
        path.to_string_lossy()
    )
}

/// Whether a node runs the gate (18 or later: optional chaining and the
/// rest of what the gate is written in); with `CI` set, red, never skipped.
fn node() -> bool {
    let recent = Command::new("node")
        .args([
            "-e",
            "process.exit(Number(process.versions.node.split('.')[0]) >= 18 ? 0 : 1)",
        ])
        .output()
        .is_ok_and(|out| out.status.success());
    if recent {
        return true;
    }
    let which = which_node();
    assert!(
        std::env::var_os("CI").is_none(),
        "rules_gate_status: no node 18 or later, and CI is set: {which}"
    );
    eprintln!("rules_gate_status: no node 18 or later, skipped: {which}");
    false
}

fn repo(case: &str, status: &str) -> Repo {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rules-gate-status-{case}-{}-{nanos}",
        std::process::id()
    ));
    write(
        &root,
        "scripts/rules-gate.mjs",
        content_of("scripts/rules-gate.mjs").expect("installed"),
    );
    write(&root, "rules.json", RULES);
    write(&root, "target/rules-status.jsonl", status);
    Repo(root)
}

fn status(repo: &Repo, args: &[&str]) -> Output {
    Command::new("node")
        .arg("scripts/rules-gate.mjs")
        .arg("status")
        .args(args)
        .env_remove("RULES_STATUS")
        .current_dir(&repo.0)
        .output()
        .expect("node runs")
}

#[test]
fn a_mis_mounted_test_is_said_apart_with_its_rules_reds_and_always_fails() {
    if !node() {
        return;
    }
    let repo = repo("apart", STATUS);
    let out = status(&repo, &[]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{said}");
    let lines: Vec<&str> = said.lines().collect();
    let apart = lines
        .iter()
        .position(|l| l.starts_with("mis-mounted: "))
        .expect("the mis-mounted are said apart");
    assert_eq!(
        lines[apart],
        "mis-mounted: 3 rule(s) whose test is mounted wrong, not the product — the rules' task fixes the test before anything is contested; a development copies the line into CONTEST.md and changes nothing for it"
    );
    assert_eq!(
        &lines[..apart],
        &["R1 green", "R5 pending — waiting for capability:a.c", ""],
        "every other rule first, none of the mis-mounted"
    );
    assert_eq!(
        &lines[apart + 1..],
        &[
            "::error::R2 mis-mounted — campaign.listEntityVersions has no input \"id\"; its capability.json takes entity (crates/campaign/capabilities/list-entity-versions/capability.json)",
            "  R2 also red: r2a — still listed (the product's, fix it now)",
            "::error::R3 mis-mounted — opens screen:campagne, which its needs do not name: a journey lists every screen it walks through",
            "::error::R4 mis-mounted (to conform) — reaches capability:a.b, which its needs do not name: a test lists every unit it uses",
        ],
        "a mis-mount wins over its rule's red, its own test's why, the red listed under it; a conform rule fails too"
    );
    assert!(said.contains("still listed"), "{said}");
    let written: Value = serde_json::from_str(
        &fs::read_to_string(repo.0.join("rules-status.json")).expect("written"),
    )
    .expect("JSON");
    let states: Vec<&str> = written["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .filter_map(|r| r["state"].as_str())
        .collect();
    assert_eq!(
        states,
        [
            "green",
            "mis-mounted",
            "mis-mounted",
            "mis-mounted",
            "pending"
        ]
    );

    let strict = status(&repo, &["--strict"]);
    let said = String::from_utf8_lossy(&strict.stdout);
    assert_eq!(strict.status.code(), Some(1));
    assert!(
        said.contains("::error::R4 mis-mounted (to conform) — reaches capability:a.b"),
        "strict, where a milestone lands, the same: {said}"
    );
}

#[test]
fn a_conform_rule_alone_mis_mounted_fails_without_strict_and_none_says_nothing_apart() {
    if !node() {
        return;
    }
    let only_conform = repo(
        "conform",
        "{\"rule\": \"R4\", \"test\": \"r4\", \"state\": \"mis-mounted\", \"why\": \"x\"}\n",
    );
    let out = status(&only_conform, &[]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a mounting defect is never waived by conform: {said}"
    );
    assert!(
        said.contains("::error::R4 mis-mounted (to conform) — x"),
        "{said}"
    );

    let conform_red = repo(
        "conform-red",
        "{\"rule\": \"R4\", \"test\": \"r4\", \"state\": \"red\", \"why\": \"y\"}\n",
    );
    let out = status(&conform_red, &[]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a conform rule's red waits for --strict: {said}"
    );

    let clean = repo(
        "clean",
        "{\"rule\": \"R1\", \"test\": \"r1\", \"state\": \"green\", \"why\": \"\"}\n",
    );
    let out = status(&clean, &[]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{said}");
    assert!(!said.contains("mis-mounted"), "{said}");
}
