//! What a data snapshot reads and decides.
//!
//! The archive's index, which heads are new, where a snapshot lands, what the
//! run reports. The two sides of the data model, read and compared, are
//! [`harness_architecture::model`]: the factory view reads the same module, so
//! the page shows exactly the divergences the watch acts on.

pub mod index;
pub mod report;
