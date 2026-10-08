//! Which `claude` the steward runs — the newest one on the machine.
//!
//! A `claude` on `PATH` is often a wrapper, and a wrapper is often stale: the
//! one found here pointed at a VS Code extension two releases behind. So the
//! steward asks every known install for its version and keeps the highest.
//! This spawns `claude --version` a few times at start-up, which is why it
//! lives with the adapters.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a Claude Code binary tends to be, beyond `PATH`.
fn candidates(home: &Path) -> Vec<PathBuf> {
    let mut found = vec![PathBuf::from("claude")];
    found.push(home.join(".local/bin/claude"));
    found.push(home.join(".claude/local/claude"));
    found.push(PathBuf::from("/opt/homebrew/bin/claude"));
    found.push(PathBuf::from("/usr/local/bin/claude"));
    for editor in [".vscode", ".vscode-insiders", ".cursor"] {
        if let Ok(entries) = std::fs::read_dir(home.join(editor).join("extensions")) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().starts_with("anthropic.claude-code-") {
                    found.push(entry.path().join("resources/native-binary/claude"));
                }
            }
        }
    }
    found
}

/// `2.1.292 (Claude Code)` → `[2, 1, 292]`; anything else is `None`.
#[must_use]
pub fn parse_version(text: &str) -> Option<Vec<u64>> {
    let first = text.split_whitespace().next()?;
    let parts: Vec<u64> = first
        .split('.')
        .map(|p| p.trim_start_matches('v').parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    (parts.len() >= 2).then_some(parts)
}

fn version_of(binary: &Path) -> Option<Vec<u64>> {
    let out = Command::new(binary).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// The newest `claude` among the known installs, and its version — or
/// `None` when none answers.
#[must_use]
pub fn newest() -> Option<(PathBuf, String)> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for candidate in candidates(&home) {
        if candidate.is_absolute() && !candidate.is_file() {
            continue;
        }
        if let Some(version) = version_of(&candidate)
            && best.as_ref().is_none_or(|(v, _)| version > *v)
        {
            best = Some((version, candidate));
        }
    }
    best.map(|(version, path)| {
        let shown = version
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(".");
        (path, shown)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_line_reads_as_numbers_that_compare_newest_last() {
        assert_eq!(
            parse_version("2.1.292 (Claude Code)"),
            Some(vec![2, 1, 292])
        );
        assert_eq!(parse_version("v2.1.9"), Some(vec![2, 1, 9]));
        assert!(
            parse_version("2.1.9") < parse_version("2.1.292"),
            "numeric, not lexical"
        );
        assert_eq!(parse_version("claude: command not found"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("2"), None);
    }

    #[test]
    fn the_candidates_start_with_path_and_scan_the_editor_extensions() {
        let dir = std::env::temp_dir().join(format!("harness-view-claude-{}", std::process::id()));
        let ext = dir.join(
            ".vscode/extensions/anthropic.claude-code-2.1.292-darwin-arm64/resources/native-binary",
        );
        std::fs::create_dir_all(&ext).expect("mkdir");
        std::fs::write(ext.join("claude"), "").expect("write");
        std::fs::create_dir_all(dir.join(".vscode/extensions/ms-python.python-1.0"))
            .expect("mkdir");
        let found = candidates(&dir);
        assert_eq!(found[0], PathBuf::from("claude"));
        assert!(found.iter().any(|p| p.ends_with(
            "anthropic.claude-code-2.1.292-darwin-arm64/resources/native-binary/claude"
        )));
        assert!(
            !found
                .iter()
                .any(|p| p.to_string_lossy().contains("ms-python"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
