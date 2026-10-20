//! The server skeleton `init-repo` installs, compiled: a throwaway product
//! around `crates/architecture/assets/dataguard-server/`, the engine taken
//! by `path` instead of `git`, `cargo check` on it, and the registry its
//! build script generates read back. Ignored by default — it fetches axum
//! and tokio from crates.io and builds them — and run on demand:
//! `cargo test -p harness-architecture --test dataguard_server -- --ignored`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CARGO_TOML: &str = include_str!("../assets/dataguard-server/Cargo.toml");
const BUILD_RS: &str = include_str!("../assets/dataguard-server/build.rs");
const MAIN_RS: &str = include_str!("../assets/dataguard-server/src/main.rs");

const ACCOUNT: &str = r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"normal"}},"invariants":[{"id":"limit-under-ceiling","rule":"creditLimit <= 10000","onViolation":"reject"}],"relations":[]}"#;
const LIMIT: &str = r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard","version":2,"effect":"update","target":{"aggregate":"Account","id":"$accountId"},"touches":["Account.creditLimit"],"mode":"confirm_on_stale","permissions":["credit.limit.write"],"callableBy":["credit.*"]}"#;

struct Product(PathBuf);

impl Drop for Product {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

#[test]
fn the_skeleton_takes_the_engine_at_a_release_tag_never_a_branch() {
    let line = CARGO_TOML
        .lines()
        .find(|l| l.starts_with("harness-dataguard = { git = "))
        .expect("the git line the product takes the engine by");
    assert!(line.contains("tag = \"dataguard-v"), "{line}");
    assert!(
        !line.contains("branch =") && !line.contains("rev ="),
        "a branch moves under the product, a bare rev names no release: {line}"
    );
}

/// The skeleton's manifest with the engine taken from this checkout.
fn local_manifest() -> String {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dataguard")
        .canonicalize()
        .expect("crates/dataguard beside crates/architecture");
    let line = CARGO_TOML
        .lines()
        .find(|l| l.starts_with("harness-dataguard = { git = "))
        .expect("the git line the product takes the engine by");
    CARGO_TOML.replace(
        line,
        &format!(
            "harness-dataguard = {{ path = {:?}, features = [\"postgres\"] }}",
            engine.display().to_string()
        ),
    )
}

#[test]
#[ignore = "fetches and builds axum and tokio; run on demand"]
fn the_installed_server_skeleton_compiles_and_registers_what_the_checkout_holds() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let product = Product(std::env::temp_dir().join(format!(
        "harness-dataguard-server-{}-{nanos}",
        std::process::id()
    )));
    let root = &product.0;
    write(
        root,
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/dataguard/server\"]\n",
    );
    write(
        root,
        "crates/dataguard/server/Cargo.toml",
        &local_manifest(),
    );
    write(root, "crates/dataguard/server/build.rs", BUILD_RS);
    write(root, "crates/dataguard/server/src/main.rs", MAIN_RS);
    write(
        root,
        "crates/credit/aggregates/Account/aggregate.json",
        ACCOUNT,
    );
    write(
        root,
        "crates/credit/data-capabilities/request-limit/data-capability.json",
        LIMIT,
    );
    write(root, "queries/registry.json", "{\"queries\":[]}\n");
    let output = Command::new("cargo")
        .args(["check", "--all-targets"])
        .current_dir(root)
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "the skeleton does not compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The build script found both manifests, three levels under the server.
    let registry = fs::read_dir(root.join("target/debug/build"))
        .expect("build dirs")
        .flatten()
        .map(|e| e.path().join("out/registry.rs"))
        .find(|p| p.is_file())
        .expect("the generated registry");
    let generated = fs::read_to_string(registry).expect("read");
    assert!(generated.contains("aggregate.json"), "{generated}");
    assert!(generated.contains("data-capability.json"), "{generated}");
    assert!(generated.contains("queries/registry.json"), "{generated}");
}
