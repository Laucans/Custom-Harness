//! What an evolution's stages pass to each other.

use harness_architecture::graph::{Graph, Kind};
use harness_core::domain::Issue;

use crate::contract_manager::data::request::Request;

/// The state of one evolution.
#[derive(Debug, Clone, Default)]
pub struct ManagerState {
    /// The request issue, set by precheck.
    pub issue: Option<Issue>,
    /// The request its body holds, read by the context stage.
    pub request: Option<Request>,
    /// The plan's graph on the integration branch, when the plan wrote one.
    pub graph: Option<Graph>,
    /// What the graph knows of the id: its kind and the version it is at.
    /// `None` when the graph does not know it — nobody else's contract.
    pub known: Option<(Kind, u32)>,
    /// The milestone that produces the id, when the graph names one.
    pub producer: Option<u64>,
    /// The milestones that consume the id, by number.
    pub consumers: Vec<u64>,
    /// The milestone issue the producer task goes under: the producer's
    /// while it is open, else the requester's.
    pub target_milestone: Option<u64>,
}

impl ManagerState {
    /// The issue, or a programming failure if reached before `precheck`.
    ///
    /// # Panics
    /// If called before `precheck` sets it — something no path in
    /// [`Workflow::execute`](harness_core::execution::Workflow::execute)
    /// allows.
    #[must_use]
    pub const fn issue(&self) -> &Issue {
        self.issue
            .as_ref()
            .expect("precheck must set the issue before the stages run")
    }

    /// The request, or a programming failure if reached before the context
    /// stage read it.
    ///
    /// # Panics
    /// If called before the context stage sets it.
    #[must_use]
    pub const fn request(&self) -> &Request {
        self.request
            .as_ref()
            .expect("the context stage must read the request before the stages that use it")
    }

    /// The version the contract is at, 1 when the graph does not say.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.known.map_or(1, |(_, version)| version)
    }

    /// Whether there is nothing to evolve: an id the graph does not know is
    /// the requester's own to change.
    #[must_use]
    pub const fn nobody_else_s(&self) -> bool {
        self.request.is_some() && self.known.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_knows_nothing_and_an_unknown_id_is_the_requester_s_own() {
        let mut state = ManagerState::default();
        assert!(state.issue.is_none() && state.request.is_none());
        assert_eq!(state.version(), 1);
        assert!(!state.nobody_else_s(), "no request read yet");
        state.request = Some(Request::default());
        assert!(state.nobody_else_s());
        state.known = Some((Kind::Capability, 3));
        assert!(!state.nobody_else_s());
        assert_eq!(state.version(), 3);
    }
}
