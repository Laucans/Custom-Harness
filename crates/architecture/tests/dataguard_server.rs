//! The write side `init-repo` installs, compiled: a throwaway product made
//! of the installed assets (`crates/dataguard/server`, the registry,
//! `crates/server`, `scripts/register.sh`), the engine taken by `path`
//! instead of `git`, one unit of each kind, `register.sh` run, `cargo check`
//! on the whole, the glue and the server's `inventory` read back, then one
//! more unit registered by its folder and the script alone. Ignored by
//! default — it fetches axum and tokio from crates.io and builds them — and
//! run on demand:
//! `cargo test -p harness-architecture --test dataguard_server -- --ignored`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use harness_architecture::assets::{FILES, content_of};

const ENGINE_LINE: &str = "harness-dataguard = { git = ";

const ACCOUNT: &str = r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"normal"}},"invariants":[{"id":"limit-under-ceiling","rule":"creditLimit <= 10000","onViolation":"reject"}],"relations":[]}"#;
const LIMIT: &str = r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard","version":2,"effect":"update","target":{"aggregate":"Account","id":"$accountId"},"touches":["Account.creditLimit"],"mode":"confirm_on_stale","permissions":["credit.limit.write"],"callableBy":["credit.*"]}"#;
const CLOSE: &str = r#"{"dataCapability":"credit.closeAccount","owner":"dataguard","version":1,"effect":"update","target":{"aggregate":"Account","id":"$accountId"},"touches":["Account.creditLimit"],"mode":"confirm_on_stale","permissions":["credit.limit.write"],"callableBy":["credit.*"]}"#;

/// The aggregate's crate: a `Judge` and its `plugin()`.
const ACCOUNT_LIB: &str = r#"use std::collections::BTreeMap;

use harness_dataguard::command::Entry;
use harness_dataguard::plugins::{Judge, Plugin};
use serde_json::Value;

struct Ceiling;

impl Judge for Ceiling {
    fn violated(&self, state: &BTreeMap<String, Value>, _entry: &Entry) -> Vec<String> {
        let limit = state.get("creditLimit").and_then(Value::as_f64).unwrap_or(0.0);
        if limit > 10_000.0 { vec!["limit-under-ceiling".to_string()] } else { Vec::new() }
    }
}

pub fn plugin() -> Plugin {
    Plugin::new().judge("Account", Ceiling)
}
"#;

/// The DataCapability's crate: a `Prepare`, a need it fills itself, and
/// its `plugin()`.
const LIMIT_LIB: &str = r#"use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use harness_dataguard::plugins::{Context, Plugin, Prepare, Prepared, Refusal};
use serde_json::Value;

/// The ceiling a request may not pass.
pub struct Policy(pub f64);

struct Limit;

#[async_trait]
impl Prepare for Limit {
    async fn prepare(
        &self,
        context: &Context<'_>,
        payload: &BTreeMap<String, Value>,
        target_id: Option<&str>,
    ) -> Result<Prepared, Refusal> {
        let ceiling = context.shared.get::<Policy>().map_or(0.0, |p| p.0);
        let asked = payload.get("creditLimit").and_then(Value::as_f64).unwrap_or(0.0);
        if asked > ceiling {
            return Err(Refusal::unprocessable("over the policy"));
        }
        Ok(Prepared {
            payload: payload.clone(),
            target_id: target_id.map(str::to_string),
            warnings: Vec::new(),
        })
    }
}

pub fn plugin() -> Plugin {
    Plugin::new()
        .prepare("credit.requestLimitChange", Limit)
        .needs::<Policy>()
        .share(Arc::new(Policy(5_000.0)))
}
"#;

/// The Capability's crate: its four items, and no engine.
const RISK_LIB: &str = r#"use std::collections::BTreeMap;

use serde_json::{Value, json};

pub const CAPABILITY: &str = "credit.calculateRisk";
pub const SQL: &str = "select score from risk where account = $1";
pub const VARIABLES: &[&str] = &["accountId"];

pub fn answer(rows: &Value, variables: &BTreeMap<String, Value>) -> Value {
    json!({
        "rows": rows.as_array().map_or(0, Vec::len),
        "accountId": variables.get("accountId").cloned().unwrap_or(Value::Null),
    })
}
"#;

/// The planted server, run once over memory: the units fit, a read is
/// answered by the Capability, a submission prepared by the
/// DataCapability, and a refusal of its `Prepare` is a 422's variant.
const PLANT_TEST: &str = r#"use std::collections::BTreeMap;

use dataguard_server::engine::Error;
use dataguard_server::engine::api::Submission;
use dataguard_server::engine::plugins::{RefusalKind, Shared};
use serde_json::json;

fn submission(limit: u64) -> Submission {
    serde_json::from_value(json!({
        "dataCapability": "credit.requestLimitChange",
        "targetId": "881",
        "payload": { "creditLimit": limit },
        "caller": {
            "by": "ana", "from": "credit.LimitForm",
            "permissions": ["credit.limit.write"], "roles": []
        }
    }))
    .expect("a submission")
}

#[tokio::test]
async fn the_registry_s_units_are_planted_over_memory() {
    let planted = dataguard_server::plant(dataguard_server::MEMORY, Shared::default())
        .await
        .expect("the units fit");
    let variables = BTreeMap::from([("accountId".to_string(), json!("881"))]);
    let answer = planted.reads.run("credit.calculateRisk", &variables).await.expect("read");
    assert_eq!(answer, json!({ "rows": 0, "accountId": "881" }));
    planted.engine.submit(submission(4_000)).await.expect("prepared and queued");
    match planted.engine.submit(submission(9_000)).await {
        Err(Error::Refused(refusal)) => assert_eq!(refusal.kind, RefusalKind::Unprocessable),
        other => panic!("refused by the Prepare: {other:?}"),
    }
}
"#;

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

/// The engine of this checkout, by `path`, with the features the skeleton
/// takes it with.
fn local_engine() -> String {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dataguard")
        .canonicalize()
        .expect("crates/dataguard beside crates/architecture");
    format!(
        "harness-dataguard = {{ path = {:?}, features = [\"postgres\", \"uuid\"] }}",
        engine.display().to_string()
    )
}

fn server_manifest() -> &'static str {
    content_of("crates/dataguard/server/Cargo.toml").expect("the server's manifest is installed")
}

#[test]
fn the_skeleton_takes_the_engine_at_a_release_tag_never_a_branch() {
    let line = server_manifest()
        .lines()
        .find(|l| l.starts_with(ENGINE_LINE))
        .expect("the git line the product takes the engine by");
    assert!(line.contains("tag = \"dataguard-v"), "{line}");
    assert!(
        !line.contains("branch =") && !line.contains("rev ="),
        "a branch moves under the product, a bare rev names no release: {line}"
    );
}

#[test]
fn the_engine_s_tag_is_written_once_and_the_units_are_the_registry_s() {
    for path in ["crates/server/Cargo.toml", "crates/server/src/main.rs"] {
        let text = content_of(path).expect("installed");
        assert!(
            !text.contains("tag = \"dataguard-v"),
            "{path} reaches the engine through dataguard_server"
        );
    }
    let server = server_manifest();
    assert_eq!(server.matches("tag = \"dataguard-v").count(), 1);
    assert!(server.contains("dataguard-registry = { path = \"../registry\" }"));
    assert!(
        !server.contains("build ="),
        "the server crate generates nothing"
    );
    let script = content_of("scripts/register.sh").expect("installed");
    assert!(script.starts_with("#!/usr/bin/env bash"));
    assert!(script.contains("set -euo pipefail"));
    assert!(script.contains("server=crates/dataguard/server/Cargo.toml"));
    assert!(
        FILES
            .iter()
            .all(|a| a.path != "crates/dataguard/registry/Cargo.toml"),
        "the registry's manifest is the script's, never an asset"
    );
}

/// Runs `program` in `root`, the product's target kept between runs.
fn run(root: &Path, program: &str, args: &[&str]) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(root)
        .env(
            "CARGO_TARGET_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("dataguard-product-target"),
        )
        .env("DATABASE_URL", "memory")
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

/// The newest glue the registry's build script wrote.
fn glue() -> String {
    let build = Path::new(env!("CARGO_TARGET_TMPDIR")).join("dataguard-product-target/debug/build");
    fs::read_dir(build)
        .expect("build dirs")
        .flatten()
        .map(|e| e.path().join("out/glue.rs"))
        .filter(|p| p.is_file())
        .max_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
        .map(|p| fs::read_to_string(p).expect("read"))
        .expect("the generated glue")
}

/// A unit crate's manifest: its package, the engine when it needs it.
fn unit_manifest(package: &str, engine: bool) -> String {
    let mut text = format!(
        "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nserde_json = \"1\"\n"
    );
    if engine {
        text.push_str("async-trait = \"0.1\"\n");
        text.push_str(&local_engine());
        text.push('\n');
    }
    text
}

#[test]
#[ignore = "fetches and builds axum and tokio; run on demand"]
fn the_installed_write_side_registers_every_unit_by_its_folder_and_the_script() {
    let product = Product(Path::new(env!("CARGO_TARGET_TMPDIR")).join("dataguard-product"));
    let root = &product.0;
    let _ = fs::remove_dir_all(root);
    for asset in FILES.iter().filter(|a| {
        a.path.starts_with("crates/dataguard/")
            || a.path.starts_with("crates/server/")
            || a.path == "scripts/register.sh"
            || a.path == ".gitignore"
    }) {
        let content = if asset.path == "crates/dataguard/server/Cargo.toml" {
            let line = asset
                .content
                .lines()
                .find(|l| l.starts_with(ENGINE_LINE))
                .expect("the git line");
            asset.content.replace(line, &local_engine())
        } else {
            asset.content.to_string()
        };
        write(root, asset.path, &content);
    }
    write(
        root,
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\n  \"crates/dataguard/server\",\n  \"crates/dataguard/registry\",\n  \"crates/server\",\n  \"crates/*/aggregates/*\",\n  \"crates/*/data-capabilities/*\",\n  \"crates/*/capabilities/*\",\n]\n",
    );
    write(root, "crates/dataguard/server/tests/plant.rs", PLANT_TEST);
    let dev =
        "\n[dev-dependencies]\ntokio = { version = \"1\", features = [\"macros\", \"rt\"] }\n";
    let server = fs::read_to_string(root.join("crates/dataguard/server/Cargo.toml")).expect("read");
    write(
        root,
        "crates/dataguard/server/Cargo.toml",
        &format!("{server}{dev}"),
    );

    let account = "crates/credit/aggregates/Account";
    write(root, &format!("{account}/aggregate.json"), ACCOUNT);
    write(
        root,
        &format!("{account}/Cargo.toml"),
        &unit_manifest("aggregate-account", true),
    );
    write(root, &format!("{account}/src/lib.rs"), ACCOUNT_LIB);
    let limit = "crates/credit/data-capabilities/request-limit";
    write(root, &format!("{limit}/data-capability.json"), LIMIT);
    write(
        root,
        &format!("{limit}/Cargo.toml"),
        &unit_manifest("request-limit", true),
    );
    write(root, &format!("{limit}/src/lib.rs"), LIMIT_LIB);
    let risk = "crates/credit/capabilities/calculate-risk";
    write(
        root,
        &format!("{risk}/capability.json"),
        r#"{"capability":"credit.calculateRisk"}"#,
    );
    write(
        root,
        &format!("{risk}/Cargo.toml"),
        &unit_manifest("calculate-risk", false),
    );
    write(root, &format!("{risk}/src/lib.rs"), RISK_LIB);
    // A hostless DataCapability runs in no server: never registered.
    let plugin = "crates/credit/data-capabilities/in-the-plugin";
    write(
        root,
        &format!("{plugin}/data-capability.json"),
        r#"{"dataCapability":"credit.inThePlugin", "host": "hostless"}"#,
    );
    write(
        root,
        &format!("{plugin}/Cargo.toml"),
        &unit_manifest("in-the-plugin", false),
    );
    write(root, &format!("{plugin}/src/lib.rs"), "");

    // The script writes the registry: one line per unit crate, sorted, and
    // the engine's line as the server has it.
    ok(root, "bash", &["scripts/register.sh"]);
    let registry =
        fs::read_to_string(root.join("crates/dataguard/registry/Cargo.toml")).expect("written");
    let lines: Vec<&str> = registry
        .lines()
        .skip_while(|l| *l != "[dependencies]")
        .skip(1)
        .collect();
    assert_eq!(
        lines,
        [
            local_engine().as_str(),
            "aggregate-account = { path = \"../../credit/aggregates/Account\" }",
            "calculate-risk = { path = \"../../credit/capabilities/calculate-risk\" }",
            "request-limit = { path = \"../../credit/data-capabilities/request-limit\" }",
        ],
        "{registry}"
    );
    ok(root, "bash", &["scripts/register.sh", "--check"]);

    ok(root, "cargo", &["check", "--all-targets"]);
    // What the product's CI runs on the installed code, glue included.
    ok(
        root,
        "cargo",
        &["clippy", "--all-targets", "--", "-D", "warnings"],
    );
    let generated = glue();
    for item in [
        "aggregate_account::plugin",
        "request_limit::plugin",
        "calculate_risk::CAPABILITY",
        "calculate_risk::SQL",
        "calculate_risk::VARIABLES",
        "calculate_risk::answer",
        "aggregate.json",
        "data-capability.json",
    ] {
        assert!(
            generated.contains(item),
            "the glue names {item}:\n{generated}"
        );
    }
    assert!(!generated.contains("in_the_plugin"), "{generated}");

    let inventory = ok(
        root,
        "cargo",
        &["run", "-q", "-p", "server", "--", "inventory"],
    );
    let line_of = |folder: &str| {
        inventory
            .lines()
            .find(|l| l.contains(folder))
            .unwrap_or_else(|| panic!("{folder} is listed:\n{inventory}"))
            .to_string()
    };
    assert!(line_of(account).starts_with("aggregate "), "{inventory}");
    assert!(line_of(account).ends_with("needs: -"), "{inventory}");
    assert!(
        line_of(limit).starts_with("data-capability "),
        "{inventory}"
    );
    assert!(
        line_of(limit).ends_with("needs: request_limit::Policy"),
        "{inventory}"
    );
    assert!(line_of(risk).starts_with("capability "), "{inventory}");
    assert_eq!(inventory.lines().count(), 3, "{inventory}");

    ok(root, "cargo", &["test", "-q", "-p", "dataguard-server"]);

    // One more DataCapability: its folder and one rerun of the script, no
    // other file of the product touched.
    ok(root, "git", &["init", "-q"]);
    ok(root, "git", &["add", "-A"]);
    ok(
        root,
        "git",
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "base",
        ],
    );
    let close = "crates/credit/data-capabilities/close-account";
    write(root, &format!("{close}/data-capability.json"), CLOSE);
    write(
        root,
        &format!("{close}/Cargo.toml"),
        &unit_manifest("close-account", true),
    );
    write(
        root,
        &format!("{close}/src/lib.rs"),
        "pub fn plugin() -> harness_dataguard::plugins::Plugin {\n    harness_dataguard::plugins::Plugin::new()\n}\n",
    );
    assert!(
        !run(root, "bash", &["scripts/register.sh", "--check"])
            .status
            .success(),
        "a unit the registry does not name fails the check"
    );
    ok(root, "bash", &["scripts/register.sh"]);
    let status = ok(
        root,
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    );
    let mut changed: Vec<&str> = status.lines().collect();
    changed.sort_unstable();
    assert_eq!(
        changed,
        [
            " M crates/dataguard/registry/Cargo.toml",
            "?? crates/credit/data-capabilities/close-account/",
        ],
        "{status}"
    );
    ok(root, "cargo", &["check", "--all-targets"]);
    assert!(glue().contains("close_account::plugin"));

    // A folder the script was not run for stops the build, named.
    let forgotten = "crates/credit/data-capabilities/forgotten";
    write(
        root,
        &format!("{forgotten}/Cargo.toml"),
        &unit_manifest("forgotten", false),
    );
    write(root, &format!("{forgotten}/src/lib.rs"), "");
    let refused = run(root, "cargo", &["check", "-p", "dataguard-registry"]);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{stderr}");
    assert!(
        stderr.contains("run scripts/register.sh") && stderr.contains(forgotten),
        "{stderr}"
    );
}
