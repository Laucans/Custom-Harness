//! The versions of the product's data model the archive keeps — the target's
//! branch `harness/data-model`, written by the watch after every merge — put
//! in the order the page offers them: `main`, the integration branch, then
//! one entry per feature (milestone) branch, newest version first.

use serde::Serialize;

use harness_workflows::data_snapshot::data::index::{Index, Snapshot};

/// What a branch is to the human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `main`: what a human released.
    Release,
    /// The agents' integration branch: `main_agent`.
    Integration,
    /// A milestone's branch, where its tasks merge.
    Feature,
}

/// One version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Version {
    /// Its folder on the archive branch: what the page asks for.
    pub dir: String,
    /// The commit it was read at.
    pub sha: String,
    /// When, ISO-8601 UTC.
    pub at: String,
    /// The dump is in it.
    pub schema: bool,
    /// The agents' model is in it.
    pub model: bool,
}

/// One branch and its versions, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Track {
    /// As git names it.
    pub branch: String,
    /// What it is.
    pub kind: Kind,
    /// What the page writes: `main`, `#12 data model`.
    pub label: String,
    /// Newest first.
    pub versions: Vec<Version>,
}

/// Every branch the archive knows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Versions {
    /// The archive was found at all.
    pub archive: bool,
    /// Release, integration, then features by milestone number.
    pub branches: Vec<Track>,
}

/// The release branch's name.
pub const RELEASE: &str = "main";

/// Orders the archive's versions for the page.
#[must_use]
pub fn versions(index: Option<&Index>, integration: &str) -> Versions {
    let Some(index) = index else {
        return Versions::default();
    };
    let mut names: Vec<&str> = index.snapshots.iter().map(|s| s.branch.as_str()).collect();
    names.extend(index.heads.keys().map(String::as_str));
    names.sort_by_key(|name| rank(name, integration));
    names.dedup();
    let branches = names
        .into_iter()
        .map(|name| Track {
            branch: name.to_string(),
            kind: kind_of(name, integration),
            label: label_of(name),
            versions: index
                .snapshots
                .iter()
                .rev()
                .filter(|s| s.branch == name)
                .map(version)
                .collect(),
        })
        .collect();
    Versions {
        archive: true,
        branches,
    }
}

/// The version a page shows when it names none: the branch's newest, the
/// integration branch's when it names no branch either.
#[must_use]
pub fn pick<'a>(
    index: &'a Index,
    branch: Option<&str>,
    dir: Option<&str>,
    integration: &str,
) -> Option<&'a Snapshot> {
    if let Some(dir) = dir {
        return index.snapshots.iter().find(|s| s.dir == dir);
    }
    let branch = branch.unwrap_or(integration);
    index.latest(branch).or_else(|| {
        (branch == integration)
            .then(|| index.snapshots.last())
            .flatten()
    })
}

fn version(s: &Snapshot) -> Version {
    Version {
        dir: s.dir.clone(),
        sha: s.sha.clone(),
        at: s.at.clone(),
        schema: s.schema,
        model: s.model,
    }
}

fn kind_of(name: &str, integration: &str) -> Kind {
    if name == RELEASE {
        Kind::Release
    } else if name == integration {
        Kind::Integration
    } else {
        Kind::Feature
    }
}

/// Release first, integration second, then milestones by number, the rest
/// by name.
fn rank(name: &str, integration: &str) -> (u8, u64, String) {
    match kind_of(name, integration) {
        Kind::Release => (0, 0, String::new()),
        Kind::Integration => (1, 0, String::new()),
        Kind::Feature => (
            2,
            milestone_number(name).unwrap_or(u64::MAX),
            name.to_string(),
        ),
    }
}

fn milestone_number(name: &str) -> Option<u64> {
    name.strip_prefix("milestone/")?
        .split('-')
        .next()?
        .parse()
        .ok()
}

/// `milestone/12-data-model` → `#12 data model`; any other name as is.
fn label_of(name: &str) -> String {
    match (milestone_number(name), name.strip_prefix("milestone/")) {
        (Some(n), Some(rest)) => {
            let words = rest
                .split_once('-')
                .map_or("", |(_, w)| w)
                .replace('-', " ");
            if words.is_empty() {
                format!("#{n}")
            } else {
                format!("#{n} {words}")
            }
        }
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(branch: &str, sha: &str, at: &str) -> Snapshot {
        Snapshot {
            branch: branch.to_string(),
            sha: sha.to_string(),
            at: at.to_string(),
            dir: format!("{branch}/{sha}"),
            schema: true,
            model: true,
        }
    }

    fn index() -> Index {
        let mut index = Index::parse(None);
        index.snapshots = vec![
            snap("main_agent", "a1", "1"),
            snap("milestone/12-data-model", "m1", "2"),
            snap("main", "r1", "3"),
            snap("milestone/3-campaigns", "c1", "4"),
            snap("main_agent", "a2", "5"),
        ];
        index
    }

    #[test]
    fn release_then_integration_then_features_newest_first() {
        let v = versions(Some(&index()), "main_agent");
        assert!(v.archive);
        let order: Vec<(&str, Kind, &str)> = v
            .branches
            .iter()
            .map(|b| (b.branch.as_str(), b.kind, b.label.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("main", Kind::Release, "main"),
                ("main_agent", Kind::Integration, "main_agent"),
                ("milestone/3-campaigns", Kind::Feature, "#3 campaigns"),
                ("milestone/12-data-model", Kind::Feature, "#12 data model"),
            ]
        );
        assert_eq!(
            v.branches[1]
                .versions
                .iter()
                .map(|x| x.sha.as_str())
                .collect::<Vec<_>>(),
            ["a2", "a1"]
        );
        assert_eq!(versions(None, "main_agent"), Versions::default());
    }

    #[test]
    fn a_page_that_names_nothing_sees_the_integration_branch_newest() {
        let index = index();
        assert_eq!(
            pick(&index, None, None, "main_agent").map(|s| s.sha.as_str()),
            Some("a2")
        );
        assert_eq!(
            pick(&index, Some("main"), None, "main_agent").map(|s| s.sha.as_str()),
            Some("r1")
        );
        assert_eq!(
            pick(&index, None, Some("main_agent/a1"), "main_agent").map(|s| s.sha.as_str()),
            Some("a1")
        );
        assert!(pick(&index, Some("nowhere"), None, "main_agent").is_none());
        let mut only_features = Index::parse(None);
        only_features.snapshots = vec![snap("milestone/1-x", "f", "1")];
        assert_eq!(
            pick(&only_features, None, None, "main_agent").map(|s| s.sha.as_str()),
            Some("f")
        );
    }
}
