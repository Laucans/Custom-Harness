//! The wire protocol, as the mock server of every product already speaks it.
//!
//! `POST /commands` with a submission, `GET /commands/<id>` for its result,
//! `POST /capabilities/<id>` for a read, and the `dataVersion` stream. The
//! engine's methods take and answer these; the product's server binds them
//! to HTTP.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::command::{BasedOn, CommandResult};
use crate::guard::Caller;

/// What `POST /commands` carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Submission {
    /// The `DataCapability`, with or without its `@version`.
    pub data_capability: String,
    /// The fields and their values.
    #[serde(default)]
    pub payload: BTreeMap<String, Value>,
    /// The row's id, when the payload does not name it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// What the author read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<BasedOn>,
    /// The idempotency key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Who triggers, from where, with what rights.
    #[serde(default)]
    pub caller: Caller,
}

/// What `POST /commands` answers (202): the command's place, and its
/// result so far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Submitted {
    /// The command's id.
    pub command_id: String,
    /// `<Aggregate>/<id>`.
    pub partition: String,
    /// True when an idempotency key named a command already submitted:
    /// its result is this one's.
    pub replayed: bool,
    /// What the author should know now: an invariant at risk on the
    /// projected state, a value declared ahead.
    pub warnings: Vec<String>,
    /// Where it stands.
    pub result: CommandResult,
}

/// What `POST /capabilities/<id>` carries: the hash of the persisted query
/// the Capability reads through, and its variables.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    /// The query's hash, as the registry has it.
    pub sha256: String,
    /// The variables.
    #[serde(default)]
    pub variables: BTreeMap<String, Value>,
}

/// What a served read answers: the version it is consistent with, and
/// the data the product's read crate produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResponse {
    /// The `dataVersion` the read saw.
    pub as_of: u64,
    /// The data.
    pub data: Value,
}

/// What `POST /holds` carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldRequest {
    /// `<Aggregate>/<id>`.
    pub partition: String,
    /// Who holds.
    pub by: String,
    /// The `ttl`, as `1m`; the aggregate's maximum when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
}

/// What `POST /reviews/<id>` carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewDecision {
    /// Who decides.
    pub by: String,
    /// Their roles.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Approved or rejected.
    pub approved: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_submission_reads_the_shape_the_mock_server_sends() {
        let json = r#"{"dataCapability":"credit.requestLimitChange@2","payload":{"newLimit":4500},"targetId":"881","basedOn":{"version":10,"values":{"creditLimit":5000}},"caller":{"by":"carol","from":"credit.LimitReviewForm","permissions":["credit.limit.write"]}}"#;
        let submission: Submission = serde_json::from_str(json).expect("the wire shape");
        assert_eq!(submission.data_capability, "credit.requestLimitChange@2");
        assert_eq!(submission.target_id.as_deref(), Some("881"));
        assert_eq!(submission.based_on.as_ref().map(|b| b.version), Some(10));
        assert_eq!(submission.caller.by, "carol");
        assert!(submission.caller.roles.is_empty(), "absent, empty");
        let minimal: Submission =
            serde_json::from_str(r#"{"dataCapability":"x.y"}"#).expect("defaults");
        assert_eq!(minimal.caller, Caller::default());
        let back = serde_json::to_value(&submission).expect("json");
        assert_eq!(back["caller"]["from"], "credit.LimitReviewForm");
    }

    #[test]
    fn a_read_names_the_query_s_hash_and_answers_as_of_a_version() {
        let request: ReadRequest =
            serde_json::from_str(r#"{"sha256":"ab12","variables":{"accountId":"881"}}"#)
                .expect("the wire shape");
        assert_eq!(request.sha256, "ab12");
        assert_eq!(request.variables["accountId"], "881");
        let bare: ReadRequest = serde_json::from_str(r#"{"sha256":"ab12"}"#).expect("no variables");
        assert!(bare.variables.is_empty(), "expected nothing");
        let answer = ReadResponse {
            as_of: 7,
            data: serde_json::json!({ "creditLimit": 5000 }),
        };
        let json = serde_json::to_value(&answer).expect("json");
        assert_eq!(json["asOf"], 7);
        assert_eq!(json["data"]["creditLimit"], 5000);
        let hold: HoldRequest = serde_json::from_str(
            r#"{"partition":"Account/881","by":"credit.limitReview","ttl":"1m"}"#,
        )
        .expect("a hold");
        assert_eq!(hold.ttl.as_deref(), Some("1m"));
        let decision: ReviewDecision =
            serde_json::from_str(r#"{"by":"dana","approved":true}"#).expect("a decision");
        assert!(
            decision.roles.is_empty() && decision.approved,
            "roles default to none"
        );
    }

    #[test]
    fn a_submitted_answer_carries_the_five_fields_of_the_mock() {
        let answer = Submitted {
            command_id: "c_1".to_string(),
            partition: "Account/881".to_string(),
            replayed: false,
            warnings: Vec::new(),
            result: CommandResult {
                command_id: "c_1".to_string(),
                status: crate::command::State::Queued,
                data_version: None,
                violations: Vec::new(),
                review_id: None,
            },
        };
        let json = serde_json::to_value(&answer).expect("json");
        assert_eq!(json["commandId"], "c_1");
        assert_eq!(json["result"]["status"], "queued");
        assert_eq!(json["replayed"], false);
    }
}
