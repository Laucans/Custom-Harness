//! What each stage's prompt carries of the brief, and what it leaves out.
//!
//! The waste this exists to remove is structural rather than accidental: the
//! three stages of a round received **the same** SCOPE block, so each one paid,
//! on every turn, for the parts of the brief the other two act on.
//!
//! Measured on #65 — SCOPE was 60 363 characters, and the body alone 40 367:
//!
//! | section | characters |
//! | --- | --- |
//! | `## Business Goal` | 1 557 |
//! | `## Acceptance Criteria` | 2 238 |
//! | `## Business Rules` | 7 873 |
//! | `## Technical` | 9 306 |
//! | `## Technical Implementation Plan` | 16 861 |
//! | `## Assumptions (autonomous run)` | 2 333 |
//!
//! `technical-refinement` was receiving 26 167 characters of sections **it was
//! about to write**.
//!
//! # What this is not
//!
//! Not a cost optimisation, and it should not be sold as one. The fixed prompt
//! is about 17 000 tokens of a context that averaged 200 000 across #65's
//! `code` stage — the rest is accumulated tool output. Halving SCOPE removes a
//! few percent of the cache read, which is inside the noise of two runs of the
//! same task.
//!
//! What it is: a prompt that stops telling a stage about work that is not its
//! own. The claim is attention and honesty, not dollars, and neither is
//! something this repository can measure.
//!
//! # Why a list of headings and not a list of keys
//!
//! `## Assumptions (autonomous run)` is written by a session and is not one of
//! `common::sections::SECTIONS`. A drop-list of canonical keys could not name
//! it, and the heading text is what the issue actually carries — see
//! [`sections::without`](crate::common::sections::without) for how a name is
//! compared.

/// What a stage's prompt carries of the brief, and which of the body's sections
/// it leaves out.
///
/// The variants mirror [`prompts::Brief`](harness_core::domain::prompts::Brief)
/// — this one adds the sections to drop, which is a decision about stages and
/// therefore cannot live in `core`.
#[derive(Debug, Clone, Copy)]
pub enum Cut {
    /// Nothing: the stage works on no task.
    Unscoped,
    /// The task and the hierarchy around it, minus these headings.
    Situated(&'static [&'static str]),
    /// The task alone, minus these headings.
    TaskOnly(&'static [&'static str]),
}

impl Cut {
    /// The headings dropped from the issue body before it is injected.
    #[must_use]
    pub const fn without(self) -> &'static [&'static str] {
        match self {
            Self::Unscoped => &[],
            Self::Situated(headings) | Self::TaskOnly(headings) => headings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unscoped_stage_names_no_section_to_drop() {
        // There is no body to drop one from: the list has to be empty rather
        // than whatever the previous variant carried.
        assert!(Cut::Unscoped.without().is_empty());
    }

    #[test]
    fn both_scoped_shapes_carry_their_own_drop_list() {
        assert_eq!(Cut::Situated(&["Technical"]).without(), ["Technical"]);
        assert_eq!(
            Cut::TaskOnly(&["Business Goal"]).without(),
            ["Business Goal"]
        );
    }
}
