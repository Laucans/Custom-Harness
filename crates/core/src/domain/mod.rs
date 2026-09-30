//! Vocabulaire pur : ni disque, ni subprocess, ni bibliothèque externe.
//!
//! Ne nomme aucun workflow, même en commentaire — c'est ce qui permet à
//! `harness-workflows` de dépendre de ce module sans jamais que la
//! réciproque devienne pensable.

pub mod markers;

mod halt;
mod issue;
mod resumable;
mod spend;
mod verdict;

pub use halt::{Halt, Severity};
pub use issue::Issue;
pub use resumable::Resumable;
pub use spend::{Spend, Tokens};
pub use verdict::{Outcome, Verdict};
