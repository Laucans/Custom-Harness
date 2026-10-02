//! The `harness:*` labels, shared by workflows that read them.
//!
//! They are **created by hand on the repo**, and each workflow's preflight
//! checks that its labels exist before spending anything: a misspelled label
//! makes the list empty, and an empty list reads as "nothing left to do".
//!
//! Renamed from `pipeline:*` — the migration takes control of tracking rather
//! than coexist. The rename on GitHub is a human gesture: it stops the Python
//! pipeline the moment it is done. `REFINEMENT` is renamed as written, like
//! the seven others — what stays human is the `gh label edit` itself,
//! documented in `docs/CUTOVER.md`.

/// A roadmap item: what `/planner` draws a milestone from.
pub const ROADMAP: &str = "harness:roadmap";

/// A milestone: the current batch of work.
pub const MILESTONE: &str = "harness:milestone";

/// A task the agent can run.
pub const AGENT: &str = "harness:agent";

/// What only the human can do. Blocked by dependency, not by mechanism.
pub const HUMAN: &str = "harness:human";

/// The checkbox only the human checks. Nothing is taken without it.
pub const READY: &str = "harness:ready";

/// The SPEC has been written into the issue body.
pub const SPEC_WRITTEN: &str = "harness:spec-written";

/// Delivered on the integration branch, not yet merged into `main`.
///
/// The issue stays **open**: closing it would say the work is integrated,
/// which is true only after the merge. This third state is what separates
/// "the agent is done" from "it is in `main`".
pub const WAITING_MERGE: &str = "harness:waiting-merge";

/// A refinement round remains to be done on this issue.
///
/// Set by a human (or the planner) to request a round; refinement removes it
/// once the round is written.
pub const REFINEMENT: &str = "harness:refinement";

/// The seven the loop requires.
///
/// `REFINEMENT` is not among them: the loop does not read it, and its
/// preflight would refuse to run without it.
pub const LOOP: [&str; 7] = [
    ROADMAP,
    MILESTONE,
    AGENT,
    HUMAN,
    READY,
    SPEC_WRITTEN,
    WAITING_MERGE,
];

/// One `harness:*` label, as `init-repo` creates it.
pub struct Label {
    /// The label's name, e.g. `harness:ready`.
    pub name: &'static str,
    /// The hex color GitHub shows it with, no leading `#`.
    pub color: &'static str,
    /// What the label means, shown next to it on GitHub.
    pub description: &'static str,
}

/// The eight labels `init-repo` creates when they are missing.
///
/// An existing label is never recolored or re-described: a human may have
/// adjusted it, and the only thing the harness needs is that the name
/// exists.
pub const ALL: [Label; 8] = [
    Label {
        name: ROADMAP,
        color: "5319e7",
        description: "A roadmap item: what /planner draws a milestone from",
    },
    Label {
        name: MILESTONE,
        color: "0e8a16",
        description: "A milestone: the current batch of work",
    },
    Label {
        name: AGENT,
        color: "1d76db",
        description: "A task the agent can run",
    },
    Label {
        name: HUMAN,
        color: "d93f0b",
        description: "What only the human can do",
    },
    Label {
        name: READY,
        color: "fbca04",
        description: "The checkbox only the human checks",
    },
    Label {
        name: SPEC_WRITTEN,
        color: "c2e0c6",
        description: "The SPEC has been written into the issue body",
    },
    Label {
        name: WAITING_MERGE,
        color: "bfd4f2",
        description: "Delivered on the integration branch, not yet merged",
    },
    Label {
        name: REFINEMENT,
        color: "d4c5f9",
        description: "A refinement round remains to be done",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_the_loop_requires_is_in_the_list() {
        // Preflight reads this list: a label forgotten here would be a label
        // it does not verify, so an unexplained empty list.
        for label in [
            ROADMAP,
            MILESTONE,
            AGENT,
            HUMAN,
            READY,
            SPEC_WRITTEN,
            WAITING_MERGE,
        ] {
            assert!(LOOP.contains(&label), "{label} missing from LOOP");
        }
    }

    #[test]
    fn they_all_share_the_namespace_and_none_is_a_prefix_of_another() {
        let all = [
            ROADMAP,
            MILESTONE,
            AGENT,
            HUMAN,
            READY,
            SPEC_WRITTEN,
            WAITING_MERGE,
            REFINEMENT,
        ];
        for label in all {
            assert!(label.starts_with("harness:"), "{label} outside namespace");
        }
        // A label that is a prefix of another would make `has()` ambiguous if
        // someone ever moved to a prefix comparison.
        for a in all {
            let prefixes = all.iter().filter(|b| b.starts_with(a)).count();
            assert_eq!(prefixes, 1, "{a} is the prefix of another label");
        }
    }

    #[test]
    fn every_const_the_loop_or_refinement_names_is_in_all() {
        let names: Vec<&str> = ALL.iter().map(|l| l.name).collect();
        for label in LOOP {
            assert!(names.contains(&label), "{label} missing from ALL");
        }
        assert!(names.contains(&REFINEMENT), "{REFINEMENT} missing from ALL");
    }

    #[test]
    fn all_entries_have_distinct_names() {
        let names: Vec<&str> = ALL.iter().map(|l| l.name).collect();
        for name in &names {
            assert_eq!(
                names.iter().filter(|n| n == &name).count(),
                1,
                "{name} appears more than once in ALL"
            );
        }
    }
}
