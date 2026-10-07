//! The two halves of a refinement: what needs no code, and what does.
//!
//! The business half rewrites the sections that only depend on the request
//! and can evolve without the repository; the technical half needs the code
//! and is better done once the tasks it builds on are delivered.

use crate::common::labels;

/// Which half of the refinement a run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// Business Goal, Acceptance Criteria, Business Rules — no code needed.
    #[default]
    Business,
    /// Technical, Technical Implementation Plan — reads the code.
    Technical,
}

impl Phase {
    /// The section keys this phase writes, in body order.
    #[must_use]
    pub const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Business => &["business-goal", "acceptance-criteria", "business-rules"],
            Self::Technical => &["technical", "technical-plan"],
        }
    }

    /// The label that asks for a round of this phase.
    #[must_use]
    pub const fn requested_by(self) -> &'static str {
        match self {
            Self::Business => labels::REFINEMENT,
            Self::Technical => labels::TECH_REFINEMENT,
        }
    }

    /// The label this phase leaves once its round is written.
    #[must_use]
    pub const fn leaves(self) -> &'static str {
        match self {
            Self::Business => labels::SPEC_WRITTEN,
            Self::Technical => labels::TECH_WRITTEN,
        }
    }

    /// Which of this phase's sections drag which, as the router is told.
    ///
    /// Phase-specific, and that is the point: a business round used to be told
    /// that "a changed Technical section drags the Technical Implementation
    /// Plan" — two sections it is not allowed to name, so the sentence could
    /// only produce a reply the filter then threw away.
    #[must_use]
    pub const fn drags(self) -> &'static str {
        match self {
            Self::Business => {
                "a changed Business Goal usually drags the Acceptance Criteria \
                 with it, and reworked Acceptance Criteria usually drag the \
                 Business Rules"
            }
            Self::Technical => {
                "a changed Technical section usually drags the Technical \
                 Implementation Plan with it"
            }
        }
    }

    /// The round-end comment marker, matched case-insensitively.
    ///
    /// Distinct per phase: each one counts its own rounds. The technical
    /// marker does not start with the business one, so neither counter reads
    /// the other's comments.
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Business => "refinement round:",
            Self::Technical => "technical refinement round:",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_phases_cover_every_section_but_scope_without_overlap() {
        use crate::common::sections;

        let mut all: Vec<&str> = Phase::Business.keys().to_vec();
        all.extend(Phase::Technical.keys());
        for key in &all {
            assert_eq!(all.iter().filter(|k| k == &key).count(), 1);
        }
        // `scope` is the slice's own boundary: written once by `split`, owned
        // by no phase, so no round can rewrite or erase it.
        assert!(!all.contains(&sections::SCOPE));
        assert_eq!(all.len(), sections::KEYS.len() - 1);
        for key in sections::KEYS {
            assert!(
                key == sections::SCOPE || all.contains(&key),
                "{key} belongs to no phase and is not scope"
            );
        }
    }

    #[test]
    fn a_phase_only_names_sections_it_is_allowed_to_write() {
        // The business round was told about the two technical sections, which
        // its own filter then dropped: a sentence that could only mislead.
        let business = Phase::Business.drags().to_lowercase();
        assert!(!business.contains("technical"));
        assert!(Phase::Technical.drags().contains("Technical"));
    }

    #[test]
    fn neither_marker_is_matched_by_the_other() {
        assert!(
            !Phase::Technical
                .marker()
                .starts_with(Phase::Business.marker())
        );
        assert!(
            !Phase::Business
                .marker()
                .starts_with(Phase::Technical.marker())
        );
    }
}
