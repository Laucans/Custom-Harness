//! The skeletons and the test kit, compiled in a throwaway product: the
//! installed `crates/platform/{scratch-db,test-kit,conformance}`, a Data
//! layer as small as the kit needs it, one aggregate, then a
//! `DataCapability`'s skeleton and a Capability's rendered as the dev loop
//! renders them, the engine taken by `path` instead of `git`.
//!
//! What it proves: every rendered file is `rustfmt`-clean (a product's CI
//! runs `cargo fmt --check`), both units pass `cargo check --all-targets`
//! and the kit passes `cargo clippy -D warnings` and its own tests; a
//! skeleton's frames pass where they need no judgement, its holes fail its
//! tests (an unfilled manifest is refused by the engine), and once the
//! manifest is filled the unit is planted beside its aggregate through the
//! kit. The installed `scripts/dead-code.sh` passes on the fresh kit.
//! Where `SCRATCH_DB_URL` names a PostgreSQL whose role may create
//! databases, the kit's `Scratch` migrates and seeds a database of its own.
//!
//! Ignored by default — it builds tokio-postgres and the engine — and run on
//! demand: `cargo test -p harness-architecture --test skeleton_product --
//! --ignored`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::FILES;
use harness_architecture::skeleton::{AggregateCrate, MARKER, render, target};
use harness_architecture::tree::Tree;
use harness_architecture::vocabulary::Declaration;

/// The throwaway product, removed when the test ends.
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

/// The product's Data layer, as small as the kit needs it: `migrate`
/// applies every file of a folder in name order.
const DATA: &str = r#"//! A product's Data layer, as small as the kit needs it.
pub use tokio_postgres::Client;

/// Applies every `*.sql` of `dir`, in name order.
///
/// # Errors
///
/// A file that cannot be read or applied.
pub async fn migrate(client: &mut Client, dir: &std::path::Path) -> Result<Vec<String>, String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    files.sort();
    let mut applied = Vec::new();
    for file in files {
        let sql = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        client.batch_execute(&sql).await.map_err(|e| e.to_string())?;
        applied.push(file.display().to_string());
    }
    Ok(applied)
}
"#;

const TOKEN: &str = r#"{"aggregate":"Token","fields":{"q":{"kind":"authoritative","criticality":"normal"}},"invariants":[{"id":"on-the-map","rule":"q >= 0","onViolation":"reject"}],"relations":[]}"#;

const TOKEN_LIB: &str = "pub fn plugin() -> harness_dataguard::plugins::Plugin {\n    harness_dataguard::plugins::Plugin::new()\n}\n";

/// A product test of the kit over PostgreSQL: migrated, seeded, read.
const SCRATCH_TEST: &str = r#"use platform_test_kit::Scratch;
use serde_json::json;

#[tokio::test]
async fn a_scratch_is_migrated_seeded_and_read() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let scratch = Scratch::new("kit_scratch").await;
    scratch.seed("INSERT INTO token (id, q) VALUES ('t1', 3)").await;
    let rows = scratch.rows("SELECT id, q FROM token WHERE id = $1", &[json!("t1")]).await;
    assert_eq!(rows, vec![json!({ "id": "t1", "q": 3 })]);
}
"#;

fn run(root: &Path, program: &str, args: &[&str]) -> Output {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(root)
        .env(
            "CARGO_TARGET_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("skeleton-product-target"),
        )
        .env_remove("DATABASE_URL");
    if let Ok(url) = std::env::var("SCRATCH_DB_URL") {
        command.env("DATABASE_URL", url);
    }
    command
        .output()
        .unwrap_or_else(|e| panic!("{program} runs: {e}"))
}

fn ok(root: &Path, program: &str, args: &[&str]) -> String {
    let output = run(root, program, args);
    assert!(
        output.status.success(),
        "{program} {args:?} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
#[ignore = "builds tokio-postgres and the engine; SCRATCH_DB_URL for the database half"]
fn the_skeletons_compile_on_the_kit_and_their_holes_fail_their_tests() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let product = Product(
        std::env::temp_dir().join(format!("skeleton-product-{}-{nanos}", std::process::id())),
    );
    let root = &product.0;
    for asset in FILES.iter().filter(|a| {
        a.path.starts_with("crates/platform/scratch-db/")
            || a.path.starts_with("crates/platform/test-kit/")
            || a.path.starts_with("crates/platform/conformance/")
            || a.path == "scripts/dead-code.sh"
    }) {
        write(root, asset.path, asset.content);
    }
    // A fresh product's dead-code gate passes on the installed kit: every
    // `pub` item of the kit is written somewhere else, its own tests count.
    ok(root, "git", &["init", "--quiet"]);
    ok(root, "git", &["add", "--all"]);
    ok(root, "bash", &["scripts/dead-code.sh"]);
    let engine = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dataguard")
        .canonicalize()
        .expect("crates/dataguard beside crates/architecture");
    write(
        root,
        "Cargo.toml",
        &format!(
            "[workspace]\nresolver = \"3\"\nmembers = [\n  \"crates/*/aggregates/*\",\n  \"crates/*/capabilities/*\",\n  \"crates/*/data-capabilities/*\",\n  \"crates/data\",\n]\n\n[workspace.package]\nedition = \"2024\"\nrust-version = \"1.99\"\n\n[workspace.dependencies]\nharness-dataguard = {{ path = {:?}, features = [\"postgres\", \"uuid\"] }}\n",
            engine.display().to_string()
        ),
    );
    write(
        root,
        "crates/data/Cargo.toml",
        "[package]\nname = \"data\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\ntokio-postgres = { version = \"0.7\", default-features = false }\n",
    );
    write(root, "crates/data/src/lib.rs", DATA);
    write(
        root,
        "crates/data/migrations/0001_token.sql",
        "CREATE TABLE token (id text PRIMARY KEY, q integer NOT NULL);\n",
    );
    let token = "crates/map/aggregates/Token";
    write(root, &format!("{token}/aggregate.json"), TOKEN);
    write(
        root,
        &format!("{token}/Cargo.toml"),
        "[package]\nname = \"aggregate-token\"\nversion = \"0.1.0\"\nedition.workspace = true\npublish = false\n\n[dependencies]\nharness-dataguard = { workspace = true }\n",
    );
    write(root, &format!("{token}/src/lib.rs"), TOKEN_LIB);
    write(
        root,
        "crates/platform/test-kit/tests/scratch.rs",
        SCRATCH_TEST,
    );
    let kit = fs::read_to_string(root.join("crates/platform/test-kit/Cargo.toml")).expect("read");
    write(
        root,
        "crates/platform/test-kit/Cargo.toml",
        &kit.replace(
            "[dev-dependencies]\n",
            "[dev-dependencies]\nserde_json = \"1\"\n",
        ),
    );

    let aggregate = AggregateCrate {
        name: "Token".to_string(),
        dir: token.to_string(),
        package: "aggregate-token".to_string(),
    };
    let set_visibility = Declaration::parse(
        "unit: data-capability\nsystem: map\neffect: update\ntouches: q\naggregate: Token\nproduces: map.setVisibility\n",
    )
    .expect("a declaration");
    let list = Declaration::parse(
        "unit: capability\nsystem: map\nconcept: Distance@1\nproduces: map.listTokens\n",
    )
    .expect("a declaration");
    let mut rust = Vec::new();
    for (declaration, projects) in [(&set_visibility, true), (&list, false)] {
        let target = target(declaration).expect("a target");
        let skeleton = render(
            &target,
            declaration,
            &Tree::default(),
            Some(&aggregate),
            projects,
        )
        .expect("rendered");
        for (path, text) in &skeleton.files {
            assert!(!text.contains("{{"), "{path} keeps a placeholder");
            write(root, path, text);
            if path.ends_with(".rs") {
                rust.push(path.clone());
            }
        }
        assert!(skeleton.holes() > 0);
    }
    write(root, "mock/fixtures/map.listTokens.json", "[]\n");
    write(
        root,
        "concepts/Distance/v1/fixtures.json",
        "{\"concept\":\"Distance\",\"version\":1,\"cases\":[]}\n",
    );

    let config = root.join("rustfmt.toml");
    fs::write(&config, "").expect("config");
    for path in &rust {
        let out = Command::new("rustfmt")
            .args(["--check", "--edition", "2024", "--config-path"])
            .arg(&config)
            .arg(root.join(path))
            .output()
            .expect("rustfmt runs");
        assert!(
            out.status.success(),
            "{path} is not rustfmt-clean:\n{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    ok(
        root,
        "cargo",
        &["check", "--quiet", "--workspace", "--all-targets"],
    );
    ok(
        root,
        "cargo",
        &[
            "clippy",
            "--quiet",
            "-p",
            "platform-test-kit",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    );
    ok(
        root,
        "cargo",
        &["test", "--quiet", "-p", "platform-test-kit"],
    );

    // The frames that need no judgement pass; the holes fail.
    ok(
        root,
        "cargo",
        &[
            "test",
            "--quiet",
            "-p",
            "set-visibility",
            "--test",
            "prepare",
            "--",
            "--skip",
            "scaffold",
        ],
    );
    let holes = run(
        root,
        "cargo",
        &[
            "test",
            "--quiet",
            "-p",
            "set-visibility",
            "--test",
            "prepare",
        ],
    );
    assert!(
        !holes.status.success(),
        "the hole of the acceptance criteria fails"
    );
    let unfilled = run(
        root,
        "cargo",
        &[
            "test",
            "--quiet",
            "-p",
            "set-visibility",
            "--test",
            "engine",
            "planted",
        ],
    );
    let said = String::from_utf8_lossy(&unfilled.stdout);
    assert!(
        !unfilled.status.success(),
        "an unfilled manifest is refused"
    );
    assert!(
        said.contains("contract F") || said.contains("refuses"),
        "{said}"
    );
    ok(
        root,
        "cargo",
        &[
            "test",
            "--quiet",
            "-p",
            "list-tokens",
            "--",
            "--skip",
            "scaffold",
            "--skip",
            "conformance",
        ],
    );

    // The manifest filled: the unit is planted beside its aggregate.
    write(
        root,
        "crates/map/data-capabilities/set-visibility/data-capability.json",
        r#"{"dataCapability":"map.setVisibility","owner":"dataguard","version":1,"effect":"update","target":{"aggregate":"Token","id":"$token"},"touches":["Token.q"],"payload":{"q":"integer — the column"},"mode":"overwrite","invariants":["on-the-map"],"permissions":[],"callableBy":["*"],"idempotencyKey":"optional"}"#,
    );
    ok(
        root,
        "cargo",
        &[
            "test",
            "--quiet",
            "-p",
            "set-visibility",
            "--test",
            "engine",
            "planted",
        ],
    );
    let left =
        fs::read_to_string(root.join("crates/map/data-capabilities/set-visibility/src/lib.rs"))
            .expect("read");
    assert!(
        left.contains(MARKER),
        "the code's holes are still there to fill"
    );
}
