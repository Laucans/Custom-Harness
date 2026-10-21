//! The rules exist twice, knowingly: `tree::violations` is the loop's own
//! reading, `assets/gates.yml` runs the same rules in the target's CI. This
//! runs both on the same fixture repositories and requires the same
//! verdict — healthy passes both, each break fails both — so a change to
//! one cannot drift from the other unnoticed.
//!
//! The jobs run here are shell over the files (no `npx`): members by glob,
//! the system frontier of a Micro-UI's `needs`, its `actions`, the
//! ownership of a mutating DataCapability, its folder, an aggregate's
//! folder, the Concepts a Capability implements — and `units-registered`,
//! which runs `scripts/register.sh --check` and `cargo metadata --no-deps`
//! (no network) on a fixture that is a real workspace.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::tree::{Tree, undocumented_concepts, violations};
use harness_core::domain::Outcome;
use harness_core::ports::shell::disk::Disk;

const GATES: &str = include_str!("../assets/gates.yml");
const REGISTER: &str = include_str!("../assets/register.sh");

/// The real disk, read-only, for the loop's reading of the fixture.
struct RealDisk;

impl Disk for RealDisk {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
        unreachable!("read-only")
    }
    fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
        unreachable!("read-only")
    }
    fn dir_names(&self, path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
    fn file_names(&self, path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|e| e.path().is_file())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
    fn read_to_string(&self, path: &Path) -> Option<String> {
        fs::read_to_string(path).ok()
    }
    fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
        unreachable!("read-only")
    }
}

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

/// A unit crate: its `Cargo.toml` and an empty library.
fn krate(root: &Path, dir: &str, package: &str) {
    write(
        root,
        &format!("{dir}/Cargo.toml"),
        &format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    );
    write(root, &format!("{dir}/src/lib.rs"), "");
}

/// Runs the installed `scripts/register.sh` in the fixture: the registry
/// as the unit folders say.
fn register(root: &Path) {
    let out = Command::new("bash")
        .arg("scripts/register.sh")
        .current_dir(root)
        .output()
        .expect("bash runs");
    assert!(
        out.status.success(),
        "register.sh: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The healthy shape: one system, a Capability with its Concept, a
/// Micro-UI that needs it and triggers one DataCapability of an aggregate
/// that declares what it touches — the three units crates, named in the
/// registry the script writes, beside the server whose engine line it
/// copies.
fn repo(case: &str) -> Repo {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "harness-parity-{case}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("root");
    write(
        &root,
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/*/capabilities/*\", \"crates/*/data-capabilities/*\", \"crates/*/aggregates/*\", \"crates/dataguard/server\", \"crates/dataguard/registry\"]\n",
    );
    write(&root, "scripts/register.sh", REGISTER);
    write(
        &root,
        "crates/dataguard/server/Cargo.toml",
        "[package]\nname = \"dataguard-server\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nharness-dataguard = { git = \"https://example.invalid/harness.git\", tag = \"dataguard-v2\" }\ndataguard-registry = { path = \"../registry\" }\n",
    );
    write(&root, "crates/dataguard/server/src/lib.rs", "");
    write(&root, "crates/dataguard/registry/src/lib.rs", "");
    write(
        &root,
        "crates/dataguard/registry/build.rs",
        "fn main() {}\n",
    );
    write(&root, "contracts/README.md", "contracts");
    write(
        &root,
        "crates/credit/capabilities/risk/capability.json",
        r#"{"capability":"credit.calculateRisk","system":"credit","implements":"Risk@3","reads":["q.graphql"]}"#,
    );
    krate(&root, "crates/credit/capabilities/risk", "risk");
    write(
        &root,
        "crates/credit/capabilities/risk/q.graphql",
        "# fields: a.b\nquery Q { a }",
    );
    let sha = harness_architecture::tree::sha256("# fields: a.b\nquery Q { a }");
    write(
        &root,
        "queries/registry.json",
        &format!(
            r#"{{"queries":[{{"query":"Q","capability":"credit.calculateRisk","file":"crates/credit/capabilities/risk/q.graphql","sha256":"{sha}","fields":["a.b"]}}]}}"#
        ),
    );
    write(
        &root,
        "concepts/Risk/v3/concept.json",
        r#"{"concept":"Risk","version":3}"#,
    );
    write(
        &root,
        "apps/credit/risk-badge/micro-ui.json",
        r#"{"microUi":"RiskBadge","system":"credit","needs":["credit.calculateRisk"],"actions":["credit.requestLimitChange"]}"#,
    );
    write(
        &root,
        "crates/credit/data-capabilities/request-limit/data-capability.json",
        r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard","effect":"update","target":{"aggregate":"Account"},"touches":["Account.creditLimit"],"callableBy":["credit.*","RiskBadge"],"invariants":[]}"#,
    );
    krate(
        &root,
        "crates/credit/data-capabilities/request-limit",
        "request-limit",
    );
    write(
        &root,
        "crates/credit/aggregates/Account/aggregate.json",
        r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative"}},"invariants":[],"relations":[]}"#,
    );
    krate(
        &root,
        "crates/credit/aggregates/Account",
        "aggregate-account",
    );
    write(
        &root,
        "mock/pages.json",
        r#"{"version":1,"pages":[],"components":[{"name":"RiskBadge","system":"credit","description":"d","fragment":"components/RiskBadge.html","needs":["credit.calculateRisk"],"actions":["credit.requestLimitChange"]}]}"#,
    );
    write(
        &root,
        "mock/components/RiskBadge.html",
        r#"<div data-reads="credit.calculateRisk"><button data-writes="credit.requestLimitChange"></button></div>"#,
    );
    register(&root);
    Repo { root }
}

/// The `run: |` block of job `name` in gates.yml — the first step's script.
fn job_script(name: &str) -> String {
    let mut lines = GATES.lines();
    for line in lines.by_ref() {
        if line.trim_end() == format!("  {name}:") {
            break;
        }
    }
    let mut script = Vec::new();
    let mut inside = false;
    for line in lines {
        if inside {
            if line.starts_with("          ") || line.trim().is_empty() {
                script.push(line.get(10..).unwrap_or(""));
                continue;
            }
            break;
        }
        if line.trim() == "run: |" {
            inside = true;
        }
    }
    assert!(!script.is_empty(), "no `run: |` block under job {name}");
    script.join("\n")
}

/// Runs job `name`'s script at the root: its verdict and what it printed.
fn gate(repo: &Repo, name: &str) -> (bool, String) {
    let out = Command::new("bash")
        .arg("-c")
        .arg(job_script(name))
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), said)
}

/// The loop's reading of the same repository.
fn loop_violations(repo: &Repo) -> Vec<String> {
    let tree = Tree::read(&RealDisk, &repo.root);
    let mut found = violations(&tree, None);
    found.extend(undocumented_concepts(&tree));
    found
}

/// The jobs mirrored here, each with the words its violation carries on
/// the loop's side, so the same break is recognised in both readings.
fn mirrored() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("workspace-members-by-glob", "member by name"),
        ("system-frontier", "needs"),
        ("write-actions", "triggers"),
        ("ownership", "touches"),
        ("data-capability-placed", "its id's namespace"),
        ("aggregate-placed", "an aggregate's folder"),
        ("units-registered", "registry"),
        ("concepts-documented", "implements"),
        ("queries-registered", "contract C"),
    ])
}

#[test]
fn a_need_and_an_action_the_plan_s_graph_produces_pass_both_readings() {
    // The Capability and the DataCapability are not built: a milestone of the
    // plan produces them (contract R), and the Micro-UI builds against them.
    let repo = repo("planned");
    write(
        &repo.root,
        "apps/credit/risk-badge/micro-ui.json",
        r#"{"microUi":"RiskBadge","system":"credit","needs":["credit.calculateRisk","credit.listAccounts"],"actions":["credit.requestLimitChange","credit.closeAccount"]}"#,
    );
    write(
        &repo.root,
        "mock/pages.json",
        r#"{"version":1,"pages":[],"components":[{"name":"RiskBadge","system":"credit","description":"d","fragment":"components/RiskBadge.html","needs":["credit.calculateRisk","credit.listAccounts"],"actions":["credit.requestLimitChange","credit.closeAccount"]}]}"#,
    );
    write(
        &repo.root,
        "plan/graph.json",
        r#"{"version":1,"milestones":[],"ids":{"credit.listAccounts":{"kind":"capability","producer":7},"credit.closeAccount":{"kind":"data-capability","producer":7}}}"#,
    );
    for name in ["system-frontier", "write-actions"] {
        let (ok, said) = gate(&repo, name);
        assert!(ok, "{name}: {said}");
    }
    assert!(
        loop_violations(&repo).is_empty(),
        "{:?}",
        loop_violations(&repo)
    );
    // An id the graph knows but nobody produces is still undeclared.
    write(
        &repo.root,
        "plan/graph.json",
        r#"{"version":1,"milestones":[],"ids":{"credit.listAccounts":{"kind":"capability","producer":null},"credit.closeAccount":{"kind":"data-capability","producer":null}}}"#,
    );
    for (name, word) in [("system-frontier", "needs"), ("write-actions", "action")] {
        let (ok, said) = gate(&repo, name);
        assert!(!ok, "{name}: {said}");
        assert!(said.contains(word), "{name}: {said}");
    }
    assert_eq!(
        loop_violations(&repo).len(),
        2,
        "{:?}",
        loop_violations(&repo)
    );
}

#[test]
fn the_healthy_shape_passes_both_readings() {
    let repo = repo("healthy");
    for job in mirrored().keys() {
        if *job == "queries-registered" {
            // Needs sha256sum and jq to compare files; the loop's side is
            // tested on the same fixture below, the script's own cases in
            // the Rust tree tests.
            continue;
        }
        let (ok, said) = gate(&repo, job);
        assert!(ok, "{job}: {said}");
    }
    let found = loop_violations(&repo);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_micro_ui_reading_another_system_s_capability_passes_both_readings() {
    let repo = repo("cross-system");
    write(
        &repo.root,
        "apps/orders/order-risk/micro-ui.json",
        r#"{"microUi":"OrderRisk","system":"orders","description":"x","needs":["credit.calculateRisk"],"actions":[],"props":{}}"#,
    );
    let (ok, said) = gate(&repo, "system-frontier");
    assert!(ok, "reading is free: {said}");
    let found = loop_violations(&repo);
    assert!(
        !found.iter().any(|f| f.contains("credit.calculateRisk")),
        "{found:?}"
    );
}

/// One break per mirrored rule: what to write, and the job it must fail.
fn breaks() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        (
            "by-name",
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/credit/capabilities/risk\", \"crates/dataguard/server\"]\n",
            "workspace-members-by-glob",
        ),
        (
            "undeclared-need",
            "apps/credit/risk-badge/micro-ui.json",
            r#"{"microUi":"RiskBadge","system":"credit","needs":["orders.list"],"actions":[]}"#,
            "system-frontier",
        ),
        (
            "action",
            "apps/credit/risk-badge/micro-ui.json",
            r#"{"microUi":"RiskBadge","system":"credit","needs":[],"actions":["credit.nowhere"]}"#,
            "write-actions",
        ),
        (
            "ownership",
            "crates/credit/data-capabilities/request-limit/data-capability.json",
            r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard","effect":"update","target":{"aggregate":"Account"},"touches":["Account.tags"],"callableBy":["*"],"invariants":[]}"#,
            "ownership",
        ),
        (
            // ForgeDesMondes milestone 18: `campaign` DataCapabilities laid
            // under the DataGuard's folder, where no gate looked.
            "misplaced",
            "crates/dataguard/data-capabilities/close-account/data-capability.json",
            r#"{"dataCapability":"credit.closeAccount","owner":"dataguard","effect":"update","target":{"aggregate":"Account"},"touches":["Account.creditLimit"],"callableBy":["*"],"invariants":[]}"#,
            "data-capability-placed",
        ),
        (
            "aggregate-by-name",
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/credit/aggregates/Account\", \"crates/dataguard/server\"]\n",
            "workspace-members-by-glob",
        ),
        (
            // ForgeDesMondes' twelve aggregates, under the DataGuard's
            // folder until the migration moves them.
            "aggregate-under-the-dataguard",
            "crates/dataguard/aggregates/Ledger/aggregate.json",
            r#"{"aggregate":"Ledger","fields":{"total":{"kind":"derived","criticality":"low"}},"invariants":[],"relations":[]}"#,
            "aggregate-placed",
        ),
        (
            "aggregate-renamed",
            "crates/credit/aggregates/Ledgers/aggregate.json",
            r#"{"aggregate":"Ledger","fields":{"total":{"kind":"derived","criticality":"low"}},"invariants":[],"relations":[]}"#,
            "aggregate-placed",
        ),
        (
            "derived",
            "crates/credit/aggregates/Account/aggregate.json",
            r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"derived"}},"invariants":[],"relations":[]}"#,
            "ownership",
        ),
        (
            "aggregate-lib",
            "crates/credit/aggregates/Account/Cargo.toml",
            "[package]\nname = \"aggregate-account\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"account\"\n",
            "units-registered",
        ),
        (
            "data-capability-lib",
            "crates/credit/data-capabilities/request-limit/Cargo.toml",
            "[package]\nname = \"request-limit\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"limit\"\n",
            "units-registered",
        ),
        (
            // A unit whose folder the script never saw: its crate is there,
            // the registry does not name it.
            "unregistered",
            "crates/credit/capabilities/score/src/lib.rs",
            "",
            "units-registered",
        ),
        (
            "stale-registry",
            "crates/dataguard/registry/Cargo.toml",
            "[package]\nname = \"dataguard-registry\"\nversion = \"0.1.0\"\nedition = \"2024\"\nbuild = \"build.rs\"\n\n[dependencies]\ngone = { path = \"../../credit/capabilities/gone\" }\n",
            "units-registered",
        ),
        (
            "concept",
            "crates/credit/capabilities/risk/capability.json",
            r#"{"capability":"credit.calculateRisk","system":"credit","implements":"Risk@4","reads":["q.graphql"]}"#,
            "concepts-documented",
        ),
    ]
}

#[test]
fn each_break_fails_the_gate_and_the_loop_alike() {
    for (case, path, text, job) in breaks() {
        let repo = repo(case);
        write(&repo.root, path, text);
        if case == "unregistered" {
            krate(&repo.root, "crates/credit/capabilities/score", "score");
        }
        let (ok, said) = gate(&repo, job);
        assert!(!ok, "{case}: the gate {job} let it through: {said}");
        // Refused by its rule, never by a script that broke on the way.
        assert!(said.contains("::error"), "{case}: {said}");
        let found = loop_violations(&repo);
        let words = mirrored()[job];
        assert!(
            found.iter().any(|v| v.contains(words)),
            "{case}: the loop found nothing about `{words}`: {found:?}"
        );
    }
}

#[test]
fn the_ownership_rule_reads_the_touched_field_on_both_sides() {
    // Touching a declared field: fine both ways.
    let repo = repo("owned");
    let (ok, said) = gate(&repo, "ownership");
    assert!(ok, "{said}");
    assert!(
        !loop_violations(&repo).iter().any(|v| v.contains("touches")),
        "expected nothing about touches"
    );
}

#[test]
fn a_derived_field_touched_is_named_derived_on_both_sides() {
    let repo = repo("derived-words");
    write(
        &repo.root,
        "crates/credit/aggregates/Account/aggregate.json",
        r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"derived"}},"invariants":[],"relations":[]}"#,
    );
    let (ok, said) = gate(&repo, "ownership");
    assert!(!ok);
    assert!(said.contains("declares derived"), "{said}");
    assert!(
        loop_violations(&repo)
            .iter()
            .any(|v| v.contains("declares derived")),
        "{:?}",
        loop_violations(&repo)
    );
}

#[test]
fn an_aggregate_under_the_dataguard_is_sent_to_the_system_that_targets_it_on_both_sides() {
    let repo = repo("home");
    write(
        &repo.root,
        "crates/dataguard/aggregates/Account/aggregate.json",
        r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative"}},"invariants":[],"relations":[]}"#,
    );
    let (ok, said) = gate(&repo, "aggregate-placed");
    assert!(!ok);
    assert!(
        said.contains("Account belongs under crates/credit/aggregates/Account/"),
        "{said}"
    );
    assert!(
        loop_violations(&repo)
            .iter()
            .any(|v| v.contains("belongs under crates/credit/aggregates/Account/")),
        "{:?}",
        loop_violations(&repo)
    );
}

#[test]
fn a_hostless_unit_waits_on_no_registry_on_both_sides() {
    let repo = repo("hostless");
    krate(
        &repo.root,
        "crates/credit/data-capabilities/export",
        "export",
    );
    write(
        &repo.root,
        "crates/credit/data-capabilities/export/data-capability.json",
        r#"{"dataCapability":"credit.export","owner":"dataguard","effect":"insert","touches":[],"callableBy":["*"],"invariants":[],"host": "hostless"}"#,
    );
    let (ok, said) = gate(&repo, "units-registered");
    assert!(ok, "{said}");
    assert!(
        loop_violations(&repo).is_empty(),
        "{:?}",
        loop_violations(&repo)
    );
    // Without the registry, both refuse: the script writes one as soon as a
    // unit crate is there, hostless or not.
    fs::remove_file(repo.root.join("crates/dataguard/registry/Cargo.toml")).expect("remove");
    for dir in [
        "crates/credit/capabilities/risk",
        "crates/credit/data-capabilities/request-limit",
        "crates/credit/aggregates/Account",
    ] {
        fs::remove_file(repo.root.join(dir).join("Cargo.toml")).expect("remove");
    }
    write(
        &repo.root,
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/credit/data-capabilities/export\"]\n",
    );
    let (ok, said) = gate(&repo, "units-registered");
    assert!(!ok, "{said}");
    assert!(said.contains("::error"), "{said}");
    assert!(
        loop_violations(&repo)
            .iter()
            .any(|v| v.contains("is missing and 1 unit crate(s) wait on the registry")),
        "{:?}",
        loop_violations(&repo)
    );
}
