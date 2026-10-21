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
///
/// The row's id is read from `targetId` or, as the mock and the rules
/// suite send it, from `"target": {"id": …}` (a string or an integer),
/// which wins when both are given, as in the mock; it is written back as
/// `targetId`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "WireSubmission")]
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

/// A submission as the wire spells it: the target's id in either form.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSubmission {
    data_capability: String,
    #[serde(default)]
    payload: BTreeMap<String, Value>,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    target: Option<WireTarget>,
    #[serde(default)]
    based_on: Option<BasedOn>,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    caller: Caller,
}

/// `"target": {"id": …}`; the mock's `aggregate` beside it is ignored, the
/// manifest names it.
#[derive(Deserialize)]
struct WireTarget {
    #[serde(default)]
    id: Option<Value>,
}

impl TryFrom<WireSubmission> for Submission {
    type Error = String;

    fn try_from(wire: WireSubmission) -> Result<Self, Self::Error> {
        let nested = match wire.target.and_then(|target| target.id) {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(id),
            Some(Value::Number(id)) if id.is_i64() || id.is_u64() => Some(id.to_string()),
            Some(other) => {
                return Err(format!("target.id is a string or an integer, not {other}"));
            }
        };
        Ok(Self {
            data_capability: wire.data_capability,
            payload: wire.payload,
            target_id: nested.or(wire.target_id),
            based_on: wire.based_on,
            idempotency_key: wire.idempotency_key,
            caller: wire.caller,
        })
    }
}

/// What became of one follow-up submission ([`crate::plugins::Prepared::then`]).
///
/// On the wire, `{"submitted": {…}}` for one that entered the queue, or
/// `{"refused": {"dataCapability": …, "error": …}}` for one the engine
/// refused, with the message the server would have answered it with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FollowUp {
    /// It entered the queue, or replayed one that had.
    Submitted(Box<Submitted>),
    /// It was refused, and nothing of it queued.
    Refused {
        /// The `DataCapability` it named.
        #[serde(rename = "dataCapability")]
        data_capability: String,
        /// Why.
        error: String,
    },
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
    /// What became of the follow-ups its `Prepare` asked for, in order: one
    /// per follow-up submitted, up to and including the first refused —
    /// those after it are not submitted. Absent from the wire when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub then: Vec<FollowUp>,
}

/// What `POST /capabilities/<id>` carries: the variables, and the hash of
/// the persisted query the Capability reads through when it names one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    /// The query's hash, as the registry has it; absent for a Capability
    /// whose plugin brings its own query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
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
        let json = r#"{"dataCapability":"credit.requestLimitChange@2","payload":{"creditLimit":4500},"targetId":"881","basedOn":{"version":10,"values":{"creditLimit":5000}},"caller":{"by":"carol","from":"credit.LimitReviewForm","permissions":["credit.limit.write"]}}"#;
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
        assert_eq!(request.sha256.as_deref(), Some("ab12"));
        assert_eq!(request.variables["accountId"], "881");
        let bare: ReadRequest = serde_json::from_str(r#"{"sha256":"ab12"}"#).expect("no variables");
        assert!(bare.variables.is_empty(), "expected nothing");
        let unhashed: ReadRequest =
            serde_json::from_str(r#"{"variables":{"accountId":"881"}}"#).expect("no hash");
        assert_eq!(unhashed.sha256, None);
        assert_eq!(
            serde_json::to_value(&unhashed).expect("json"),
            serde_json::json!({ "variables": { "accountId": "881" } }),
            "an absent hash is not sent"
        );
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
                attachment: None,
            },
            then: Vec::new(),
        };
        let json = serde_json::to_value(&answer).expect("json");
        assert_eq!(json["commandId"], "c_1");
        assert_eq!(json["result"]["status"], "queued");
        assert_eq!(json["replayed"], false);
        assert!(json.get("then").is_none(), "no follow-up, not on the wire");
        let followed = Submitted {
            then: vec![
                FollowUp::Submitted(Box::new(Submitted {
                    command_id: "c_2".to_string(),
                    ..answer.clone()
                })),
                FollowUp::Refused {
                    data_capability: "credit.stamp".to_string(),
                    error: "no such account".to_string(),
                },
            ],
            ..answer
        };
        let json = serde_json::to_value(&followed).expect("json");
        assert_eq!(json["then"][0]["submitted"]["commandId"], "c_2");
        assert_eq!(
            json["then"][1],
            serde_json::json!({ "refused": { "dataCapability": "credit.stamp", "error": "no such account" } })
        );
        let back: Submitted = serde_json::from_value(json).expect("read back");
        assert_eq!(back, followed);
    }

    #[test]
    fn a_submission_names_its_row_by_target_id_or_by_target_dot_id() {
        let read = |json: serde_json::Value| serde_json::from_value::<Submission>(json);
        let flat = read(serde_json::json!({ "dataCapability": "x.y", "targetId": "881" }))
            .expect("targetId");
        let nested =
            read(serde_json::json!({ "dataCapability": "x.y", "target": { "id": "881" } }))
                .expect("target.id, as the rules suite sends it");
        assert_eq!(nested, flat);
        let mocked = read(serde_json::json!({
            "dataCapability": "x.y",
            "target": { "aggregate": "Account", "id": 881 },
            "targetId": "5"
        }))
        .expect("the mock's shape, an integer id");
        assert_eq!(mocked.target_id.as_deref(), Some("881"), "target.id wins");
        for absent in [
            serde_json::json!({ "dataCapability": "x.y", "target": {} }),
            serde_json::json!({ "dataCapability": "x.y", "target": { "id": null } }),
        ] {
            assert_eq!(read(absent).expect("no id").target_id, None);
        }
        let kept = read(serde_json::json!({ "dataCapability": "x.y", "target": { "id": null }, "targetId": "7" }))
            .expect("targetId kept");
        assert_eq!(kept.target_id.as_deref(), Some("7"));
        for wrong in [
            serde_json::json!(1.5),
            serde_json::json!(true),
            serde_json::json!(["1"]),
        ] {
            let refused =
                read(serde_json::json!({ "dataCapability": "x.y", "target": { "id": wrong } }));
            assert!(
                refused.as_ref().is_err_and(|e| e
                    .to_string()
                    .contains("target.id is a string or an integer")),
                "{refused:?}"
            );
        }
        let negative = read(serde_json::json!({ "dataCapability": "x.y", "target": { "id": -3 } }))
            .expect("an integer");
        assert_eq!(negative.target_id.as_deref(), Some("-3"));
        assert_eq!(
            serde_json::to_value(&nested).expect("json"),
            serde_json::json!({ "dataCapability": "x.y", "payload": {}, "targetId": "881",
                "caller": serde_json::to_value(Caller::default()).expect("json") }),
            "written back as targetId"
        );
    }
}
