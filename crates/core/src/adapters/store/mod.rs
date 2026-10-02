//! Ce que le harness écrit et relit sur le disque : sa comptabilité, et son
//! point de reprise.
//!
//! Les deux survivent au code qu'ils décrivent — un workspace jetable
//! disparaît, la comptabilité reste. C'est ce qui rend un clone jetable.

pub mod checkpoint;
pub mod ledger;
pub mod spending;
