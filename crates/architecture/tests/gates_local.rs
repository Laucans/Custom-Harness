//! `scripts/gates-local.mjs`, the gates a session runs before its pull
//! request, against node and git on repositories of their own.
//!
//! The sync guard first: the script reads every step of the harness's own
//! gates.yml, and knows every expression it uses — a gate written in a shape
//! it cannot read fails here, in the harness's CI, not in a session. Then
//! its mechanics on small workflows: the work judged is the uncommitted one,
//! the files a sparse checkout hides included; a gate red on the base with
//! the same errors is `base-red`, one that adds an error is not; jobs run in
//! parallel and a job's steps in order; a step, a job key or a job env it
//! cannot read is said and exits 2; the gates read are the merged tree's;
//! `--failed` reruns what was left red, the closure check and a conflict
//! included; a job's Postgres gates are skipped without docker; the closure
//! check is the harness binary's, skipped without one, exit 2 when it
//! cannot judge. Skipped, loudly, without a node 22 or later — red in CI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ENGINE: &str = include_str!("../assets/gates-local.mjs");
const ENTRY: &str = include_str!("../assets/gates-local.sh");
const GATES: &str = include_str!("../assets/gates.yml");

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn git(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Whether a node 22 or later answers; red when `CI` is set.
fn node_ready(test: &str) -> bool {
    let major = Command::new("node")
        .args(["-p", "process.versions.node.split('.')[0]"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<u32>()
                .ok()
        })
        .unwrap_or(0);
    if major >= 22 {
        return true;
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "gates_local {test}: no node 22 or later on PATH, and CI is set"
    );
    eprintln!("gates_local {test}: no node 22 or later on PATH — skipped");
    false
}

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos()
}

/// A throwaway folder with `engine.mjs` in it, for the runs that read no
/// repository.
fn bare(case: &str) -> Dir {
    let root = std::env::temp_dir().join(format!(
        "gates-local-{case}-{}-{}",
        std::process::id(),
        stamp()
    ));
    fs::create_dir_all(&root).expect("root");
    write(&root, "engine.mjs", ENGINE);
    Dir(root)
}

fn node(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new("node");
    command
        .arg(root.join("engine.mjs"))
        .args(args)
        .current_dir(root)
        .env_remove("HARNESS_BIN")
        .env_remove("HARNESS_FACTS")
        .env_remove("CARGO_TARGET_DIR");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("node runs")
}

fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The step ids of a workflow, every part included, in order.
fn step_ids(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("- id: "))
        .map(|id| id.trim().to_string())
        .collect()
}

#[test]
fn every_step_of_the_harness_gates_is_read_and_none_is_unreadable() {
    if !node_ready("sync") {
        return;
    }
    let dir = bare("sync");
    write(&dir.0, "gates.yml", GATES);
    let out = node(
        &dir.0,
        &["--list", "--json", "--workflow", "gates.yml"],
        &[],
    );
    let text = said(&out);
    assert!(out.status.success(), "{text}");
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(listed["error"].is_null(), "{text}");
    let steps = listed["steps"].as_array().expect("steps");
    let ids: Vec<String> = steps
        .iter()
        .filter(|step| step["kind"] == "gate")
        .map(|step| step["id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(ids, step_ids(GATES), "every gate step, in order: {text}");
    assert!(
        steps.iter().all(|step| step["kind"] != "unreadable"),
        "{text}"
    );
    // A job-level key it does not run (`needs:`, `if:`) would leave the kind
    // `gate` and say it only here: none may.
    assert!(steps.iter().all(|step| step["why"].is_null()), "{text}");
    // `rules-hold` is listed, off unless asked: it starts the product.
    assert!(
        steps
            .iter()
            .filter(|step| step["job"] == "rules-hold")
            .all(|step| step["off"] == "--rules"),
        "{text}"
    );
}

/// The verdict of one expression under the local pull request's context.
fn eval(root: &Path, expression: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["--eval", expression];
    args.extend_from_slice(extra);
    let out = node(root, &args, &[]);
    assert!(out.status.success(), "{}", said(&out));
    serde_json::from_slice(&out.stdout).expect("json")
}

#[test]
fn every_expression_of_the_gates_has_its_value_under_the_local_pull_request() {
    if !node_ready("expressions") {
        return;
    }
    let dir = bare("expressions");
    let root = &dir.0;
    let mut expressions: Vec<String> = Vec::new();
    for line in GATES.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("${{") {
            let Some(end) = rest[at..].find("}}") else {
                break;
            };
            let expression = &rest[at..at + end + 2];
            // The one `${{ }}` of the file sits in a comment that warns
            // against it.
            if expression != "${{ }}" && !expressions.iter().any(|e| e == expression) {
                expressions.push(expression.to_string());
            }
            rest = &rest[at + end + 2..];
        }
        if let Some(bare_if) = line.trim().strip_prefix("if: ")
            && !bare_if.starts_with("${{")
            && !expressions.iter().any(|e| e == bare_if)
        {
            expressions.push(bare_if.to_string());
        }
    }
    assert!(expressions.len() >= 15, "{expressions:?}");
    for expression in &expressions {
        let verdict = eval(root, expression, &["--number", "7"]);
        assert!(
            verdict["unreadable"].is_null(),
            "{expression} is not in the table: {verdict}"
        );
    }

    let rules = "${{ !cancelled() && github.event_name == 'pull_request' && !contains(github.event.pull_request.labels.*.name, 'harness:rules') }}";
    assert_eq!(eval(root, rules, &[])["value"], true);
    assert_eq!(
        eval(root, rules, &["--label", "harness:rules"])["value"],
        false
    );
    let strict = "${{ github.event_name == 'pull_request' && github.base_ref == 'main_agent' && !contains(github.event.pull_request.labels.*.name, 'harness:rules') && '--strict' || '' }}";
    assert_eq!(eval(root, strict, &[])["text"], "--strict");
    assert_eq!(
        eval(root, strict, &["--label", "harness:rules"])["text"],
        ""
    );
    assert_eq!(eval(root, strict, &["--base", "milestone/3-x"])["text"], "");
    let number = "${{ github.event.pull_request.number }}";
    assert_eq!(eval(root, number, &["--number", "7"])["text"], "7");
    assert!(eval(root, number, &[])["skip"].is_string());
    assert!(eval(root, "${{ github.token }}", &[])["skip"].is_string());
    assert_eq!(eval(root, "failure()", &[])["value"], false);
    assert_eq!(
        eval(
            root,
            "${{ github.base_ref }}",
            &["--base", "origin/milestone/2-y"]
        )["text"],
        "milestone/2-y"
    );
    assert!(eval(root, "${{ github.event.issue.title }}", &[])["unreadable"].is_string());
}

/// A product with a bare `origin`: `files` committed on `main` and pushed,
/// then a branch `feat/x` checked out, the engine under `scripts/`.
fn product(case: &str, workflow: &str, files: &[(&str, &str)]) -> (Dir, PathBuf) {
    let dir = Dir(std::env::temp_dir().join(format!(
        "gates-local-{case}-{}-{}",
        std::process::id(),
        stamp()
    )));
    let origin = dir.0.join("origin.git");
    let root = dir.0.join("work");
    fs::create_dir_all(&origin).expect("origin");
    fs::create_dir_all(&root).expect("work");
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(&root, &["init", "-q", "-b", "main"]);
    write(&root, ".github/workflows/gates.yml", workflow);
    write(&root, "scripts/gates-local.mjs", ENGINE);
    write(&root, "scripts/gates-local.sh", ENTRY);
    for (path, text) in files {
        write(&root, path, text);
    }
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(
        &root,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    git(&root, &["push", "-q", "origin", "main"]);
    git(&root, &["checkout", "-q", "-b", "feat/x"]);
    (dir, root)
}

fn gates(root: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut command = Command::new("bash");
    command
        .arg("scripts/gates-local.sh")
        .args(["--no-gh", "--base", "main"])
        .args(args)
        .current_dir(root)
        .env_remove("HARNESS_BIN")
        .env_remove("HARNESS_FACTS")
        .env_remove("CARGO_TARGET_DIR")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t");
    for (key, value) in env {
        command.env(key, value);
    }
    let out = command.output().expect("bash runs");
    (out.status.code().unwrap_or(-1), said(&out))
}

/// One job, one gate: every `status*.txt` says `ok`, an error per file
/// that does not; a report step and the checkout around it, as in CI.
const STATUS: &str = r#"name: gates
on:
  pull_request:
    branches: [main]
jobs:
  checks:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      # Every status file says ok.
      - id: says-ok
        name: says-ok
        if: ${{ !cancelled() }}
        shell: bash
        run: |
          set -euo pipefail
          failed=0
          for f in status*.txt; do
            grep -q '^ok$' "$f" || { echo "::error file=$f::not ok"; failed=1; }
          done
          exit $failed
      - id: hidden-present
        name: hidden-present
        if: ${{ !cancelled() }}
        shell: bash
        run: test -f deep/hidden.txt
      - name: Failing gates
        if: failure()
        shell: bash
        env:
          STEPS: ${{ toJSON(steps) }}
        run: echo "$STEPS"
"#;

fn status_product(case: &str) -> (Dir, PathBuf) {
    product(
        case,
        STATUS,
        &[("status1.txt", "ok\n"), ("deep/hidden.txt", "here\n")],
    )
}

#[test]
fn the_work_judged_is_the_uncommitted_one_and_its_report_is_written() {
    if !node_ready("uncommitted") {
        return;
    }
    let (_dir, root) = status_product("uncommitted");
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("ok · says-ok"), "{text}");
    assert!(text.contains("ok · hidden-present"), "{text}");
    assert!(
        text.contains("skip · closure · harness sessions only"),
        "{text}"
    );
    assert!(
        !text.contains("Failing gates"),
        "a report step is no gate: {text}"
    );

    // A break left in the working tree, never committed: the snapshot carries it.
    write(&root, "status1.txt", "broken\n");
    write(&root, "notes/scratch.txt", "untracked\n");
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("FAIL · says-ok"), "{text}");
    assert!(text.contains("::error file=status1.txt::not ok"), "{text}");
    assert!(text.contains("full log: "), "{text}");
    assert!(
        text.contains("1 untracked file(s) included: notes/scratch.txt"),
        "{text}"
    );
    let report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(".git/gates-local/report.json")).expect("a report"),
    )
    .expect("json");
    assert_eq!(report["base"], "main");
    assert_eq!(report["head"], git(&root, &["rev-parse", "HEAD"]));
    assert_eq!(report["counts"]["FAIL"], 1);
    let says_ok = report["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|g| g["id"] == "says-ok")
        .expect("says-ok reported");
    assert_eq!(says_ok["outcome"], "FAIL");
    assert_eq!(says_ok["job"], "checks");
    // The work stays as it was: nothing staged, nothing committed.
    assert_eq!(git(&root, &["diff", "--cached", "--name-only"]), "");
    let porcelain = git(&root, &["status", "--porcelain"]);
    assert!(
        porcelain.contains("status1.txt") && porcelain.contains("?? notes/"),
        "{porcelain}"
    );

    // `--failed` reruns the red gate alone, and its report says it judged
    // a subset.
    write(&root, "status1.txt", "ok\n");
    let (code, text) = gates(&root, &["--failed"], &[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("ok · says-ok"), "{text}");
    assert!(!text.contains("hidden-present"), "{text}");
    assert!(text.contains("gates-local: job checks running"), "{text}");
    assert!(text.contains("partial (--failed)"), "{text}");
    let report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(".git/gates-local/report.json")).expect("a report"),
    )
    .expect("json");
    assert_eq!(report["partial"], "--failed");

    // Nothing left red: nothing reruns.
    let (code, text) = gates(&root, &["--failed"], &[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("nothing to rerun"), "{text}");
}

#[test]
fn a_gate_red_on_the_base_with_the_same_errors_is_not_the_change_s() {
    if !node_ready("base-red") {
        return;
    }
    let (_dir, root) = product(
        "base-red",
        STATUS,
        &[("status1.txt", "broken\n"), ("deep/hidden.txt", "here\n")],
    );
    write(&root, "other.txt", "a change of its own\n");
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("base-red · says-ok"), "{text}");
    assert!(text.contains("rerun on the base"), "{text}");

    // A second error on the same gate is this change's: red.
    write(&root, "status2.txt", "broken\n");
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("FAIL · says-ok"), "{text}");
}

#[test]
fn a_change_that_conflicts_with_the_base_is_a_finding_of_its_own() {
    if !node_ready("conflict") {
        return;
    }
    let (_dir, root) = status_product("conflict");
    write(&root, "status1.txt", "ok\nmine\n");
    git(&root, &["commit", "-qam", "mine"]);
    // The base moves on under the branch, on the same line.
    git(&root, &["checkout", "-q", "main"]);
    write(&root, "status1.txt", "ok\ntheirs\n");
    git(&root, &["commit", "-qam", "theirs"]);
    git(&root, &["push", "-q", "origin", "main"]);
    git(&root, &["checkout", "-q", "feat/x"]);
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("FAIL · merge"), "{text}");
    assert!(text.contains("status1.txt"), "{text}");
    // The report holds no gate after a conflict: `--failed` redoes the
    // merge, never a green "nothing to rerun".
    let (code, text) = gates(&root, &["--failed"], &[]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("FAIL · merge"), "{text}");
}

#[test]
fn the_gates_read_are_the_merged_tree_s_a_step_the_base_added_included() {
    if !node_ready("base-step") {
        return;
    }
    let (_dir, root) = status_product("base-step");
    git(&root, &["checkout", "-q", "main"]);
    let added = STATUS.replace(
        "      - name: Failing gates",
        "      - id: base-added\n        name: base-added\n        if: ${{ !cancelled() }}\n        run: test -f status1.txt\n      - name: Failing gates",
    );
    write(&root, ".github/workflows/gates.yml", &added);
    git(&root, &["commit", "-qam", "a gate on the base"]);
    git(&root, &["push", "-q", "origin", "main"]);
    git(&root, &["checkout", "-q", "feat/x"]);
    assert!(
        !fs::read_to_string(root.join(".github/workflows/gates.yml"))
            .expect("gates")
            .contains("base-added"),
        "the branch never saw it"
    );
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("ok · base-added"), "{text}");
}

#[test]
fn the_merge_with_the_base_s_tip_is_what_the_gates_read() {
    if !node_ready("merge") {
        return;
    }
    let (_dir, root) = status_product("merge");
    // The base breaks a file the branch never touched: the PR's merge holds
    // the break, as the CI's `refs/pull/<n>/merge` would.
    git(&root, &["checkout", "-q", "main"]);
    write(&root, "status3.txt", "broken\n");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "theirs"]);
    git(&root, &["push", "-q", "origin", "main"]);
    git(&root, &["checkout", "-q", "feat/x"]);
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("base-red · says-ok"), "{text}");
}

#[test]
fn a_sparse_checkout_hides_nothing_from_the_gates_and_stays_as_it_was() {
    if !node_ready("sparse") {
        return;
    }
    let (_dir, root) = status_product("sparse");
    git(
        &root,
        &[
            "sparse-checkout",
            "set",
            "--no-cone",
            "/status1.txt",
            "/scripts/",
            "/.github/",
        ],
    );
    assert!(!root.join("deep/hidden.txt").exists(), "hidden by the cone");
    let before = git(&root, &["sparse-checkout", "list"]);
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("ok · hidden-present"), "{text}");
    let judged = root.join(".git/gates-local/head-checks/deep/hidden.txt");
    assert!(judged.exists(), "the hidden file is in the judged tree");
    assert_eq!(git(&root, &["sparse-checkout", "list"]), before);
    assert!(!root.join("deep/hidden.txt").exists(), "still hidden here");
    assert_eq!(git(&root, &["status", "--porcelain"]), "");
}

#[test]
fn jobs_run_side_by_side_and_a_job_s_steps_in_their_order() {
    if !node_ready("parallel") {
        return;
    }
    let job = |name: &str| {
        format!(
            "  {name}:\n    runs-on: ubuntu-latest\n    steps:\n      - id: {name}-first\n        name: {name}-first\n        if: ${{{{ !cancelled() }}}}\n        run: sleep 2 && touch first.marker\n      - id: {name}-second\n        name: {name}-second\n        if: ${{{{ !cancelled() }}}}\n        run: test -f first.marker\n"
        )
    };
    let workflow = format!("name: gates\njobs:\n{}{}", job("one"), job("two"));
    let (_dir, root) = product("parallel", &workflow, &[]);
    let started = Instant::now();
    let (code, text) = gates(&root, &[], &[]);
    let took = started.elapsed();
    assert_eq!(code, 0, "{text}");
    for gate in ["one-first", "one-second", "two-first", "two-second"] {
        assert!(text.contains(&format!("ok · {gate}")), "{text}");
    }
    assert!(
        took < Duration::from_millis(3_900),
        "two jobs of 2 s took {took:?}: {text}"
    );
}

#[test]
fn a_step_it_cannot_read_is_said_and_exits_two() {
    if !node_ready("unreadable") {
        return;
    }
    let workflow = "name: gates\njobs:\n  checks:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: someone/else@v1\n      - id: fine\n        name: fine\n        run: 'true'\n      - id: strange\n        name: strange\n        if: ${{ github.event.issue.title == 'x' }}\n        run: 'true'\n";
    let (_dir, root) = product("unreadable", workflow, &[]);
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("unreadable · step 1 · uses someone/else@v1"),
        "{text}"
    );
    assert!(text.contains("unreadable · strange · if:"), "{text}");
    assert!(text.contains("ok · fine"), "{text}");
}

#[test]
fn a_postgres_gate_without_docker_is_skipped_and_said() {
    if !node_ready("no-docker") {
        return;
    }
    let workflow = "name: gates\njobs:\n  shell:\n    runs-on: ubuntu-latest\n    services:\n      postgres:\n        image: postgres:16\n        env:\n          POSTGRES_PASSWORD: gates\n        ports: [\"5432:5432\"]\n    steps:\n      - id: needs-db\n        name: needs-db\n        if: ${{ !cancelled() }}\n        run: psql -h localhost -U postgres -c 'select 1'\n      - id: no-db\n        name: no-db\n        if: ${{ !cancelled() }}\n        run: 'true'\n";
    let (_dir, root) = product("no-docker", workflow, &[]);
    let (code, text) = gates(&root, &[], &[("GATES_LOCAL_DOCKER", "/nonexistent/docker")]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("skip · needs-db · no docker"), "{text}");
    assert!(text.contains("ok · no-db"), "{text}");
}

#[test]
fn the_closure_check_is_the_harness_binary_s_with_the_session_s_facts() {
    if !node_ready("closure") {
        return;
    }
    let (dir, root) = status_product("closure");
    let bin = dir.0.join("harness");
    let seen = dir.0.join("args.txt");
    write(
        &dir.0,
        "harness",
        &format!(
            "#!/usr/bin/env bash\necho \"$@\" > '{}'\necho 'crates/x/y.rs: changed by this round, outside the closure'\nexit 1\n",
            seen.display()
        ),
    );
    Command::new("chmod")
        .args(["+x", &bin.to_string_lossy()])
        .status()
        .expect("chmod");
    let facts = dir.0.join("facts.json");
    write(
        &dir.0,
        "facts.json",
        r#"{"task":7,"base":"origin/main","branch":"feat/x","labels":[]}"#,
    );
    let (code, text) = gates(
        &root,
        &[],
        &[
            ("HARNESS_BIN", &bin.to_string_lossy()),
            ("HARNESS_FACTS", &facts.to_string_lossy()),
        ],
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("FAIL · closure"), "{text}");
    assert!(text.contains("outside the closure"), "{text}");
    let args = fs::read_to_string(&seen).expect("the binary ran");
    assert!(args.starts_with("architecture-check --facts "), "{args}");
    assert!(args.contains(" --tree "), "{args}");
    assert!(args.contains(" --from "), "{args}");
    assert!(args.contains(" --to "), "{args}");
}

/// A fake harness binary under `dir` that answers `code`, saying `said`.
fn harness_bin(dir: &Path, code: i32, said: &str) -> PathBuf {
    let bin = dir.join("harness");
    fs::write(
        &bin,
        format!("#!/usr/bin/env bash\necho '{said}'\nexit {code}\n"),
    )
    .expect("write");
    Command::new("chmod")
        .args(["+x", &bin.to_string_lossy()])
        .status()
        .expect("chmod");
    bin
}

#[test]
fn failed_reruns_a_red_closure_check_with_every_gate_green() {
    if !node_ready("closure-failed") {
        return;
    }
    let (dir, root) = status_product("closure-failed");
    write(
        &dir.0,
        "facts.json",
        r#"{"task":7,"base":"origin/main","branch":"feat/x","labels":[]}"#,
    );
    let facts = dir.0.join("facts.json").to_string_lossy().to_string();
    let red = harness_bin(&dir.0, 1, "crates/x/y.rs: outside the closure");
    let red = red.to_string_lossy().to_string();
    let env = |bin: &str| {
        vec![
            ("HARNESS_BIN", bin.to_string()),
            ("HARNESS_FACTS", facts.clone()),
        ]
    };
    let run = |args: &[&str], bin: &str| {
        let owned = env(bin);
        let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        gates(&root, args, &pairs)
    };
    let (code, text) = run(&[], &red);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("ok · says-ok"), "{text}");
    assert!(text.contains("FAIL · closure"), "{text}");
    // Every gate green, the closure red: `--failed` reruns the closure.
    let (code, text) = run(&["--failed"], &red);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("FAIL · closure"), "{text}");
    assert!(!text.contains("nothing to rerun"), "{text}");
    let green = harness_bin(&dir.0, 0, "closure holds")
        .to_string_lossy()
        .to_string();
    let (code, text) = run(&["--failed"], &green);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("ok · closure"), "{text}");
}

#[test]
fn a_closure_check_that_cannot_judge_exits_two() {
    if !node_ready("closure-error") {
        return;
    }
    let (dir, root) = status_product("closure-error");
    write(
        &dir.0,
        "facts.json",
        r#"{"task":7,"base":"origin/main","branch":"feat/x","labels":[]}"#,
    );
    let facts = dir.0.join("facts.json");
    let bin = harness_bin(&dir.0, 2, "cannot read the task");
    let (code, text) = gates(
        &root,
        &[],
        &[
            ("HARNESS_BIN", &bin.to_string_lossy()),
            ("HARNESS_FACTS", &facts.to_string_lossy()),
        ],
    );
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("error · closure"), "{text}");
    let report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(".git/gates-local/report.json")).expect("a report"),
    )
    .expect("json");
    assert_eq!(report["counts"]["closure-error"], 1);
}

#[test]
fn a_job_or_workflow_key_it_does_not_run_makes_the_job_unreadable() {
    if !node_ready("job-keys") {
        return;
    }
    let job = |name: &str, extra: &str| {
        format!(
            "  {name}:\n    runs-on: ubuntu-latest\n{extra}    steps:\n      - id: {name}-gate\n        name: {name}-gate\n        run: 'true'\n"
        )
    };
    let needs = format!(
        "name: gates\njobs:\n{}{}",
        job("first", ""),
        job("second", "    needs: first\n")
    );
    let dir = bare("job-keys");
    write(&dir.0, "needs.yml", &needs);
    let out = node(
        &dir.0,
        &["--list", "--json", "--workflow", "needs.yml"],
        &[],
    );
    assert_eq!(out.status.code(), Some(2), "{}", said(&out));
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let second = listed["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["id"] == "second-gate")
        .expect("listed")
        .clone();
    assert_eq!(second["kind"], "unreadable", "{listed}");
    assert!(
        second["why"].as_str().unwrap_or_default().contains("needs"),
        "{listed}"
    );

    let env = format!("name: gates\nenv:\n  X: y\njobs:\n{}", job("only", ""));
    write(&dir.0, "env.yml", &env);
    let out = node(&dir.0, &["--list", "--workflow", "env.yml"], &[]);
    assert_eq!(out.status.code(), Some(2), "{}", said(&out));
    assert!(
        said(&out).contains("workflow key(s) not read: env"),
        "{}",
        said(&out)
    );

    let (_dir, root) = product("job-keys-run", &needs, &[]);
    let (code, text) = gates(&root, &[], &[]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("unreadable · second-gate"), "{text}");
    assert!(text.contains("ok · first-gate"), "{text}");
}

#[test]
fn a_job_env_it_cannot_give_stops_its_gates_and_continue_on_error_and_timeouts_hold() {
    if !node_ready("step-keys") {
        return;
    }
    let workflow = "name: gates\njobs:\n  strange:\n    runs-on: ubuntu-latest\n    env:\n      TITLE: ${{ github.event.issue.title }}\n    steps:\n      - id: strange-gate\n        name: strange-gate\n        run: test -n \"$TITLE\"\n  tolerant:\n    runs-on: ubuntu-latest\n    steps:\n      - id: may-fail\n        name: may-fail\n        continue-on-error: true\n        run: exit 1\n      - id: after\n        name: after\n        run: 'true'\n      - id: slow\n        name: slow\n        if: ${{ !cancelled() }}\n        timeout-minutes: 0.02\n        run: sleep 20\n";
    let (_dir, root) = product("step-keys", workflow, &[]);
    let started = Instant::now();
    let (code, text) = gates(&root, &[], &[]);
    assert!(started.elapsed() < Duration::from_secs(15), "{text}");
    assert_eq!(code, 2, "the job env is unreadable: {text}");
    assert!(
        text.contains("unreadable · strange-gate · job env TITLE"),
        "{text}"
    );
    assert!(text.contains("tolerated · may-fail"), "{text}");
    // A tolerated failure leaves the job green: the next `success()` step runs.
    assert!(text.contains("ok · after"), "{text}");
    // Killed at its timeout, well before its 20 s: red, here and on the base,
    // which holds the same step.
    assert!(text.contains("red · slow"), "{text}");
    assert!(text.contains("base-red · slow"), "{text}");
}

#[test]
fn the_entry_refuses_an_old_node_and_names_the_fix() {
    let (_dir, root) = status_product("old-node");
    let fake = root.join("old-node");
    fs::write(&fake, "#!/usr/bin/env bash\necho 12\n").expect("write");
    Command::new("chmod")
        .args(["+x", &fake.to_string_lossy()])
        .status()
        .expect("chmod");
    let (code, text) = gates(&root, &[], &[("GATES_LOCAL_NODE", &fake.to_string_lossy())]);
    assert_eq!(code, 3, "{text}");
    assert!(text.contains("node 22 or later is needed"), "{text}");
}

#[test]
fn before_its_branch_a_session_s_head_is_the_branch_its_task_declares() {
    if !node_ready("facts") {
        return;
    }
    let (dir, root) = status_product("facts");
    git(&root, &["checkout", "-q", "main"]);
    write(
        &dir.0,
        "facts.json",
        r#"{"task":"7","base":"origin/main","branch":"feat/declared","labels":[]}"#,
    );
    let facts = dir.0.join("facts.json");
    let mut command = Command::new("bash");
    command
        .arg("scripts/gates-local.sh")
        .current_dir(&root)
        .env_remove("HARNESS_BIN")
        .env("HARNESS_FACTS", &facts)
        .env("GATES_LOCAL_DOCKER", "/nonexistent/docker");
    // No `--base`, no `--no-gh`: the facts name the base, and gh is never
    // asked on the base itself.
    let out = command.output().expect("bash runs");
    let text = said(&out);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(
        text.contains("gates-local: feat/declared -> main @"),
        "{text}"
    );
}
