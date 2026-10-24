//! The part `queries-valid--capabilities` of `gates.yml`, as `init-repo`
//! installs it, run for real against a throwaway repository and a
//! PostgreSQL: the `pub const SQL` of a Capability the change adds or
//! changes prepares against the schema the migrations build, every
//! parameter typed as text; an untouched Capability is not judged.
//!
//! Needs `bash`, `git`, `jq`, `perl`, `psql`, and `GATES_PGPASSWORD`: the
//! password of the role `postgres` on `localhost:5432` (the harness's
//! PostgreSQL in Docker). Without it the test says so and passes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::content_of;

struct Repo {
    root: PathBuf,
    base: String,
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

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A Capability whose SQL reads `columns` of `combat`.
fn capability(folder: &str, sql: &str) -> Vec<(String, String)> {
    let dir = format!("crates/table/capabilities/{folder}");
    vec![
        (
            format!("{dir}/capability.json"),
            format!(r#"{{"capability":"table.{folder}","reads":["queries/{folder}.graphql"]}}"#),
        ),
        (
            format!("{dir}/src/lib.rs"),
            format!(
                "pub const SQL: &str = r#\"{sql}\"#;\npub const VARIABLES: &[&str] = &[\"campaign\"];\n"
            ),
        ),
    ]
}

/// The step's `run: |` block.
fn script() -> String {
    let gates = content_of(".github/workflows/gates.yml").expect("installed");
    let step = gates
        .split("- id: queries-valid--capabilities\n")
        .nth(1)
        .expect("the part");
    step.split("run: |\n")
        .nth(1)
        .expect("its script")
        .lines()
        .take_while(|line| line.starts_with("          ") || line.trim().is_empty())
        .map(|line| line.get(10..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn run(repo: &Repo, password: &str) -> (bool, String) {
    let out = Command::new("bash")
        .arg("-c")
        .arg(script())
        .env("BASE", &repo.base)
        .env("HEAD_REF", "feat/list-combats")
        .env("PGPASSWORD", password)
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn a_changed_capability_s_sql_prepares_against_the_migrated_schema() {
    let Ok(password) = std::env::var("GATES_PGPASSWORD") else {
        eprintln!("capability_sql_gate: GATES_PGPASSWORD is not set, skipped");
        return;
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "capability-sql-gate-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("mkdir");
    git(&root, &["init", "-q", "-b", "main_agent"]);
    write(
        &root,
        "crates/data/migrations/0001_combat.sql",
        "CREATE TABLE combat (id uuid PRIMARY KEY, campaign_id uuid NOT NULL, round integer NOT NULL);\n",
    );
    // An older Capability whose SQL the schema refuses: untouched, unjudged.
    for (path, text) in capability("list-old", "SELECT nothing FROM nowhere WHERE x::text = $1") {
        write(&root, &path, &text);
    }
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let repo = Repo { root, base };

    for (path, text) in capability(
        "list-combats",
        "SELECT c.id::text AS id, c.round FROM combat c WHERE c.campaign_id::text = $1",
    ) {
        write(&repo.root, &path, &text);
    }
    for (path, text) in capability(
        "list-rows",
        "SELECT r.values FROM dataguard_rows r WHERE r.partition::text = $1",
    ) {
        write(&repo.root, &path, &text);
    }
    write(
        &repo.root,
        "crates/table/capabilities/in-the-plugin/capability.json",
        r#"{"capability":"table.inThePlugin","host":"hostless"}"#,
    );
    write(
        &repo.root,
        "crates/table/capabilities/in-the-plugin/src/lib.rs",
        "pub fn exported() {}\n",
    );
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-qm", "a read"]);
    let (ok, said) = run(&repo, &password);
    assert!(ok, "{said}");
    assert!(said.contains("1 Capability SQL prepared"), "{said}");
    assert!(
        said.contains("list-rows/src/lib.rs reads the engine's own tables"),
        "{said}"
    );

    for (path, text) in capability(
        "list-combats",
        "SELECT c.id::text AS id, c.turn FROM combat c WHERE c.campaign_id::text = $1",
    ) {
        write(&repo.root, &path, &text);
    }
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-qm", "a column that is not there"]);
    let (ok, said) = run(&repo, &password);
    assert!(!ok, "{said}");
    assert!(
        said.contains("::error file=crates/table/capabilities/list-combats/src/lib.rs::its SQL does not prepare"),
        "{said}"
    );
    assert!(said.contains("turn"), "{said}");
    assert!(!said.contains("list-old"), "{said}");
    let _ = Command::new("psql")
        .args([
            "-h",
            "localhost",
            "-U",
            "postgres",
            "-d",
            "postgres",
            "-qc",
            "DROP DATABASE IF EXISTS gate_capabilities WITH (FORCE)",
        ])
        .env("PGPASSWORD", &password)
        .output();
}

#[test]
fn the_part_judges_a_task_s_pull_request_only() {
    let gates = content_of(".github/workflows/gates.yml").expect("installed");
    let step = gates
        .split("- id: queries-valid--capabilities\n")
        .nth(1)
        .and_then(|rest| rest.split("\n      - ").next())
        .expect("the part");
    for part in [
        "if: ${{ !cancelled() && github.event_name == 'pull_request' }}",
        "milestone/*) echo",
        "--diff-filter=AMR \"$BASE\"...HEAD",
        "PREPARE gate_c_$n$types AS $sql",
    ] {
        assert!(step.contains(part), "the part: {part}");
    }
}
