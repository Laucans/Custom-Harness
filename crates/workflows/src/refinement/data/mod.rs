//! What the refinement reads and decides: its state, the round counter, the
//! five sections of the body, and the advice it leaves on the issue.
//!
//! Pure business logic — nothing here calls `gh` or writes anywhere. What
//! writes lives in `action`, what judges in `checks`.

pub mod advice;
pub mod phase;
pub mod rounds;
/// The sections of an issue body, now shared: `common::hierarchy` reads a
/// sibling's body with the same model that writes it here.
pub use crate::common::sections;
pub mod state;
