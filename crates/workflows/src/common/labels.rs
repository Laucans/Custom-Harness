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

/// The technical sections have been written into the issue body.
///
/// Set by the technical refinement (or, when nobody asked for one, by the
/// dev loop's own technical stage) — the loop skips that stage once it is
/// there.
pub const TECH_WRITTEN: &str = "harness:tech-written";

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

/// A technical refinement round remains to be done on this issue.
///
/// Same gesture as [`REFINEMENT`], for the sections that need the code:
/// `Technical` and `Technical Implementation Plan`. Without it, the dev loop
/// writes them itself in a stage of its own.
pub const TECH_REFINEMENT: &str = "harness:tech-refinement";

/// The refinement's advice says a human has a decision to make on this issue.
///
/// Posed — or removed — by the **business** refinement, from the same reading
/// of the advice that writes the comment: see
/// [`advice::Read::wants_a_human`](crate::refinement::data::advice::Read::wants_a_human).
/// Derived state, re-decided on every round, so a later round that concludes
/// `no` clears it rather than leaving the board claiming a blocker that is gone.
///
/// **What it is for**: `gh issue list --label harness:needs-decision` answers
/// "what is waiting on me" in one call. The advice already said so in a comment,
/// but a comment is only found by opening the issue — on a board of twenty
/// tasks, that is twenty issues to open.
///
/// **It is not read by any workflow**, hence not in [`LOOP`]: nothing is gated
/// on it, and a task carrying it still runs the moment [`READY`] is set. The
/// human removes it by hand when they have answered — the same gesture as
/// adding [`READY`], which is how they say they decided.
pub const NEEDS_DECISION: &str = "harness:needs-decision";

/// This milestone has already been split into tasks.
///
/// Posed by the split workflow once it has opened a milestone's tasks, in
/// the same breath as removing `READY` — a re-poll must not re-split a
/// milestone it already saw. Removing `TRIGGERED` by hand is the resplit
/// gesture. Not read by `dev_loop`, hence not in `LOOP`.
pub const TRIGGERED: &str = "harness:triggered";

/// This PR is waiting for an agent review.
///
/// Posed on a **pull request**, by a human or by whatever opened it. It is a
/// request, not state the harness clears: a PR that already carries a
/// review stops being offered on its own, because the review's own skip
/// rules say so. Not read by `dev_loop`, hence not in `LOOP`.
pub const TO_REVIEW: &str = "harness:to-review";

/// This PR's red CI is worth one repair attempt.
///
/// Posed on a **pull request**. It authorizes exactly one attempt: the
/// repair consumes it before paying for anything, so a PR that cannot be
/// fixed does not burn one session per poll. Re-posing it by hand is the
/// gesture that asks for another attempt. Not read by `dev_loop`, hence not
/// in `LOOP`.
pub const PR_FIX: &str = "harness:pr-fix";

/// The task touches only the **read side** of the agent-native architecture.
///
/// That architecture is `docs/ARCHITECTURE.md` of the target repo; its read
/// side is a Capability, a Micro-UI, a Concept, a persisted query, a screen
/// composition, or an insert-only `DataCapability`. Such a task runs and
/// merges on its own, and several of these may run in parallel — nothing
/// they touch is shared.
///
/// Posed by `split` from the task's `## Architecture` section
/// ([`crate::common::architecture`]), and by `planner` on a milestone whose
/// work is all read-side.
pub const READ_SIDE: &str = "harness:read-side";

/// The task touches the **write side** of the architecture.
///
/// A `DataCapability` that updates, deletes or upserts, an invariant, a
/// relation policy, a schema migration, anything behind the `DataGuard`.
/// The architecture asks a human to review every mutation of existing data,
/// so its PR is never merged by the session: it waits for a human merge
/// under [`REVIEW_PENDING`].
///
/// Posed by `split` (task) and `planner` (milestone), like [`READ_SIDE`].
pub const WRITE_SIDE: &str = "harness:write-side";

/// A write-side task's PR is open and waits for a human merge.
///
/// Posed by the dev loop once the session has opened the PR (with
/// `harness:to-review`, so the agent review runs on it) and stopped short of
/// merging. While it is there the loop neither replays the task nor starts
/// one that depends on it; the human merging the PR is the review, and the
/// next poll trades this label for [`WAITING_MERGE`].
pub const REVIEW_PENDING: &str = "harness:review-pending";

/// The eleven the loop requires.
///
/// `REFINEMENT` is not among them: the loop does not read it, and its
/// preflight would refuse to run without it. The three architecture labels
/// are: the loop reads the side of every task it picks, and `review-pending`
/// is what keeps a write-side task from being replayed.
pub const LOOP: [&str; 11] = [
    ROADMAP,
    MILESTONE,
    AGENT,
    HUMAN,
    READY,
    SPEC_WRITTEN,
    TECH_WRITTEN,
    WAITING_MERGE,
    READ_SIDE,
    WRITE_SIDE,
    REVIEW_PENDING,
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

/// The seventeen labels `init-repo` creates when they are missing.
///
/// An existing label is never recolored or re-described: a human may have
/// adjusted it, and the only thing the harness needs is that the name
/// exists.
pub const ALL: [Label; 17] = [
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
        name: TECH_WRITTEN,
        color: "c2e0c6",
        description: "The technical sections have been written into the issue body",
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
    Label {
        name: TECH_REFINEMENT,
        color: "d4c5f9",
        description: "A technical refinement round remains to be done",
    },
    Label {
        name: NEEDS_DECISION,
        color: "ee0701",
        description: "The refinement's advice says a human has a decision to make",
    },
    Label {
        name: TRIGGERED,
        color: "e99695",
        description: "This milestone has already been split into tasks",
    },
    Label {
        name: TO_REVIEW,
        color: "006b75",
        description: "This PR is waiting for an agent review",
    },
    Label {
        name: PR_FIX,
        color: "b60205",
        description: "This PR's red CI is worth one repair attempt",
    },
    Label {
        name: READ_SIDE,
        color: "0052cc",
        description: "Read side only: Capability, Micro-UI, Concept, query — runs in parallel, merges alone",
    },
    Label {
        name: WRITE_SIDE,
        color: "e11d21",
        description: "Write side: DataCapability, invariant, schema — a human merges its PR",
    },
    Label {
        name: REVIEW_PENDING,
        color: "f9d0c4",
        description: "A write-side PR is open and waits for a human merge",
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
            TECH_WRITTEN,
            WAITING_MERGE,
            READ_SIDE,
            WRITE_SIDE,
            REVIEW_PENDING,
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
            TECH_WRITTEN,
            WAITING_MERGE,
            REFINEMENT,
            TECH_REFINEMENT,
            NEEDS_DECISION,
            TRIGGERED,
            TO_REVIEW,
            PR_FIX,
            READ_SIDE,
            WRITE_SIDE,
            REVIEW_PENDING,
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
    fn every_const_the_loop_or_another_workflow_names_is_in_all() {
        let names: Vec<&str> = ALL.iter().map(|l| l.name).collect();
        for label in LOOP {
            assert!(names.contains(&label), "{label} missing from ALL");
        }
        // The six outside `LOOP`: `init-repo` still has to create them, or
        // the workflow that writes one would fail on a label GitHub does not
        // know — and for the ones a workflow reads, an empty list reads as
        // "nothing to do".
        for label in [
            REFINEMENT,
            TECH_REFINEMENT,
            NEEDS_DECISION,
            TRIGGERED,
            TO_REVIEW,
            PR_FIX,
        ] {
            assert!(names.contains(&label), "{label} missing from ALL");
        }
    }

    #[test]
    fn the_two_pull_request_labels_stay_out_of_the_loops_requirements() {
        // `dev_loop`'s preflight demands every label of `LOOP` exist before
        // it spends anything. A PR label it never reads has no business
        // blocking a run.
        for label in [TO_REVIEW, PR_FIX] {
            assert!(!LOOP.contains(&label), "{label} must not be in LOOP");
        }
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
