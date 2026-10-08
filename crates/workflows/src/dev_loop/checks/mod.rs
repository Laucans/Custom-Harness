//! What judges without ever writing: a stage's gates, and those of the
//! preflight that runs before the first.
//!
//! The counterpart to `action`: decision #1 says a `Verification` judges and
//! doesn't write, and the borrow checker holds that — what writes lives there.

pub mod architecture;
pub mod gates;
pub mod preflight;
