//! What **judges**, never writes: the gate that groups checks, and the
//! guards every stage undergoes regardless of workflow.
//!
//! The counterpart of `action`: decision #1 says a `Verification` judges and
//! never writes, and the borrow checker holds it — what writes lives there.

pub mod gate;
pub mod guards;
