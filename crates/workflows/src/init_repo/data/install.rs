//! What `init-repo` installs in the target repository, and what a run must
//! write of it given what is already there.
//!
//! Pure: decides from texts already read, writes nothing. The files are
//! [`harness_architecture::assets`], compiled in, so a repository brought
//! under the plant carries the same architecture as every other one, from the
//! same source.
//!
//! # Two rules
//!
//! - **Nothing already there is overwritten** unless `--force`: a human may
//!   have adapted a contract or a gate, and the harness's version is not
//!   worth more than theirs. A file that exists is reported as kept.
//! - **`CLAUDE.md` is appended to, never replaced**: the rules go in a
//!   block between two markers, after whatever the project already says
//!   about itself. `--force` rewrites the block in place and touches nothing
//!   outside it.
//! - **A marked block is versioned**: the opening marker carries the version
//!   of the architecture that wrote it. An older block — in `CLAUDE.md` or in
//!   a document the harness installs — is replaced in place by the current
//!   one without `--force`, and never added twice.
//! - **A file the product never touched follows the harness**: the
//!   manifest ([`MANIFEST`]) keeps the fingerprint of what the harness last
//!   wrote of each file. A file that still matches it is the harness's own,
//!   and a newer harness rewrites it; a file that differs was adapted by the
//!   product and is kept, reported as diverged. Without it, a fix to an
//!   installed script or contract never reached a product made before
//!   (a product kept a broken `rules-runtime.sh`, contracts B and F).

use std::collections::BTreeMap;

use harness_architecture::assets::{
    CLAUDE_RULES, CLAUDE_VERSION, FILES, MARKER_CLOSE, MARKER_PREFIX, VERSION, installed_version,
};

/// Where the product keeps the fingerprints of what the harness wrote.
pub const MANIFEST: &str = ".harness/installed.json";

/// The path of the file the rules are appended to.
pub const CLAUDE_MD: &str = "CLAUDE.md";

/// The commit that carries the install.
pub const COMMIT_MESSAGE: &str = "chore(harness): install the agent-native architecture rules";

/// What one run writes, and what it leaves alone.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InstallPlan {
    /// Files to write, with their full new content — `CLAUDE.md` included
    /// when its block is to be added or rewritten, the manifest when it
    /// changes.
    pub writes: Vec<(String, String)>,
    /// Files already there and left as they are.
    pub kept: Vec<String>,
    /// Files the product never touched, rewritten to the harness's newer
    /// version.
    pub upgraded: Vec<String>,
    /// Files that differ from both the harness's version and what it last
    /// wrote: the product's own now, kept.
    pub diverged: Vec<String>,
}

impl InstallPlan {
    /// Decides the writes from what is in the checkout. `existing` answers
    /// the content of a path, or `None` when the file is absent.
    pub fn new(existing: &dyn Fn(&str) -> Option<String>, force: bool) -> Self {
        let mut plan = Self::default();
        let before = existing(MANIFEST);
        let recorded = before.as_deref().map(parse_manifest).unwrap_or_default();
        let mut manifest: BTreeMap<String, String> = BTreeMap::new();
        for asset in &FILES {
            let current = fingerprint(asset.content);
            let write = match existing(asset.path) {
                None => true,
                Some(_) if force => true,
                Some(text) if text == asset.content => false,
                Some(text) if outdated(&text, VERSION) => true,
                Some(text) if recorded.get(asset.path) == Some(&fingerprint(&text)) => {
                    plan.upgraded.push(asset.path.to_string());
                    true
                }
                Some(_) => {
                    plan.diverged.push(asset.path.to_string());
                    if let Some(last) = recorded.get(asset.path) {
                        manifest.insert(asset.path.to_string(), last.clone());
                    }
                    plan.kept.push(asset.path.to_string());
                    continue;
                }
            };
            manifest.insert(asset.path.to_string(), current);
            if write {
                plan.writes
                    .push((asset.path.to_string(), asset.content.to_string()));
            } else {
                plan.kept.push(asset.path.to_string());
            }
        }
        match merge_claude_md(existing(CLAUDE_MD).as_deref(), force) {
            Some(text) => plan.writes.push((CLAUDE_MD.to_string(), text)),
            None => plan.kept.push(CLAUDE_MD.to_string()),
        }
        let after = render_manifest(&manifest);
        if before.as_deref() != Some(after.as_str()) {
            plan.writes.push((MANIFEST.to_string(), after));
        }
        plan
    }

    /// The commit message: an upgrade says so, an install says what it is.
    #[must_use]
    pub fn commit_message(&self) -> String {
        if self.writes.iter().all(|(path, _)| path == MANIFEST) {
            "chore(harness): record what the harness installed".to_string()
        } else if self.upgraded.is_empty() {
            COMMIT_MESSAGE.to_string()
        } else {
            format!(
                "chore(harness): upgrade what the harness installed\n\n{}\n",
                self.upgraded.join("\n")
            )
        }
    }

    /// Nothing to write: the repository already carries everything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }
}

/// A text's fingerprint: FNV-1a over its bytes, in hex. Not a defence
/// against anyone — only a way to tell "unchanged since the harness wrote
/// it" without keeping every version.
#[must_use]
pub fn fingerprint(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// The manifest's fingerprints by path; empty when it does not parse.
fn parse_manifest(text: &str) -> BTreeMap<String, String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("files").cloned())
        .and_then(|files| serde_json::from_value(files).ok())
        .unwrap_or_default()
}

/// The manifest's text: stable, one path per line, sorted.
fn render_manifest(files: &BTreeMap<String, String>) -> String {
    let value = serde_json::json!({ "files": files });
    let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
    text.push('\n');
    text
}

/// True when a text carries a marked block of an older version than
/// `current`, the one these assets install for it: a reinstall replaces it.
/// The documents and the rules block of `CLAUDE.md` count apart.
fn outdated(text: &str, current: u32) -> bool {
    installed_version(text).is_some_and(|v| v < current)
}

/// The `CLAUDE.md` a run must write, or `None` when it is to be left alone.
///
/// Absent: the block alone. Present without the block: the block appended
/// after the project's own text. Present with the block: left alone, unless
/// `force` or the block is of an older version — then the block is rewritten
/// in place, and nothing outside it moves.
#[must_use]
pub fn merge_claude_md(existing: Option<&str>, force: bool) -> Option<String> {
    let block = CLAUDE_RULES.trim_end();
    let Some(text) = existing else {
        return Some(format!("{block}\n"));
    };
    let Some(open) = text.find(MARKER_PREFIX) else {
        return Some(format!("{}\n\n{block}\n", text.trim_end()));
    };
    if !force && !outdated(text, CLAUDE_VERSION) {
        return None;
    }
    let close = text[open..]
        .find(MARKER_CLOSE)
        .map_or(text.len(), |at| open + at + MARKER_CLOSE.len());
    Some(format!(
        "{}{block}{}",
        &text[..open],
        text.get(close..).unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_architecture::assets::{CLAUDE_MARKER_OPEN, MARKER_OPEN};

    #[test]
    fn the_installed_ci_runs_on_the_integration_and_milestone_branches() {
        let ci = harness_architecture::assets::content_of(".github/workflows/ci.yml");
        assert!(ci.is_some());
        assert!(crate::init_repo::data::audit::ci_triggers_on(
            ci,
            "main_agent"
        ));
        assert!(crate::init_repo::data::audit::ci_triggers_on_milestones(ci));
    }

    #[test]
    fn an_empty_repository_gets_every_file_and_a_claude_md_made_of_the_rules() {
        let plan = InstallPlan::new(&|_| None, false);
        assert_eq!(plan.writes.len(), FILES.len() + 2, "and the manifest");
        assert_eq!(plan.kept, [] as [std::string::String; 0]);
        let (_, claude) = plan
            .writes
            .iter()
            .find(|(path, _)| path == CLAUDE_MD)
            .expect("CLAUDE.md");
        assert!(claude.starts_with(CLAUDE_MARKER_OPEN));
    }

    #[test]
    fn a_file_already_there_is_kept_unless_forced() {
        let existing = |path: &str| {
            (path == "contracts/a-concept.schema.json").then(|| "{\"mine\": true}".to_string())
        };
        let plan = InstallPlan::new(&existing, false);
        assert_eq!(plan.kept, ["contracts/a-concept.schema.json"]);
        assert_eq!(plan.diverged, ["contracts/a-concept.schema.json"]);
        assert_eq!(plan.writes.len(), FILES.len() + 1);
        let forced = InstallPlan::new(&existing, true);
        assert_eq!(forced.kept, [] as [std::string::String; 0]);
        assert_eq!(forced.writes.len(), FILES.len() + 2);
    }

    #[test]
    fn the_rules_are_appended_after_the_project_s_own_claude_md() {
        let merged =
            merge_claude_md(Some("# my project\n\nwhat it is.\n"), false).expect("appended");
        assert!(merged.starts_with(&format!(
            "# my project\n\nwhat it is.\n\n{CLAUDE_MARKER_OPEN}"
        )));
        assert!(merged.trim_end().ends_with(MARKER_CLOSE));
    }

    #[test]
    fn a_claude_md_that_carries_the_block_is_left_alone_and_rewritten_only_under_force() {
        let text = format!(
            "# mine\n\n{CLAUDE_MARKER_OPEN}\nold rules\n{MARKER_CLOSE}\n\n## after\n\nkept too\n"
        );
        assert_eq!(merge_claude_md(Some(&text), false), None);
        let forced = merge_claude_md(Some(&text), true).expect("rewritten");
        assert!(forced.starts_with(&format!("# mine\n\n{CLAUDE_MARKER_OPEN}")));
        assert!(!forced.contains("old rules"));
        assert!(forced.ends_with("\n\n## after\n\nkept too\n"), "{forced}");
        let plan = InstallPlan::new(&|path| (path == CLAUDE_MD).then(|| text.clone()), false);
        assert!(plan.kept.contains(&CLAUDE_MD.to_string()));
    }

    #[test]
    fn an_older_block_is_replaced_in_place_without_force_and_never_added_twice() {
        let old = format!(
            "# mine\n\n<!-- harness:architecture v1 -->\nold rules\n{MARKER_CLOSE}\n\n## after\n"
        );
        let merged = merge_claude_md(Some(&old), false).expect("replaced");
        assert!(merged.starts_with(&format!("# mine\n\n{CLAUDE_MARKER_OPEN}")));
        assert!(!merged.contains("old rules"));
        assert_eq!(merged.matches(MARKER_PREFIX).count(), 1, "one block");
        assert!(merged.ends_with("\n\n## after\n"), "{merged}");
        // The same for a document the harness installs: an older one is
        // rewritten, a foreign file of the same name is kept.
        let older_doc = "<!-- harness:architecture v1 -->\nold\n<!-- /harness:architecture -->\n";
        let plan = InstallPlan::new(
            &|path| match path {
                "docs/ARCHITECTURE.md" => Some(older_doc.to_string()),
                "docs/DATA-INTEGRITY.md" => Some("# theirs\n".to_string()),
                _ => None,
            },
            false,
        );
        assert!(plan.writes.iter().any(|(p, _)| p == "docs/ARCHITECTURE.md"));
        assert_eq!(plan.kept, ["docs/DATA-INTEGRITY.md"]);
        let same = format!("{MARKER_OPEN}\ncurrent\n{MARKER_CLOSE}\n");
        let kept = InstallPlan::new(
            &|path| (path == "docs/ARCHITECTURE.md").then(|| same.clone()),
            false,
        );
        assert!(
            kept.kept.contains(&"docs/ARCHITECTURE.md".to_string()),
            "the current version stays"
        );
    }

    #[test]
    fn a_shorter_rules_block_rewrites_claude_md_and_leaves_the_documents_alone() {
        let v2 = "<!-- harness:architecture v2 -->";
        let claude = format!("# mine\n\n{v2}\nlong rules\n{MARKER_CLOSE}\n");
        let document = format!("{v2}\na human's edit\n{MARKER_CLOSE}\n");
        let plan = InstallPlan::new(
            &|path| match path {
                CLAUDE_MD => Some(claude.clone()),
                "docs/ARCHITECTURE.md" | "docs/DATA-INTEGRITY.md" => Some(document.clone()),
                _ => None,
            },
            false,
        );
        let (_, written) = plan
            .writes
            .iter()
            .find(|(path, _)| path == CLAUDE_MD)
            .expect("CLAUDE.md rewritten");
        assert!(written.starts_with(&format!("# mine\n\n{CLAUDE_MARKER_OPEN}")));
        assert!(!written.contains("long rules"));
        for doc in ["docs/ARCHITECTURE.md", "docs/DATA-INTEGRITY.md"] {
            assert!(plan.kept.contains(&doc.to_string()), "{doc} kept");
        }
    }

    #[test]
    fn a_repository_with_everything_has_an_empty_plan() {
        let full = |path: &str| {
            if path == CLAUDE_MD {
                Some(CLAUDE_RULES.to_string())
            } else {
                FILES
                    .iter()
                    .find(|a| a.path == path)
                    .map(|a| a.content.to_string())
            }
        };
        let first = InstallPlan::new(&full, false);
        assert_eq!(
            first.writes.len(),
            1,
            "a repository without one records the manifest"
        );
        assert_eq!(
            first.commit_message(),
            "chore(harness): record what the harness installed"
        );
        let manifest = first.writes[0].1.clone();
        let plan = InstallPlan::new(
            &|path| {
                if path == MANIFEST {
                    Some(manifest.clone())
                } else {
                    full(path)
                }
            },
            false,
        );
        assert!(plan.is_empty(), "expected nothing");
        assert_eq!(plan.kept.len(), FILES.len() + 1);
    }

    /// A manifest that records `text` as what the harness wrote at `path`.
    fn manifest_of(path: &str, text: &str) -> String {
        format!("{{\"files\": {{\"{path}\": \"{}\"}}}}", fingerprint(text))
    }

    #[test]
    fn a_file_the_product_never_touched_follows_the_harness() {
        // ForgeDesMondes: the installed contract B predated `ports`.
        let path = "contracts/b-capability-manifest.schema.json";
        let old = "{\"old\": true}";
        let manifest = manifest_of(path, old);
        let plan = InstallPlan::new(
            &|p| match p {
                MANIFEST => Some(manifest.clone()),
                p if p == path => Some(old.to_string()),
                _ => None,
            },
            false,
        );
        assert_eq!(plan.upgraded, [path]);
        let (_, written) = plan
            .writes
            .iter()
            .find(|(p, _)| p == path)
            .expect("rewritten");
        assert_eq!(
            Some(written.as_str()),
            harness_architecture::assets::content_of(path)
        );
        let (_, recorded) = plan
            .writes
            .iter()
            .find(|(p, _)| p == MANIFEST)
            .expect("the manifest follows");
        assert!(recorded.contains(&fingerprint(written)));
        assert!(plan.commit_message().contains(path));
    }

    #[test]
    fn a_file_the_product_changed_is_kept_and_its_record_with_it() {
        let path = "contracts/b-capability-manifest.schema.json";
        let manifest = manifest_of(path, "{\"old\": true}");
        let plan = InstallPlan::new(
            &|p| match p {
                MANIFEST => Some(manifest.clone()),
                p if p == path => Some("{\"theirs\": true}".to_string()),
                _ => None,
            },
            false,
        );
        assert_eq!(plan.diverged, [path]);
        assert!(plan.upgraded.is_empty(), "plan.upgraded should be empty");
        assert!(!plan.writes.iter().any(|(p, _)| p == path));
        let (_, recorded) = plan
            .writes
            .iter()
            .find(|(p, _)| p == MANIFEST)
            .expect("manifest");
        assert!(recorded.contains(&fingerprint("{\"old\": true}")));
        assert_eq!(plan.commit_message(), COMMIT_MESSAGE);
        assert_eq!(fingerprint(""), "cbf29ce484222325");
        assert_eq!(fingerprint("a"), "af63dc4c8601ec8c", "FNV-1a's own vector");
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }
}
