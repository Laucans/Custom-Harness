//! What the data pane can build when a branch lacks a file: its schema (a
//! snapshot, `harness data-snapshot`) or a first model (a draft proposed as a
//! pull request, `harness data-model-draft`). The view starts the harness's
//! own command; it never builds anything itself.

use serde::Serialize;

/// What is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum What {
    /// `schema.sql`: the branch's head dumped and archived.
    Schema,
    /// `model.json`: a first model drafted and proposed as a pull request.
    Model,
}

impl What {
    /// From the page's word.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "schema" => Some(Self::Schema),
            "model" => Some(Self::Model),
            _ => None,
        }
    }

    /// The harness's arguments that build it for `branch`.
    #[must_use]
    pub fn args(self, branch: &str) -> Vec<String> {
        let command = match self {
            Self::Schema => "data-snapshot",
            Self::Model => "data-model-draft",
        };
        vec![
            command.to_string(),
            "--branch".to_string(),
            branch.to_string(),
        ]
    }
}

/// Where a build is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Running.
    Running,
    /// Ended on exit 0.
    Done,
    /// Ended otherwise.
    Failed,
}

/// One build, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Build {
    /// Its id: what its log is asked by.
    pub id: String,
    /// What.
    pub what: What,
    /// For which branch.
    pub branch: String,
    /// Where it is.
    pub state: State,
    /// When it started, ISO-8601 UTC.
    pub started: String,
    /// The last lines it printed — why it failed, or the pull request it opened.
    pub tail: String,
}

/// The run folders a harness command said it journals in — its
/// `journal: <dir>` lines — whose `session.log` the build's log shows too.
#[must_use]
pub fn journals(output: &str) -> Vec<String> {
    let mut dirs: Vec<String> = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("journal: "))
        .map(|dir| dir.trim().to_string())
        .filter(|dir| !dir.is_empty())
        .collect();
    dirs.dedup();
    dirs
}

/// A branch name the page may send: what git accepts, nothing a shell or a
/// path could make more of.
#[must_use]
pub fn is_branch(name: &str) -> bool {
    (1..=200).contains(&name.len())
        && !name.starts_with(['-', '/', '.'])
        && !name.contains("..")
        && !name.ends_with(['/', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn each_file_names_its_command() {
        assert_eq!(
            What::parse("schema").map(|w| w.args("main")),
            Some(vec![
                "data-snapshot".into(),
                "--branch".into(),
                "main".into()
            ])
        );
        assert_eq!(What::Model.args("main_agent")[0], "data-model-draft");
        assert!(What::parse("other").is_none());
    }

    #[test]
    fn the_journals_a_command_named_are_found_in_its_output() {
        let out = "draft: dumped\njournal: /r/.llocal/logs/data-model-draft/1\n[draft] session opens\n  journal: /r/.llocal/logs/data-diagnosis/2\njournal: \n";
        assert_eq!(
            journals(out),
            [
                "/r/.llocal/logs/data-model-draft/1",
                "/r/.llocal/logs/data-diagnosis/2"
            ]
        );
        assert_eq!(journals("nothing"), [] as [String; 0]);
    }

    #[test]
    fn only_a_plain_branch_name_is_taken() {
        assert!(is_branch("main_agent"));
        assert!(is_branch("milestone/53-couche-data"));
        assert!(!is_branch("--force"));
        assert!(!is_branch("../etc"));
        assert!(!is_branch("a b"));
        assert!(!is_branch("a;rm"));
        assert!(!is_branch(""));
    }

    proptest! {
        #[test]
        fn a_taken_name_never_reads_as_an_option_or_climbs(name in "\\PC{0,40}") {
            if is_branch(&name) {
                prop_assert!(!name.starts_with('-'));
                prop_assert!(!name.contains(".."));
            }
        }
    }
}
