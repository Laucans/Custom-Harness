//! A review (contract K): two values declared for one field, the AI's
//! proposal beside them, and a human's decision — the AI may propose a
//! reconciliation, never decide a critical change alone.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

/// The value the review starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    /// The `dataVersion` it was read at.
    pub version: u64,
    /// The value.
    pub value: Value,
}

/// One value a command declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// The command.
    pub command: String,
    /// Its author.
    pub by: String,
    /// The value.
    pub value: Value,
}

/// What the AI proposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    /// Always `ai`.
    pub by: String,
    /// The value it proposes.
    pub value: Value,
    /// Why, in words.
    pub rationale: String,
    /// The invariants it checked the value against.
    pub invariants_checked: Vec<String>,
}

/// Where the review stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Waiting for the approver.
    Pending,
    /// The approver took the proposal, or one candidate.
    Approved,
    /// The approver refused.
    Rejected,
}

/// The decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionOf {
    /// Where it stands.
    pub status: Status,
    /// The role that decides.
    pub approver: String,
    /// Who decided, once decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// The review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// Its id.
    pub review: String,
    /// `<Aggregate>/<id>`.
    pub aggregate: String,
    /// The field.
    pub field: String,
    /// What it starts from.
    pub base: Base,
    /// The values declared.
    pub candidates: Vec<Candidate>,
    /// What the AI proposes, once it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Proposal>,
    /// The decision.
    pub decision: DecisionOf,
}

/// The AI that proposes a reconciliation — a port: the product wires the
/// model, the engine never calls one itself.
pub trait Proposer: Send + Sync {
    /// A proposal for this review, or none when the AI has nothing to say.
    fn propose(&self, review: &Review) -> Option<Proposal>;
}

/// An AI that proposes nothing: the human sees the candidates alone.
pub struct NoProposer;

impl Proposer for NoProposer {
    fn propose(&self, _review: &Review) -> Option<Proposal> {
        None
    }
}

impl Review {
    /// A pending review of `field` on `aggregate`, for `approver`.
    #[must_use]
    pub fn open(
        id: &str,
        aggregate: &str,
        field: &str,
        base: Base,
        candidates: Vec<Candidate>,
        approver: &str,
    ) -> Self {
        Self {
            review: id.to_string(),
            aggregate: aggregate.to_string(),
            field: field.to_string(),
            base,
            candidates,
            proposal: None,
            decision: DecisionOf {
                status: Status::Pending,
                approver: approver.to_string(),
                by: None,
            },
        }
    }

    /// Asks `proposer` and keeps what it said.
    #[must_use]
    pub fn with_proposal(mut self, proposer: &dyn Proposer) -> Self {
        self.proposal = proposer.propose(&self);
        self
    }

    /// The approver decides. A proposal is never applied by itself: this
    /// is the one gesture that moves a review, and it is a human's.
    ///
    /// # Errors
    ///
    /// [`Error::Forbidden`] when `roles` does not hold the approver role,
    /// [`Error::NotNow`] when the review is already decided.
    pub fn decide(&mut self, by: &str, roles: &[String], approved: bool) -> Result<&DecisionOf> {
        if self.decision.status != Status::Pending {
            return Err(Error::NotNow(format!(
                "review {} is already {:?}",
                self.review, self.decision.status
            )));
        }
        if !roles.contains(&self.decision.approver) {
            return Err(Error::Forbidden(format!(
                "review {} is {}'s to decide, and {by} is not one",
                self.review, self.decision.approver
            )));
        }
        self.decision.status = if approved {
            Status::Approved
        } else {
            Status::Rejected
        };
        self.decision.by = Some(by.to_string());
        Ok(&self.decision)
    }

    /// The value the approval settles on: the proposal's when there is
    /// one, else the last candidate's.
    #[must_use]
    pub fn settled_value(&self) -> Option<Value> {
        self.proposal
            .as_ref()
            .map(|p| p.value.clone())
            .or_else(|| self.candidates.last().map(|c| c.value.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lower;

    impl Proposer for Lower {
        fn propose(&self, review: &Review) -> Option<Proposal> {
            let lowest = review
                .candidates
                .iter()
                .filter_map(|c| c.value.as_i64())
                .min()?;
            Some(Proposal {
                by: "ai".to_string(),
                value: Value::from(lowest),
                rationale: "Risk went HIGH between the two requests".to_string(),
                invariants_checked: vec!["limit-under-ceiling".to_string()],
            })
        }
    }

    fn review() -> Review {
        Review::open(
            "r_45",
            "Account/881",
            "creditLimit",
            Base {
                version: 10,
                value: Value::from(5000),
            },
            vec![
                Candidate {
                    command: "c_120".to_string(),
                    by: "alice".to_string(),
                    value: Value::from(8000),
                },
                Candidate {
                    command: "c_123".to_string(),
                    by: "bob".to_string(),
                    value: Value::from(3000),
                },
            ],
            "role:credit-manager",
        )
    }

    #[test]
    fn the_contract_s_example_the_ai_proposes_and_the_manager_decides() {
        let mut review = review().with_proposal(&Lower);
        let proposal = review.proposal.as_ref().expect("a proposal");
        assert_eq!(proposal.value, Value::from(3000));
        assert_eq!(proposal.by, "ai");
        assert_eq!(review.decision.status, Status::Pending);
        let json = serde_json::to_value(&review).expect("json");
        assert_eq!(
            json["proposal"]["invariantsChecked"],
            serde_json::json!(["limit-under-ceiling"])
        );
        assert_eq!(json["decision"]["status"], "pending");
        let err = review
            .decide("carol", &["role:clerk".to_string()], true)
            .expect_err("not an approver");
        assert!(matches!(err, Error::Forbidden(_)));
        let decided = review
            .decide("dana", &["role:credit-manager".to_string()], true)
            .expect("approved");
        assert_eq!(decided.status, Status::Approved);
        assert_eq!(decided.by.as_deref(), Some("dana"));
        assert_eq!(review.settled_value(), Some(Value::from(3000)));
        assert!(matches!(
            review.decide("dana", &["role:credit-manager".to_string()], false),
            Err(Error::NotNow(_))
        ));
    }

    #[test]
    fn without_a_proposal_the_human_sees_the_candidates_alone() {
        let review = review().with_proposal(&NoProposer);
        assert!(review.proposal.is_none());
        assert_eq!(
            review.settled_value(),
            Some(Value::from(3000)),
            "the last candidate"
        );
        let mut refused = review;
        refused
            .decide("dana", &["role:credit-manager".to_string()], false)
            .expect("rejected");
        assert_eq!(refused.decision.status, Status::Rejected);
    }
}
