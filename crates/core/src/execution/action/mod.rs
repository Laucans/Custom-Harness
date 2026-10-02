//! What a level **writes**: the local/session action shapes, and the one
//! action every paid stage shares.
//!
//! The counterpart of `checks`: decision #1 says a `Verification` judges and
//! never writes, so everything that writes lives here instead.

pub mod ask;
pub mod kinds;
