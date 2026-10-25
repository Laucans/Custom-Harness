//! The `compositions-resolve` gate of `gates.yml`, as `init-repo` installs
//! it, run for real against a throwaway tree: every Micro-UI a frozen screen
//! lays out — as an id, an object naming it in `microUi`, or an object keyed
//! by it, the form the screens are written in — is the `microUi` of a
//! `micro-ui.json`; on a pull request into a milestone branch only, one the
//! plan's graph knows as a micro-ui and the mock-up holds a fragment of
//! passes too.
//!
//! Needs `bash` 4 or later (`globstar`) and `jq`; skipped, loudly, without.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::content_of;

struct Tree {
    root: PathBuf,
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

/// Whether this machine runs the gate: a bash with `globstar`, and `jq`.
fn runnable() -> bool {
    let bash = Command::new("bash")
        .args(["-c", "shopt -s globstar"])
        .output()
        .is_ok_and(|out| out.status.success());
    let jq = Command::new("jq").arg("--version").output().is_ok();
    if !(bash && jq) {
        eprintln!("compositions_gate: needs bash 4+ and jq on the path, skipped");
    }
    bash && jq
}

/// A tree holding `files`.
fn tree(case: &str, files: &[(&str, &str)]) -> Tree {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "compositions-gate-{case}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("mkdir");
    for (path, text) in files {
        write(&root, path, text);
    }
    Tree { root }
}

/// The gate's `run: |` block, as the step runs it.
fn script() -> String {
    let gates = content_of(".github/workflows/gates.yml").expect("installed");
    let step = gates
        .split("- id: compositions-resolve\n")
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

/// Runs the gate in `tree` as the event `event` into `base` (empty on a
/// push): whether it passed, and what it said.
fn run(tree: &Tree, event: &str, base: &str) -> (bool, String) {
    let out = Command::new("bash")
        .arg("-c")
        .arg(script())
        .env("EVENT", event)
        .env("BASE_REF", base)
        .current_dir(&tree.root)
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

const GRAPH: &str = r#"{"version":1,"milestones":[],"ids":{
  "OralBrief":{"kind":"micro-ui","producer":24,"state":"planned","version":1},
  "ai.listNotes":{"kind":"capability","producer":24,"state":"planned","version":1},
  "Unmocked":{"kind":"micro-ui","producer":27,"state":"planned","version":1}
}}"#;

const PAGES: &str = r#"{"version":1,"pages":[],"components":[
  {"name":"OralBrief","fragment":"components/OralBrief.html"},
  {"name":"ai.listNotes","fragment":"components/x.html"}
]}"#;

#[test]
fn a_built_micro_ui_resolves_in_every_layout_form_and_everywhere() {
    if !runnable() {
        return;
    }
    let tree = tree(
        "built",
        &[
            (
                "apps/screens/campagne.json",
                r#"{"screen":"campagne","layout":[{"CampaignOverview":{"campaign":"c"}},"SessionList",{"microUi":"ToCreateList","props":{}}]}"#,
            ),
            (
                "apps/campaign/overview/micro-ui.json",
                r#"{"microUi":"CampaignOverview"}"#,
            ),
            (
                "apps/session/list/micro-ui.json",
                r#"{"microUi":"SessionList"}"#,
            ),
            (
                "apps/session/to-create/micro-ui.json",
                r#"{"microUi":"ToCreateList"}"#,
            ),
        ],
    );
    for (event, base) in [
        ("push", ""),
        ("pull_request", "main_agent"),
        ("pull_request", "milestone/28-screens"),
    ] {
        let (passed, said) = run(&tree, event, base);
        assert!(passed, "{event} into {base:?}: {said}");
    }
}

#[test]
fn a_keyed_layout_entry_no_manifest_declares_fails_where_it_lands() {
    if !runnable() {
        return;
    }
    let tree = tree(
        "keyed",
        &[(
            "apps/screens/player-view.json",
            r#"{"screen":"player-view","layout":[{"PlayerStage":{}}]}"#,
        )],
    );
    let (passed, said) = run(&tree, "push", "");
    assert!(!passed, "{said}");
    assert!(
        said.contains("lays out PlayerStage, which no micro-ui.json declares"),
        "the keyed form is read: {said}"
    );
}

#[test]
fn a_mocked_micro_ui_passes_into_a_milestone_and_is_refused_where_the_milestone_lands() {
    if !runnable() {
        return;
    }
    let tree = tree(
        "mocked",
        &[
            (
                "apps/screens/campagne.json",
                r#"{"screen":"campagne","layout":[{"OralBrief":{"campaign":"c"}}]}"#,
            ),
            ("plan/graph.json", GRAPH),
            ("mock/pages.json", PAGES),
        ],
    );
    let (passed, said) = run(&tree, "pull_request", "milestone/28-screens");
    assert!(passed, "frozen against the mock-up: {said}");
    assert!(said.contains("OralBrief, mocked"), "{said}");
    for (event, base) in [
        ("pull_request", "main_agent"),
        ("push", ""),
        // A push to a milestone branch carries no base: strict too.
        ("push", "milestone/28-screens"),
    ] {
        let (passed, said) = run(&tree, event, base);
        assert!(!passed, "{event} into {base:?} is strict: {said}");
    }
}

#[test]
fn only_a_planned_micro_ui_with_a_fragment_is_mocked() {
    if !runnable() {
        return;
    }
    for (case, layout) in [
        // The graph knows it as a micro-ui, the mock-up has no fragment.
        ("no-fragment", "Unmocked"),
        // The mock-up has a fragment, the graph knows it as another kind.
        ("other-kind", "ai.listNotes"),
        // Neither knows it.
        ("unknown", "Nobody"),
    ] {
        let tree = tree(
            case,
            &[
                (
                    "apps/screens/s.json",
                    &format!(r#"{{"screen":"s","layout":[{{"{layout}":{{}}}}]}}"#),
                ),
                ("plan/graph.json", GRAPH),
                ("mock/pages.json", PAGES),
            ],
        );
        let (passed, said) = run(&tree, "pull_request", "milestone/28-screens");
        assert!(!passed, "{case}: {said}");
    }
    // No graph or no mock-up at all: nothing is mocked.
    let bare = tree(
        "bare",
        &[(
            "apps/screens/s.json",
            r#"{"screen":"s","layout":[{"OralBrief":{}}]}"#,
        )],
    );
    let (passed, said) = run(&bare, "pull_request", "milestone/28-screens");
    assert!(!passed, "{said}");
}
