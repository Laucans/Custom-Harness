//! `scripts/design-system-gate.mjs` against node, on a mock-up of its own: a
//! clean one passes, and each way of styling around the design system — a
//! colour literal, an unknown token, a literal spacing, a stylesheet link out
//! of place, a script's offset in a file that does not say it places — fails.
//! Skipped, loudly, when `node` is not on the path.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const GATE: &str = include_str!("../assets/design-system-gate.mjs");

const TOKENS: &str = ":root {\n  --color-accent: #f5b83d;\n  --space-2: 8px;\n}\n";
const STYLE: &str = "body { color: var(--color-accent); padding: var(--space-2); }\n";
const LINKS_HOME: &str = "<link rel=\"stylesheet\" href=\"../design/tokens.css\">\n\
     <link rel=\"stylesheet\" href=\"../kit/kit.css\">\n\
     <link rel=\"stylesheet\" href=\"../style.css\">\n";
const LINKS_INDEX: &str = "<link rel=\"stylesheet\" href=\"design/tokens.css\">\n\
     <link rel=\"stylesheet\" href=\"kit/kit.css\">\n\
     <link rel=\"stylesheet\" href=\"style.css\">\n";
const PLACED: &str = "<div class=\"map\"></div>\n<script>\nconst el = document.querySelector('.map');\nel.style.left = '12px';\n</script>\n";

/// A product root with a clean mock-up and the gate under `scripts/`.
fn mockup(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let root = std::env::temp_dir().join(format!("ds-gate-{name}-{}-{stamp}", std::process::id()));
    for dir in [
        "mock/design",
        "mock/kit",
        "mock/pages",
        "mock/components",
        "scripts",
    ] {
        fs::create_dir_all(root.join(dir)).expect("dir");
    }
    let write = |path: &str, text: &str| fs::write(root.join(path), text).expect("write");
    write("scripts/design-system-gate.mjs", GATE);
    write("mock/design/tokens.css", TOKENS);
    write(
        "mock/kit/kit.css",
        ".kit-button { color: var(--color-accent); }\n",
    );
    write("mock/style.css", STYLE);
    write(
        "mock/index.html",
        &format!("<!doctype html>\n<html><head>\n{LINKS_INDEX}</head><body></body></html>\n"),
    );
    write(
        "mock/pages/home.html",
        &format!(
            "<!doctype html>\n<html><head>\n{LINKS_HOME}</head><body><div data-component=\"Map\"></div></body></html>\n"
        ),
    );
    write("mock/components/Map.html", "<div class=\"map\"></div>\n");
    root
}

/// Whether the gate passed, and what it said.
fn gate(root: &Path) -> (bool, String) {
    let out = Command::new("node")
        .arg("scripts/design-system-gate.mjs")
        .current_dir(root)
        .output()
        .expect("node runs");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), said)
}

#[test]
fn the_design_system_gate_refuses_every_way_around_the_tokens() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("design_system_gate: node is not on the path, skipped");
        return;
    }
    let clean = mockup("clean");
    let (passed, said) = gate(&clean);
    let _ = fs::remove_dir_all(&clean);
    assert!(passed, "a clean mock-up passes: {said}");

    let breaks: [(&str, &str, &str, &str); 5] = [
        (
            "colour",
            "mock/style.css",
            "p { color: #ffffff; }\n",
            "colour literal",
        ),
        (
            "token",
            "mock/style.css",
            "p { color: var(--color-nope); }\n",
            "unknown design token --color-nope",
        ),
        (
            "spacing",
            "mock/style.css",
            "p { padding: 4px; }\n",
            "literal spacing",
        ),
        (
            "offset",
            "mock/components/Map.html",
            PLACED,
            "literal offset",
        ),
        (
            "link",
            "mock/pages/home.html",
            "<link rel=\"stylesheet\" href=\"../style.css\">\n",
            "link 1",
        ),
    ];
    for (name, path, text, expected) in breaks {
        let root = mockup(name);
        if path.ends_with(".css") {
            let mut style = fs::read_to_string(root.join(path)).expect("read");
            style.push_str(text);
            fs::write(root.join(path), style).expect("write");
        } else if name == "link" {
            fs::write(
                root.join(path),
                format!("<!doctype html>\n<html><head>\n{text}</head><body></body></html>\n"),
            )
            .expect("write");
        } else {
            fs::write(root.join(path), text).expect("write");
        }
        let (passed, said) = gate(&root);
        let _ = fs::remove_dir_all(&root);
        assert!(!passed, "{name}: the gate fails — {said}");
        assert!(
            said.contains(expected),
            "{name}: says {expected:?} — {said}"
        );
    }

    // A script that places an element says so, and its offsets pass.
    let placed = mockup("placed");
    fs::write(
        placed.join("mock/components/Map.html"),
        format!("<!-- design-system-gate: computed placement -->\n{PLACED}"),
    )
    .expect("write");
    let (passed, said) = gate(&placed);
    let _ = fs::remove_dir_all(&placed);
    assert!(passed, "a file marked as placing by script passes: {said}");
}
