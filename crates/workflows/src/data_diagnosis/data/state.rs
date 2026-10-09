//! What the diagnosis carries from the request to its answer.

use crate::data_snapshot::data::index::Head;

use super::diagnosis::Diagnosis;

/// The state of a diagnosis.
#[derive(Debug, Clone, Default)]
pub struct DiagnosisState {
    /// The branch and the commit whose data model diverges.
    pub head: Head,
    /// The version's folder on the archive branch, checked out in the clone.
    pub dir: String,
    /// What the static comparison found.
    pub divergences: Vec<String>,
    /// The session's answer, once read.
    pub diagnosis: Option<Diagnosis>,
}
