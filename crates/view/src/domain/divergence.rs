//! A diverging data model, as the human reads it: the diagnosis the watch
//! recorded (`DataModelDiverged`), found again in the events, and the issue
//! the human may open from it — whose labels start the pipeline on their own.

use std::fmt::Write as _;

use harness_core::ports::store::events::Stored;
use harness_core::traces::Event;
use harness_workflows::common::labels;
use serde::Serialize;

/// The labels an issue opened from a divergence carries: a roadmap item,
/// ready — the planner takes it from there, the split, the loop.
pub const ISSUE_LABELS: [&str; 2] = [labels::ROADMAP, labels::READY];

/// The longest resolution a human may write, in characters.
pub const MAX_RESOLUTION: usize = 8_000;

/// One diagnosed divergence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Divergence {
    /// When it was diagnosed.
    pub at: String,
    /// The branch.
    pub branch: String,
    /// The commit read.
    pub sha: String,
    /// The version's folder on the archive branch.
    pub dir: String,
    /// What the static comparison found.
    pub divergences: Vec<String>,
    /// The diagnosis, markdown; empty when it failed.
    pub diagnosis: String,
    /// What a human can decide.
    pub actions: Vec<String>,
    /// Why there is no diagnosis.
    pub failure: String,
}

/// The newest divergence recorded for `branch` — or for `dir`, a version,
/// when the page names one.
#[must_use]
pub fn find(events: &[Stored], branch: &str, dir: Option<&str>) -> Option<Divergence> {
    events.iter().rev().find_map(|stored| match &stored.event {
        Event::DataModelDiverged {
            branch: b,
            sha,
            dir: d,
            divergences,
            diagnosis,
            actions,
            failure,
        } if b == branch && dir.is_none_or(|want| want == d) => Some(Divergence {
            at: stored.at.clone(),
            branch: b.clone(),
            sha: sha.clone(),
            dir: d.clone(),
            divergences: divergences.clone(),
            diagnosis: diagnosis.clone(),
            actions: actions.clone(),
            failure: failure.clone(),
        }),
        _ => None,
    })
}

/// The issue a human opens: its title and its body — the state of affairs,
/// then what the human decided.
///
/// # Errors
/// The sentence the page shows: nothing written, or too much.
pub fn issue(d: &Divergence, resolution: &str) -> Result<(String, String), String> {
    let resolution = resolution.trim();
    if resolution.is_empty() {
        return Err("write how to resolve it first".to_string());
    }
    if resolution.chars().count() > MAX_RESOLUTION {
        return Err(format!("at most {MAX_RESOLUTION} characters"));
    }
    let short: String = d.sha.chars().take(7).collect();
    let title = format!("Bring the data model of {} back in agreement", d.branch);
    let list = |items: &[String]| {
        items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut body = format!(
        "The database and the agents' data model disagree on `{}` at `{short}` \
         (archived on `harness/data-model` in `{}`).\n\n## How to resolve it\n\n{resolution}\n\n\
         ## The divergences\n\n{}\n",
        d.branch,
        d.dir,
        list(&d.divergences)
    );
    if !d.diagnosis.is_empty() {
        let _ = write!(body, "\n## The diagnosis\n\n{}\n", d.diagnosis);
    }
    if !d.actions.is_empty() {
        let _ = write!(body, "\n## Actions it proposed\n\n{}\n", list(&d.actions));
    }
    body.push_str(
        "\n---\nOpened from the factory view's data model pane. A migration changes the \
         structure; `data/model.json` (contract M) describes it — rule 14 of `CLAUDE.md`.\n",
    );
    Ok((title, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(id: u64, branch: &str, dir: &str) -> Stored {
        Stored {
            id,
            at: format!("2026-10-09T00:0{id}:00Z"),
            source: "watch".to_string(),
            event: Event::DataModelDiverged {
                branch: branch.to_string(),
                sha: "abcdef0123".to_string(),
                dir: dir.to_string(),
                divergences: vec![
                    "field pj.origine is in the database, not in the model".to_string(),
                ],
                diagnosis: "The model is behind.".to_string(),
                actions: vec!["Describe pj.origine.".to_string()],
                failure: String::new(),
            },
        }
    }

    #[test]
    fn the_newest_of_a_branch_or_the_named_version_is_found() {
        let events = [
            stored(1, "main_agent", "main_agent/v1"),
            stored(2, "main", "main/v1"),
            stored(3, "main_agent", "main_agent/v2"),
        ];
        assert_eq!(
            find(&events, "main_agent", None).map(|d| d.dir),
            Some("main_agent/v2".into())
        );
        assert_eq!(
            find(&events, "main_agent", Some("main_agent/v1")).map(|d| d.at),
            Some("2026-10-09T00:01:00Z".into())
        );
        assert!(find(&events, "milestone/1-x", None).is_none());
    }

    #[test]
    fn the_issue_carries_the_human_s_words_first() {
        let d = find(
            &[stored(1, "main_agent", "main_agent/v1")],
            "main_agent",
            None,
        )
        .expect("found");
        assert!(issue(&d, "   ").is_err());
        assert!(issue(&d, &"x".repeat(MAX_RESOLUTION + 1)).is_err());
        let (title, body) =
            issue(&d, "Add origine to the model; keep the column.").expect("composed");
        assert_eq!(
            title,
            "Bring the data model of main_agent back in agreement"
        );
        let resolve = body.find("## How to resolve it").expect("section");
        let found = body.find("## The divergences").expect("section");
        assert!(resolve < found);
        assert!(body.contains("Add origine to the model; keep the column."));
        assert!(body.contains("`abcdef0`"));
        assert!(body.contains("- Describe pj.origine."));
    }
}
