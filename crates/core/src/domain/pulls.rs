//! What you read from a PR. Pure domain: no I/O, no subprocesses.

/// A PR, as a review reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pr {
    /// Its number, as text — it only appears in prose and paths.
    pub num: String,
    /// The targeted branch.
    pub base: String,
    /// The branch that carries the change.
    pub head: String,
    /// Its title.
    pub title: String,
    /// Its URL.
    pub url: String,
    /// `OPEN`, `CLOSED`, `MERGED` — as the API renders it, uninterpreted.
    pub state: String,
    /// A draft has nothing to be reviewed.
    pub draft: bool,
    /// Its labels, names only — what a workflow triggered by a label on a
    /// PR checks before spending anything.
    pub labels: Vec<String>,
}

impl Pr {
    /// `#12` — as a message names it.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("#{}", self.num)
    }

    /// Does it carry this label?
    #[must_use]
    pub fn has(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }

    /// Is it still open?
    ///
    /// `OPEN` as GitHub spells it, case-insensitively — a merged or closed
    /// PR is not something to work on. An empty state reads as open, the
    /// same default [`Pr`]'s own reader applies.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.state.is_empty() || self.state.eq_ignore_ascii_case("open")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_is_the_number_prefixed() {
        let pr = Pr {
            num: "32".to_string(),
            ..Pr::default()
        };
        assert_eq!(pr.reference(), "#32");
    }

    #[test]
    fn a_label_is_found_by_name_and_nothing_else_is() {
        let pr = Pr {
            labels: vec!["harness:pr-fix".to_string()],
            ..Pr::default()
        };
        assert!(pr.has("harness:pr-fix"));
        assert!(!pr.has("harness:to-review"));
        assert!(!pr.has("harness:pr"), "no prefix match");
    }

    #[test]
    fn merged_and_closed_are_not_open_however_they_are_spelled() {
        for state in ["MERGED", "CLOSED", "closed", "merged"] {
            let pr = Pr {
                state: state.to_string(),
                ..Pr::default()
            };
            assert!(!pr.is_open(), "{state} must not read as open");
        }
        for state in ["OPEN", "open", ""] {
            let pr = Pr {
                state: state.to_string(),
                ..Pr::default()
            };
            assert!(pr.is_open(), "{state:?} must read as open");
        }
    }
}
