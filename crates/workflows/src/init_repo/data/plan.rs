//! What `init-repo` must write, decided from what was already read.
//!
//! Pure: decides, reads nothing.

use crate::common::labels::{self, Label};

/// What the integration branch needs.
pub enum BranchAction {
    /// It already exists — never fast-forwarded, never touched.
    AlreadyExists,
    /// It must be created, at this sha.
    Create {
        /// The sha of the repo's default branch.
        at_sha: String,
    },
}

/// The writes a run of `init-repo` must make.
pub struct Plan {
    /// The labels missing from the repo, in `ALL`'s order.
    pub missing_labels: Vec<&'static Label>,
    /// What the integration branch needs.
    pub branch_action: BranchAction,
}

impl Plan {
    /// Decides the plan from what was read: the repo's current labels, the
    /// integration branch's sha if it exists, and the default branch's sha.
    #[must_use]
    pub fn new(existing_labels: &[String], branch_sha: Option<&str>, default_sha: &str) -> Self {
        let missing_labels = labels::ALL
            .iter()
            .filter(|label| !existing_labels.iter().any(|name| name == label.name))
            .collect();
        let branch_action = match branch_sha {
            Some(_) => BranchAction::AlreadyExists,
            None => BranchAction::Create {
                at_sha: default_sha.to_string(),
            },
        };
        Self {
            missing_labels,
            branch_action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_label_names() -> Vec<&'static str> {
        labels::ALL.iter().map(|l| l.name).collect()
    }

    #[test]
    fn all_labels_present_means_zero_writes() {
        let present: Vec<String> = all_label_names().iter().map(ToString::to_string).collect();
        let plan = Plan::new(&present, Some("abc"), "def");
        assert!(plan.missing_labels.is_empty());
    }

    #[test]
    fn three_missing_means_exactly_those_three() {
        let present: Vec<String> = all_label_names()
            .iter()
            .skip(3)
            .map(ToString::to_string)
            .collect();
        let plan = Plan::new(&present, Some("abc"), "def");
        assert_eq!(plan.missing_labels.len(), 3);
        for label in &plan.missing_labels {
            assert!(all_label_names()[..3].contains(&label.name));
        }
    }

    #[test]
    fn an_absent_branch_plans_a_create_at_the_default_sha() {
        let plan = Plan::new(&[], None, "deadbeef");
        assert!(matches!(
            plan.branch_action,
            BranchAction::Create { ref at_sha } if at_sha == "deadbeef"
        ));
    }

    #[test]
    fn an_existing_branch_plans_no_write_at_all() {
        let plan = Plan::new(&[], Some("abc123"), "deadbeef");
        assert!(matches!(plan.branch_action, BranchAction::AlreadyExists));
    }
}
