//! What refinement **does**: ask for a stage, record what the router named,
//! rewrite the issue body.
//!
//! The counterpart of `checks`: a `Verification` judges and does not write,
//! so everything that writes lives here — including the "write" half of the
//! former `router_named_sections`.

pub mod actions;
pub mod publish;
