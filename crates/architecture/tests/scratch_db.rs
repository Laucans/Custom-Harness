//! The scratch-database guard `init-repo` installs, compiled and tested: a
//! throwaway product holding only `crates/platform/scratch-db`, the engine
//! taken by `path` instead of `git`, its own tests run — and, where
//! `SCRATCH_DB_URL` names a PostgreSQL whose role may create databases, the
//! one that proves a panicking test leaves no database behind.
//!
//! Ignored by default — it builds tokio-postgres from crates.io — and run
//! on demand:
//! `SCRATCH_DB_URL=postgres://app:…@localhost:5432/app cargo test -p
//! harness-architecture --test scratch_db -- --ignored`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::FILES;

/// The throwaway product, removed when the test ends.
struct Product(PathBuf);

impl Drop for Product {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "builds tokio-postgres; SCRATCH_DB_URL for the database half"]
fn the_installed_guard_compiles_against_the_engine_and_passes_its_tests() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let product = Product(
        std::env::temp_dir().join(format!("scratch-db-product-{}-{nanos}", std::process::id())),
    );
    let root = &product.0;
    for asset in FILES
        .iter()
        .filter(|a| a.path.starts_with("crates/platform/scratch-db/"))
    {
        let path = root.join(asset.path);
        fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        fs::write(path, asset.content).expect("write");
    }
    let engine = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dataguard")
        .canonicalize()
        .expect("crates/dataguard beside crates/architecture");
    // The product's own workspace line, the engine by path: no postgres
    // feature here, the guard asks for it itself.
    fs::write(
        root.join("Cargo.toml"),
        format!(
            "[workspace]\nresolver = \"3\"\nmembers = [\"crates/platform/scratch-db\"]\n\n\
             [workspace.dependencies]\nharness-dataguard = {{ path = {:?}, features = [\"uuid\"] }}\n",
            engine.display().to_string()
        ),
    )
    .expect("write");
    let mut test = Command::new("cargo");
    test.args(["test", "--quiet"])
        .current_dir(root)
        .env(
            "CARGO_TARGET_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("scratch-db-product-target"),
        )
        .env_remove("DATABASE_URL");
    if let Ok(url) = std::env::var("SCRATCH_DB_URL") {
        test.env("DATABASE_URL", url);
    }
    let output = test.output().expect("cargo runs");
    assert!(
        output.status.success(),
        "cargo test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
