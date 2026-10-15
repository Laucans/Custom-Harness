//! `scripts/isolation.sh` against real workspaces: the healthy shape passes,
//! and each way a slice could reach another — direct, transitive, through a
//! plumbing crate turned bridge, across systems, by an inclusion — fails
//! with the crate named.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SCRIPT: &str = include_str!("../assets/isolation.sh");

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

fn krate(root: &Path, dir: &str, name: &str, dependencies: &[&str]) {
    let deps: String = dependencies
        .iter()
        .map(|d| format!("{d} = {{ path = \"{}\" }}\n", rel(dir, d)))
        .collect();
    write(
        root,
        &format!("{dir}/Cargo.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}"
        ),
    );
    write(root, &format!("{dir}/src/lib.rs"), "");
}

/// The path of crate `name` relative to `dir`: every crate's folder is its name under a known parent.
fn rel(dir: &str, name: &str) -> String {
    let target = DIRS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| *d)
        .expect("a known crate");
    let ups = dir.split('/').count();
    format!("{}{}", "../".repeat(ups), target)
}

/// Every crate a case may name: its name and its folder.
const DIRS: [(&str, &str); 7] = [
    ("risk", "crates/credit/capabilities/risk"),
    ("score", "crates/credit/capabilities/score"),
    ("credit-query", "crates/credit/query"),
    ("credit-utils", "crates/credit/utils"),
    ("dataguard", "crates/dataguard"),
    ("limit", "crates/dataguard/data-capabilities/limit"),
    ("orders-query", "crates/orders/query"),
];

/// The healthy shape: two Capabilities on their system's plumbing, one
/// DataCapability on the DataGuard, a crate of another system beside.
fn repo(case: &str) -> Repo {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "harness-isolation-{case}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("root");
    let members: Vec<String> = DIRS.iter().map(|(_, d)| format!("\"{d}\"")).collect();
    write(
        &root,
        "Cargo.toml",
        &format!(
            "[workspace]\nresolver = \"2\"\nmembers = [{}]\n",
            members.join(", ")
        ),
    );
    krate(&root, "crates/credit/query", "credit-query", &[]);
    krate(&root, "crates/credit/utils", "credit-utils", &[]);
    krate(
        &root,
        "crates/credit/capabilities/risk",
        "risk",
        &["credit-query"],
    );
    krate(
        &root,
        "crates/credit/capabilities/score",
        "score",
        &["credit-query"],
    );
    krate(&root, "crates/dataguard", "dataguard", &[]);
    krate(
        &root,
        "crates/dataguard/data-capabilities/limit",
        "limit",
        &["dataguard"],
    );
    krate(&root, "crates/orders/query", "orders-query", &[]);
    write(&root, "scripts/isolation.sh", SCRIPT);
    Repo { root }
}

/// The script's verdict and what it said.
fn check(repo: &Repo) -> (bool, String) {
    let out = Command::new("bash")
        .arg("scripts/isolation.sh")
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

#[test]
fn the_healthy_shape_passes() {
    let repo = repo("healthy");
    let (ok, said) = check(&repo);
    assert!(ok, "{said}");
    assert!(!said.contains("::error"), "{said}");
}

#[test]
fn a_capability_depending_on_another_fails_naming_both() {
    let repo = repo("direct");
    krate(
        &repo.root,
        "crates/credit/capabilities/score",
        "score",
        &["credit-query", "risk"],
    );
    let (ok, said) = check(&repo);
    assert!(!ok);
    assert!(said.contains("score reaches risk"), "{said}");
    assert!(
        said.contains("file=crates/credit/capabilities/score/Cargo.toml"),
        "{said}"
    );
}

#[test]
fn a_plumbing_crate_that_reaches_a_slice_is_a_bridge_and_fails() {
    let repo = repo("transitive");
    // score -> credit-utils -> risk: Cargo.toml of score names no capability, the graph does.
    krate(&repo.root, "crates/credit/utils", "credit-utils", &["risk"]);
    krate(
        &repo.root,
        "crates/credit/capabilities/score",
        "score",
        &["credit-query", "credit-utils"],
    );
    let (ok, said) = check(&repo);
    assert!(!ok);
    assert!(said.contains("score reaches risk"), "{said}");
    assert!(
        said.contains("credit-utils reaches the slice risk"),
        "{said}"
    );
}

#[test]
fn a_capability_reaching_another_system_fails() {
    let repo = repo("system");
    krate(
        &repo.root,
        "crates/credit/capabilities/score",
        "score",
        &["credit-query", "orders-query"],
    );
    let (ok, said) = check(&repo);
    assert!(!ok);
    assert!(
        said.contains("score (credit) reaches orders-query (orders)"),
        "{said}"
    );
}

#[test]
fn a_data_capability_reaching_a_capability_fails() {
    let repo = repo("write-side");
    krate(
        &repo.root,
        "crates/dataguard/data-capabilities/limit",
        "limit",
        &["dataguard", "risk"],
    );
    let (ok, said) = check(&repo);
    assert!(!ok);
    assert!(
        said.contains("a DataCapability depends on no other slice: limit reaches risk"),
        "{said}"
    );
}

#[test]
fn an_inclusion_leaving_the_slice_fails_but_its_own_manifest_is_fine() {
    let repo = repo("inclusion");
    // Its own manifest beside src/ is inside the slice: allowed.
    write(
        &repo.root,
        "crates/credit/capabilities/risk/capability.json",
        "{}",
    );
    write(
        &repo.root,
        "crates/credit/capabilities/risk/src/lib.rs",
        "pub const MANIFEST: &str = include_str!(\"../capability.json\");\n",
    );
    let (ok, said) = check(&repo);
    assert!(ok, "{said}");
    // A file of another slice pulled in by path: refused.
    write(
        &repo.root,
        "crates/credit/capabilities/score/src/lib.rs",
        "#[path = \"../../risk/src/lib.rs\"]\nmod shared;\n",
    );
    let (ok, said) = check(&repo);
    assert!(!ok);
    assert!(
        said.contains("leaves crates/credit/capabilities/score"),
        "{said}"
    );
    assert!(
        said.contains("file=crates/credit/capabilities/score/src/lib.rs"),
        "{said}"
    );
}

/// A crate with a registry dependency: `cargo metadata` resolves it, so the
/// fixture needs a lockfile-free way to name one. `serde` is in every
/// registry index the test machine has; the driver names are what the
/// script looks for, and `cargo metadata --no-deps` never downloads them.
fn krate_with_registry(root: &Path, dir: &str, name: &str, deps: &str, dev_deps: &str) {
    write(
        root,
        &format!("{dir}/Cargo.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}\n[dev-dependencies]\n{dev_deps}"
        ),
    );
    write(root, &format!("{dir}/src/lib.rs"), "");
}

#[test]
fn a_capability_holding_a_database_driver_fails() {
    let repo = repo("driver");
    krate_with_registry(
        &repo.root,
        "crates/credit/capabilities/score",
        "score",
        "credit-query = { path = \"../../query\" }\nsqlx = \"0.8\"\n",
        "",
    );
    let (ok, said) = check(&repo);
    assert!(!ok);
    assert!(said.contains("score depends on sqlx"), "{said}");
    assert!(said.contains("never a database driver"), "{said}");
}

#[test]
fn a_driver_or_a_slice_as_a_dev_dependency_is_not_a_dependency() {
    let repo = repo("dev-dependency");
    // The conformance runner is exactly this: a test-only crate from another
    // folder, and a driver a test may open a fixture database with.
    krate_with_registry(
        &repo.root,
        "crates/credit/capabilities/score",
        "score",
        "credit-query = { path = \"../../query\" }\n",
        "sqlx = \"0.8\"\nrisk = { path = \"../risk\" }\n",
    );
    let (ok, said) = check(&repo);
    assert!(ok, "{said}");
}
