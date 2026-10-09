//! `index.json`, at the root of the archive branch: the last head read of
//! every branch, and every version kept.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The archive's index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Index {
    /// The format's version; 1.
    pub version: u32,
    /// The last head read, by branch — archived or found unchanged.
    pub heads: BTreeMap<String, String>,
    /// Every version kept, oldest first.
    pub snapshots: Vec<Snapshot>,
}

/// One version of a branch's data model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The branch.
    pub branch: String,
    /// The commit read.
    pub sha: String,
    /// When, ISO-8601 UTC.
    pub at: String,
    /// The folder on the archive branch.
    pub dir: String,
    /// Whether `schema.sql` is in it — empty when the checkout had no
    /// database yet.
    pub schema: bool,
    /// Whether `model.json` is in it.
    pub model: bool,
}

/// One branch's head to read: the branch, its sha.
pub type Head = (String, String);

impl Index {
    /// The index as kept; absent or unreadable, an empty one.
    #[must_use]
    pub fn parse(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_else(|| Self {
                version: 1,
                ..Self::default()
            })
    }

    /// The heads not read yet: a branch that exists and moved since.
    #[must_use]
    pub fn pending(&self, heads: &[(String, Option<String>)]) -> Vec<Head> {
        heads
            .iter()
            .filter_map(|(branch, sha)| {
                let sha = sha.as_ref()?;
                (self.heads.get(branch) != Some(sha)).then(|| (branch.clone(), sha.clone()))
            })
            .collect()
    }

    /// The branch's last version.
    #[must_use]
    pub fn latest(&self, branch: &str) -> Option<&Snapshot> {
        self.snapshots.iter().rev().find(|s| s.branch == branch)
    }

    /// The index as written: stable, one key per line.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }
}

/// Where a version lands: `<branch>/<YYYYMMDDTHHMMSSZ>-<sha7>`. A branch's
/// slashes are folders; anything else outside `[A-Za-z0-9._-]` becomes `-`.
#[must_use]
pub fn dir_for(branch: &str, at: &str, sha: &str) -> String {
    let branch: String = branch
        .split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(|part| {
            part.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/");
    // `2026-10-09T12:34:56.789Z` → `20261009T123456Z`: seconds, in UTC.
    let mut stamp: String = at
        .split(['.', 'Z'])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    stamp.push('Z');
    let branch = if branch.is_empty() {
        "_".to_string()
    } else {
        branch
    };
    let short: String = sha.chars().take(7).collect();
    format!("{branch}/{stamp}-{short}")
}

/// The commit message: every head archived or moved.
#[must_use]
pub fn commit_message(heads: &[Head]) -> String {
    let list = heads
        .iter()
        .map(|(branch, sha)| format!("{branch} @ {}", sha.chars().take(7).collect::<String>()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("data snapshot: {list}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn snap(branch: &str, sha: &str) -> Snapshot {
        Snapshot {
            branch: branch.to_string(),
            sha: sha.to_string(),
            at: "2026-10-09T12:00:00Z".to_string(),
            dir: dir_for(branch, "2026-10-09T12:00:00Z", sha),
            schema: true,
            model: false,
        }
    }

    #[test]
    fn only_the_heads_that_moved_are_read() {
        let mut index = Index::parse(None);
        index.heads.insert("main".into(), "aaa".into());
        index.heads.insert("main_agent".into(), "bbb".into());
        let pending = index.pending(&[
            ("main".into(), Some("aaa".into())),
            ("main_agent".into(), Some("ccc".into())),
            ("milestone/3-x".into(), Some("ddd".into())),
            ("gone".into(), None),
        ]);
        assert_eq!(
            pending,
            [
                ("main_agent".to_string(), "ccc".to_string()),
                ("milestone/3-x".to_string(), "ddd".to_string())
            ]
        );
    }

    #[test]
    fn a_version_lands_under_its_branch_and_its_time() {
        assert_eq!(
            dir_for(
                "milestone/12-data model",
                "2026-10-09T12:34:56.789Z",
                "abcdef0123"
            ),
            "milestone/12-data-model/20261009T123456Z-abcdef0"
        );
        assert_eq!(
            dir_for("../main", "2026-10-09T12:34:56Z", "a"),
            "main/20261009T123456Z-a"
        );
    }

    #[test]
    fn the_index_round_trips_and_names_the_last_version() {
        let mut index = Index::parse(None);
        index.snapshots.push(snap("main", "a1"));
        index.snapshots.push(snap("main_agent", "b1"));
        index.snapshots.push(snap("main", "a2"));
        let back = Index::parse(Some(&index.to_json()));
        assert_eq!(back, index);
        assert_eq!(back.latest("main").map(|s| s.sha.as_str()), Some("a2"));
        assert!(back.latest("milestone/1-x").is_none());
        assert_eq!(Index::parse(Some("not json")).version, 1);
    }

    #[test]
    fn the_message_names_every_head() {
        assert_eq!(
            commit_message(&[
                ("main".into(), "abcdef0123".into()),
                ("main_agent".into(), "12".into())
            ]),
            "data snapshot: main @ abcdef0, main_agent @ 12"
        );
    }

    proptest! {
        #[test]
        fn a_folder_never_climbs_out_of_the_archive(branch in "\\PC{0,40}", sha in "[0-9a-f]{0,40}") {
            let dir = dir_for(&branch, "2026-10-09T12:00:00Z", &sha);
            prop_assert!(!dir.split('/').any(|part| part == ".." || part == "."));
            prop_assert!(!dir.starts_with('/'));
        }
    }
}
