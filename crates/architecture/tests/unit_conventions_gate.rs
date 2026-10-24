//! The gates `data-capability-conventions` and `capability-conventions` of
//! `gates.yml`, as `init-repo` installs them, run for real against a
//! throwaway repository: a unit the change adds keeps every convention, a
//! unit it changes keeps the ones every older unit already keeps, and an
//! older unit's past deviations never turn an edit of it red.
//!
//! Needs `bash`, `git` and `node`; skipped, loudly, without node.

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

/// An older DataCapability: a pre-plugin `names()`, its manifest planted
/// through a constant — what the gate never holds against it.
const OLD_DC: &[(&str, &str)] = &[
    (
        "crates/table/data-capabilities/end-combat/data-capability.json",
        r#"{"dataCapability":"table.endCombat","target":{"aggregate":"Combat"}}"#,
    ),
    (
        "crates/table/data-capabilities/end-combat/Cargo.toml",
        "[package]\nname = \"end-combat\"\n\n[dependencies]\nserde_json = \"1\"\n",
    ),
    (
        "crates/table/data-capabilities/end-combat/src/lib.rs",
        "pub const DATA_CAPABILITY: &str = \"table.endCombat\";\npub fn names(id: &str) -> String { id.to_string() }\n",
    ),
];

/// An older Capability that filters twice, its query file camelCased.
const OLD_CAP: &[(&str, &str)] = &[
    (
        "crates/table/capabilities/list-combats/capability.json",
        r#"{"capability":"table.listCombats","input":{"campaign":"string"},"reads":["queries/listCombats.graphql"]}"#,
    ),
    (
        "crates/table/capabilities/list-combats/src/lib.rs",
        "pub const SQL: &str = r#\"SELECT c.id::text AS id, c.round FROM combat c WHERE c.campaign_id::text = $1\"#;\npub const VARIABLES: &[&str] = &[\"campaign\"];\npub fn answer(rows: &Value, variables: &BTreeMap<String, Value>) -> Value {\n    let campaign = variables.get(\"campaign\");\n    rows.clone()\n}\n",
    ),
    (
        "crates/table/queries/listCombats.graphql",
        "# fields: combat.id, combat.round\nquery ListCombats($campaign: ID!) {\n  combats(campaign: $campaign) {\n    id\n    round\n  }\n}\n",
    ),
];

/// A DataCapability as the skeleton writes it, filled.
fn clean_dc(folder: &str, id: &str) -> Vec<(String, String)> {
    let dir = format!("crates/table/data-capabilities/{folder}");
    vec![
        (
            format!("{dir}/data-capability.json"),
            format!(r#"{{"dataCapability":"{id}","target":{{"aggregate":"Combat"}}}}"#),
        ),
        (
            format!("{dir}/Cargo.toml"),
            format!(
                "[package]\nname = \"{folder}\"\n\n[dependencies]\nharness-dataguard = {{ workspace = true }}\n\n[dev-dependencies]\nplatform-test-kit = {{ path = \"../../../platform/test-kit\" }}\n"
            ),
        ),
        (
            format!("{dir}/src/lib.rs"),
            format!(
                "pub const DATA_CAPABILITY: &str = \"{id}\";\nconst ROUNDS: &str = \"SELECT round FROM combat WHERE id::text = $1\";\nasync fn read(cx: &Context<'_>) {{ cx.rows.unwrap().rows(ROUNDS, &[]).await; let at = cx.now_millis; }}\n#[cfg(test)]\nmod tests {{ fn t() {{ std::time::SystemTime::now(); }} }}\n"
            ),
        ),
        (
            format!("{dir}/tests/engine.rs"),
            "const M: &str = include_str!(\"../data-capability.json\");\n".to_string(),
        ),
        (
            format!("{dir}/tests/postgres.rs"),
            "use platform_test_kit::Scratch;\n".to_string(),
        ),
    ]
}

/// A Capability as the skeleton writes it, filled.
fn clean_cap(folder: &str, id: &str) -> Vec<(String, String)> {
    let dir = format!("crates/table/capabilities/{folder}");
    vec![
        (
            format!("{dir}/capability.json"),
            format!(r#"{{"capability":"{id}","input":{{"combat":"string"}},"reads":["queries/{folder}.graphql"]}}"#),
        ),
        (
            format!("{dir}/src/lib.rs"),
            "pub const SQL: &str = r#\"SELECT t.id::text AS id, t.hit_points FROM token t WHERE t.combat_id::text = $1\"#;\npub const VARIABLES: &[&str] = &[\"combat\"];\npub fn answer(rows: &Value, _variables: &BTreeMap<String, Value>) -> Value {\n    let key = \"{\"; answer_rows(rows)\n}\n#[cfg(test)]\nmod tests { const X: &str = \"combat\"; }\n".to_string(),
        ),
        (
            format!("crates/table/queries/{folder}.graphql"),
            "# fields: token.id, token.hitPoints\nquery ListTokens($combat: ID!) {\n  tokens(combat: $combat) {\n    id\n    hitPoints\n  }\n}\n".to_string(),
        ),
    ]
}

fn repo(case: &str) -> Option<Repo> {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("unit_conventions_gate: node is not on the path, skipped");
        return None;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "unit-conventions-gate-{case}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("mkdir");
    git(&root, &["init", "-q", "-b", "main_agent"]);
    write(
        &root,
        "scripts/unit-conventions-gate.mjs",
        content_of("scripts/unit-conventions-gate.mjs").expect("installed"),
    );
    for (path, text) in OLD_DC.iter().chain(OLD_CAP) {
        write(&root, path, text);
    }
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    Some(Repo { root, base })
}

/// The gate's `run: |` block, as the step runs it.
fn script(gate: &str) -> String {
    let gates = content_of(".github/workflows/gates.yml").expect("installed");
    let step = gates
        .split(&format!("- id: {gate}\n"))
        .nth(1)
        .expect("the gate");
    step.split("run: |\n")
        .nth(1)
        .expect("its script")
        .lines()
        .take_while(|line| line.starts_with("          ") || line.trim().is_empty())
        .map(|line| line.get(10..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Commits `files` on top of the base (an empty text deletes), then runs
/// `gate` as the pull request from `head`.
fn change(repo: &Repo, gate: &str, head: &str, files: &[(String, String)]) -> (bool, String) {
    for (path, text) in files {
        if text.is_empty() {
            let _ = fs::remove_file(repo.root.join(path));
        } else {
            write(&repo.root, path, text);
        }
    }
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-qm", "change", "--allow-empty"]);
    let out = Command::new("bash")
        .arg("-c")
        .arg(script(gate))
        .env("BASE", &repo.base)
        .env("HEAD_REF", head)
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

fn owned(files: &[(&str, &str)]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|(p, t)| ((*p).to_string(), (*t).to_string()))
        .collect()
}

const DC_GATE: &str = "data-capability-conventions";
const CAP_GATE: &str = "capability-conventions";
const TASK: &str = "feat/next-turn";

#[test]
fn a_new_data_capability_in_the_house_shape_passes_and_an_old_one_edited_too() {
    let Some(repo) = repo("dc-clean") else {
        return;
    };
    let mut files = clean_dc("next-turn", "table.nextTurn");
    // The older unit edited: its leftover `names()` is not this change's.
    files.push((
        OLD_DC[2].0.to_string(),
        format!("{}// a comment\n", OLD_DC[2].1),
    ));
    let (ok, said) = change(&repo, DC_GATE, TASK, &files);
    assert!(ok, "{said}");
    assert!(said.contains("2 unit(s) judged"), "{said}");
}

#[test]
fn a_new_data_capability_copied_from_an_old_neighbour_names_each_drift() {
    let Some(repo) = repo("dc-drift") else {
        return;
    };
    let dir = "crates/table/data-capabilities/next-turn";
    let files = owned(&[
        (
            &format!("{dir}/data-capability.json"),
            r#"{"dataCapability":"table.nextTurn","ports":["llm"]}"#,
        ),
        (
            &format!("{dir}/Cargo.toml"),
            "[package]\nname = \"next-turn\"\n\n[dependencies]\nserde_json = \"1\"\n\n[dev-dependencies]\nports-llm = { path = \"x\" }\n",
        ),
        (
            &format!("{dir}/src/lib.rs"),
            "pub const DATA_CAPABILITY: &str = \"table.nextTur\";\npub const MANIFEST: &str = include_str!(\"../data-capability.json\");\nfn at() { let now = std::time::SystemTime::now(); }\nasync fn read(cx: &Context<'_>) { cx.rows.unwrap().rows(Q, &[]).await; }\n",
        ),
        (
            &format!("{dir}/tests/engine.rs"),
            "// no Scratch here: the word in a comment runs no SQL\nfn manifest() -> &'static str { next_turn::MANIFEST }\n",
        ),
    ]);
    let (ok, said) = change(&repo, DC_GATE, TASK, &files);
    assert!(!ok, "{said}");
    for rule in [
        "clock: line 3",
        "id-constant:",
        "ports:",
        "manifest-planted:",
        "sql-tested:",
        "legacy:",
    ] {
        assert!(said.contains(rule), "{rule} in {said}");
    }
}

#[test]
fn an_old_data_capability_that_starts_reading_the_clock_is_refused() {
    let Some(repo) = repo("dc-clock") else {
        return;
    };
    let files = vec![(
        OLD_DC[2].0.to_string(),
        format!("{}fn at() -> u64 {{ chrono::Utc::now() }}\n", OLD_DC[2].1),
    )];
    let (ok, said) = change(&repo, DC_GATE, TASK, &files);
    assert!(!ok, "{said}");
    assert!(said.contains("clock:"), "{said}");
    assert!(!said.contains("legacy:"), "the past is not judged: {said}");
}

#[test]
fn a_new_capability_in_the_house_shape_passes_and_an_old_one_edited_too() {
    let Some(repo) = repo("cap-clean") else {
        return;
    };
    let mut files = clean_cap("list-tokens", "table.listTokens");
    files.push((
        OLD_CAP[1].0.to_string(),
        format!("{}// a comment\n", OLD_CAP[1].1),
    ));
    let (ok, said) = change(&repo, CAP_GATE, TASK, &files);
    assert!(ok, "{said}");
    assert!(said.contains("2 unit(s) judged"), "{said}");
}

#[test]
fn a_new_capability_copied_from_an_old_neighbour_names_each_drift() {
    let Some(repo) = repo("cap-drift") else {
        return;
    };
    let dir = "crates/table/capabilities/list-tokens";
    let files = owned(&[
        (
            &format!("{dir}/capability.json"),
            r#"{"capability":"table.listTokens","input":{"combat":"string","view":"string"},"reads":["queries/listTokens.graphql"]}"#,
        ),
        (
            &format!("{dir}/src/lib.rs"),
            "pub const SQL: &str = \"SELECT t.id AS id FROM token t WHERE t.combat_id = $2; SELECT 1\";\npub const VARIABLES: &[&str] = &[\"combat\"];\npub fn answer(rows: &Value, variables: &BTreeMap<String, Value>) -> Value {\n    let wanted = variables.get(\"combat\");\n    rows.clone()\n}\n",
        ),
        (
            "crates/table/queries/listTokens.graphql",
            "# fields: token.id\nquery ListTokens($combat: ID!, $view: String) {\n  tokens(combat: $combat) {\n    id\n    hitPoints\n  }\n}\n",
        ),
    ]);
    let (ok, said) = change(&repo, CAP_GATE, TASK, &files);
    assert!(!ok, "{said}");
    for rule in [
        "query-file-named:",
        "no-rust-refilter: answer() reads \"combat\"",
        "one-select:",
        "binds: SQL binds [$2]",
        "binds: the query's variables [combat, view]",
        "text-compare:",
        "parity: selects `hitPoints`",
    ] {
        assert!(said.contains(rule), "{rule} in {said}");
    }
    assert!(!said.contains("variables-bound"), "{said}");
}

#[test]
fn an_answer_mapping_its_rows_inline_reads_no_variable_and_one_indexing_them_does() {
    let Some(repo) = repo("cap-inline") else {
        return;
    };
    let dir = "crates/table/capabilities/get-token";
    let lib = |body: &str| {
        format!(
            "pub const SQL: &str = r#\"SELECT t.id::text AS id FROM token t WHERE t.id::text = $1\"#;\npub const VARIABLES: &[&str] = &[\"id\"];\npub fn answer(rows: &Value, variables: &BTreeMap<String, Value>) -> Value {{\n    let first = &rows[0];\n    // variables[\"id\"] was filtered by the SQL\n    {body}\n}}\n"
        )
    };
    let mut files = owned(&[
        (
            &format!("{dir}/capability.json"),
            r#"{"capability":"table.getToken","input":{"id":"string"},"reads":["queries/get-token.graphql"]}"#,
        ),
        (
            "crates/table/queries/get-token.graphql",
            "# fields: token.id\nquery GetToken($id: ID!) {\n  token(id: $id) {\n    id\n  }\n}\n",
        ),
    ]);
    files.push((
        format!("{dir}/src/lib.rs"),
        lib("json!({ \"id\": first[\"id\"] })"),
    ));
    let (ok, said) = change(&repo, CAP_GATE, TASK, &files);
    assert!(ok, "an output key is no read: {said}");
    let indexed = vec![(
        format!("{dir}/src/lib.rs"),
        lib("let _ = variables[\"id\"]; json!({ \"id\": first[\"id\"] })"),
    )];
    let (ok, said) = change(&repo, CAP_GATE, TASK, &indexed);
    assert!(!ok, "{said}");
    assert!(
        said.contains("no-rust-refilter: answer() reads \"id\""),
        "{said}"
    );
}

#[test]
fn a_skeleton_s_hole_left_in_a_unit_or_its_query_file_is_refused_added_or_changed() {
    let Some(repo) = repo("holes") else {
        return;
    };
    let mut files = clean_cap("list-tokens", "table.listTokens");
    for (path, text) in &mut files {
        if path.ends_with(".graphql") {
            *text = text.replace("# fields:", "# fields: SCAFFOLD: the fields,");
        }
        if path.ends_with("capability.json") {
            *text = text.replace(
                r#""reads""#,
                r#""description":"SCAFFOLD: what it answers","reads""#,
            );
        }
    }
    let mut dc = clean_dc("next-turn", "table.nextTurn");
    dc.push((
        "crates/table/data-capabilities/next-turn/Cargo.toml".to_string(),
        "[package]\nname = \"next-turn\"\ndescription = \"SCAFFOLD: one line\"\n\n[dependencies]\nharness-dataguard = { workspace = true }\n".to_string(),
    ));
    // The older Capability edited, a hole pasted into its lib.
    files.push((
        OLD_CAP[1].0.to_string(),
        format!("//! SCAFFOLD: what it answers\n{}", OLD_CAP[1].1),
    ));
    let (ok, said) = change(&repo, CAP_GATE, TASK, &files);
    assert!(!ok, "{said}");
    for hole in [
        "crates/table/queries/list-tokens.graphql::holes: line(s) 1",
        "capabilities/list-tokens/capability.json::holes: line(s) 1",
        "capabilities/list-combats/src/lib.rs::holes: line(s) 1",
    ] {
        assert!(said.contains(hole), "{hole} in {said}");
    }
    let (ok, said) = change(&repo, DC_GATE, TASK, &dc);
    assert!(!ok, "{said}");
    assert!(
        said.contains("data-capabilities/next-turn/Cargo.toml::holes: line(s) 3"),
        "{said}"
    );
}

#[test]
fn an_unbound_capability_and_a_query_file_that_drifts_from_its_sql_are_refused() {
    let Some(repo) = repo("cap-unbound") else {
        return;
    };
    let dir = "crates/table/capabilities/list-all";
    let mut files = owned(&[
        (
            &format!("{dir}/capability.json"),
            r#"{"capability":"table.listAll","input":{"campaign":"string"},"reads":["queries/list-all.graphql"]}"#,
        ),
        (
            &format!("{dir}/src/lib.rs"),
            "pub const SQL: &str = r#\"WITH c AS (SELECT id FROM combat) SELECT id FROM c\"#;\npub const VARIABLES: &[&str] = &[];\npub fn answer(rows: &Value, _variables: &BTreeMap<String, Value>) -> Value { rows.clone() }\n",
        ),
        (
            "crates/table/queries/list-all.graphql",
            "# fields: combat.id\nquery ListAll {\n  combats {\n    id\n  }\n}\n",
        ),
    ]);
    // The old Capability's query file now selects what its SQL does not.
    files.push((
        OLD_CAP[2].0.to_string(),
        OLD_CAP[2]
            .1
            .replace("    round\n", "    round\n    startedAt\n"),
    ));
    let (ok, said) = change(&repo, CAP_GATE, TASK, &files);
    assert!(!ok, "{said}");
    assert!(
        said.contains("variables-bound: VARIABLES is empty while the input is {campaign}"),
        "{said}"
    );
    assert!(said.contains("parity: selects `startedAt`"), "{said}");
    assert!(
        !said.contains("no-rust-refilter"),
        "the old unit's past: {said}"
    );
    assert!(
        !said.contains("query-file-named"),
        "the old unit's past: {said}"
    );
}

#[test]
fn a_milestone_s_pull_request_a_push_and_a_hostless_unit_are_not_judged() {
    let Some(repo) = repo("skips") else {
        return;
    };
    let dir = "crates/table/capabilities/in-the-plugin";
    let files = owned(&[(
        &format!("{dir}/capability.json"),
        r#"{"capability":"table.inThePlugin","host":"hostless","reads":["x.graphql"]}"#,
    )]);
    let (ok, said) = change(&repo, CAP_GATE, TASK, &files);
    assert!(ok, "a hostless unit runs no SQL: {said}");
    let drift = owned(&[(OLD_DC[2].0, "fn at() { std::time::Instant::now(); }\n")]);
    let (ok, said) = change(&repo, DC_GATE, "milestone/7-combat", &drift);
    assert!(ok, "{said}");
    assert!(said.contains("a milestone's pull request"), "{said}");
    let out = Command::new("bash")
        .arg("-c")
        .arg(script(DC_GATE))
        .env("BASE", "")
        .env("HEAD_REF", "")
        .current_dir(&repo.root)
        .output()
        .expect("bash runs");
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("no base to compare with"));
    let nothing = Command::new("node")
        .args(["scripts/unit-conventions-gate.mjs", "units", &repo.base])
        .current_dir(&repo.root)
        .output()
        .expect("node runs");
    assert_eq!(
        nothing.status.code(),
        Some(2),
        "an unknown kind is a usage error"
    );
}
