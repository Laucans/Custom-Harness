//! What the Integrity `DataGuard` tells the authors and the infrastructure
//! (contract L), and the port it tells it through.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What an author may do about a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Accept to overwrite what is declared ahead.
    ConfirmOverwrite,
    /// Withdraw the command.
    Cancel,
    /// Change the command's value.
    Edit,
    /// Put a parked command back, at the end, on a new base.
    Requeue,
    /// Renew a hold.
    Renew,
    /// Approve a review.
    Approve,
    /// Reject a review.
    Reject,
}

/// What was declared ahead of a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredAhead {
    /// The value.
    pub value: Value,
    /// Its author.
    pub by: String,
    /// Its command.
    pub command: String,
}

/// A hold, as a message names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldRef {
    /// Its id.
    pub id: String,
    /// Who holds.
    pub by: String,
    /// When it expires, as an instant in seconds since the epoch.
    pub expires: String,
}

/// The twelve messages of contract L.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// At enqueue.
    CommandQueued,
    /// The projected state violates an invariant.
    InvariantAtRisk,
    /// A value is declared ahead of the author's command.
    ValueDeclaredAhead,
    /// The partition is frozen.
    BlockedByHold,
    /// Before the end of a hold's `ttl`.
    HoldExpiring,
    /// A critical field or a plan in `human_review`.
    ReviewRequired,
    /// Reached the head unconfirmed.
    Parked,
    /// Before the end of a parked command's `ttl`.
    ParkedExpiring,
    /// A parked command withdrawn.
    Expired,
    /// A command ahead was applied or cancelled.
    AheadResolved,
    /// Applied; invalidates caches.
    CommandApplied,
    /// Rejected at the head, with the reason.
    CommandRejected,
}

/// A message (contract L).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Which one.
    pub message: Kind,
    /// Who it is for: authors, approver roles, infrastructure components.
    pub to: Vec<String>,
    /// The command it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The field, as `Account/881.creditLimit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The author's own value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub your_value: Option<Value>,
    /// What is declared ahead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_ahead: Option<DeclaredAhead>,
    /// The hold concerned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<HoldRef>,
    /// The version an application took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_version: Option<u64>,
    /// The invariants violated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<String>,
    /// Why, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What the recipient may do.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

impl Message {
    /// A message of this kind to these recipients about this command, with
    /// nothing else yet.
    #[must_use]
    pub fn new(kind: Kind, to: &[&str], command: &str) -> Self {
        Self {
            message: kind,
            to: to.iter().map(|t| (*t).to_string()).collect(),
            command: Some(command.to_string()),
            field: None,
            your_value: None,
            declared_ahead: None,
            hold: None,
            data_version: None,
            violations: Vec::new(),
            reason: None,
            actions: Vec::new(),
        }
    }
}

/// Where the messages go: one subscription per author, and the
/// infrastructure components that invalidate on `CommandApplied`.
pub trait Notifier: Send + Sync {
    /// Sends `message` to each of its recipients.
    fn notify(&self, message: &Message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contract_s_example_serialises_as_it_is_written() {
        let message = Message {
            field: Some("Account/881.creditLimit".to_string()),
            your_value: Some(Value::from(3000)),
            declared_ahead: Some(DeclaredAhead {
                value: Value::from(8000),
                by: "alice".to_string(),
                command: "c_120".to_string(),
            }),
            actions: vec![Action::ConfirmOverwrite, Action::Cancel, Action::Edit],
            ..Message::new(Kind::ValueDeclaredAhead, &["bob"], "c_123")
        };
        let json = serde_json::to_value(&message).expect("json");
        assert_eq!(json["message"], "ValueDeclaredAhead");
        assert_eq!(json["to"], serde_json::json!(["bob"]));
        assert_eq!(json["yourValue"], 3000);
        assert_eq!(json["declaredAhead"]["by"], "alice");
        assert_eq!(
            json["actions"],
            serde_json::json!(["confirm_overwrite", "cancel", "edit"])
        );
        assert!(json.get("hold").is_none(), "absent, not null");
        assert!(json.get("violations").is_none(), "empty, left out");
    }
}
