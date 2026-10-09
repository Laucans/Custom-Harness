//! What a data snapshot reads and decides.
//!
//! The archive's index, which heads
//! are new, where a snapshot lands, what the run reports — and the two sides
//! of the data model read and compared (`schema_sql`, `model`), shared with
//! the factory view so the page shows exactly the divergences the watch acts on.

pub mod index;
pub mod model;
pub mod report;
pub mod schema_sql;
