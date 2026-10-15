//! An aggregate as its `aggregate.json` declares it (contract I): its
//! fields and their criticality, its invariants, its relations, its holds.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Derived or authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Recomputed from other rows, never mutated.
    Derived,
    /// Canonical, mutated only through a `DataCapability`.
    Authoritative,
}

/// How much a field's change matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Criticality {
    /// Concurrent writes merge.
    Low,
    /// The default.
    Normal,
    /// An approver reviews every change.
    Critical,
}

/// One field of the aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Derived or authoritative.
    pub kind: Kind,
    /// How much a change matters.
    pub criticality: Criticality,
    /// The approver role a critical field requires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    /// How concurrent writes to a low-criticality field merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<String>,
}

/// What happens when an invariant does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnViolation {
    /// The command is rejected.
    Reject,
    /// A human decides.
    HumanReview,
}

/// One rule the aggregate keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invariant {
    /// Its id, what a `DataCapability` lists and a test names.
    pub id: String,
    /// The rule, in words.
    pub rule: String,
    /// What a violation does.
    #[serde(rename = "onViolation")]
    pub on_violation: OnViolation,
}

/// What a deletion does to a row that points here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnDelete {
    /// The deletion is refused while a row points here.
    Restrict,
    /// The pointing rows go too.
    Cascade,
    /// The pointing rows lose their link.
    Nullify,
    /// The pointing rows are archived.
    Archive,
    /// A human decides.
    Review,
}

/// One relation into the aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    /// `<Aggregate>.<field>` that points here.
    pub from: String,
    /// What a deletion does.
    #[serde(rename = "onDelete")]
    pub on_delete: OnDelete,
    /// What an update does: `propagate` or `restrict`.
    #[serde(default, rename = "onUpdate", skip_serializing_if = "Option::is_none")]
    pub on_update: Option<String>,
    /// The link exists outside a foreign key.
    #[serde(default)]
    pub semantic: bool,
}

/// How long a workflow may freeze the partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holds {
    /// The longest `ttl`, as `1m`, `24h`.
    #[serde(rename = "maxTtl")]
    pub max_ttl: String,
    /// Whether a hold may be renewed.
    pub renewable: bool,
}

/// The aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aggregate {
    /// Its name: `Account`.
    pub aggregate: String,
    /// Its fields.
    pub fields: BTreeMap<String, Field>,
    /// Its invariants.
    #[serde(default)]
    pub invariants: Vec<Invariant>,
    /// The relations into it.
    #[serde(default)]
    pub relations: Vec<Relation>,
    /// Its holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holds: Option<Holds>,
}

/// A duration as the contracts spell it: `5m`, `24h`, `500ms`.
///
/// `None` when the text is not one.
#[must_use]
pub fn duration_seconds(text: &str) -> Option<u64> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let unit = &text[digits.len()..];
    let number: u64 = digits.parse().ok()?;
    match unit {
        "ms" => Some(number / 1000),
        "s" => Some(number),
        "m" => number.checked_mul(60),
        "h" => number.checked_mul(3600),
        "d" => number.checked_mul(86_400),
        _ => None,
    }
}

impl Aggregate {
    /// Reads an `aggregate.json`.
    ///
    /// # Errors
    ///
    /// The text is not the contract's JSON.
    pub fn parse(json: &str) -> std::result::Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The critical fields among `fields`, with the approver each requires.
    #[must_use]
    pub fn critical_among(&self, fields: &[String]) -> Vec<(String, Option<String>)> {
        fields
            .iter()
            .filter_map(|name| {
                let field = self.fields.get(name)?;
                (field.criticality == Criticality::Critical)
                    .then(|| (name.clone(), field.review.clone()))
            })
            .collect()
    }

    /// The invariant with this id.
    #[must_use]
    pub fn invariant(&self, id: &str) -> Option<&Invariant> {
        self.invariants.iter().find(|i| i.id == id)
    }

    /// The longest hold, in seconds: one minute when the aggregate says
    /// nothing (the architecture's default).
    #[must_use]
    pub fn max_hold_seconds(&self) -> u64 {
        self.holds
            .as_ref()
            .and_then(|h| duration_seconds(&h.max_ttl))
            .unwrap_or(60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: &str = r#"{
      "aggregate": "Account",
      "fields": {
        "creditLimit": { "kind": "authoritative", "criticality": "critical", "review": "role:credit-manager" },
        "contactEmail": { "kind": "authoritative", "criticality": "normal" },
        "tags": { "kind": "authoritative", "criticality": "low", "merge": "set" }
      },
      "invariants": [
        { "id": "limit-under-ceiling", "rule": "creditLimit <= policy.maxLimit(riskLevel)", "onViolation": "reject" },
        { "id": "limit-change-requires-risk", "rule": "Risk@3 computed less than 24h ago", "onViolation": "human_review" }
      ],
      "relations": [
        { "from": "Payment.accountId", "onDelete": "restrict", "onUpdate": "propagate" },
        { "from": "Document.subject", "semantic": true, "onDelete": "review" }
      ],
      "holds": { "maxTtl": "1m", "renewable": true }
    }"#;

    #[test]
    fn the_contract_s_example_parses_whole() {
        let account = Aggregate::parse(ACCOUNT).expect("contract I");
        assert_eq!(account.aggregate, "Account");
        assert_eq!(account.fields.len(), 3);
        assert_eq!(account.invariants.len(), 2);
        assert_eq!(account.relations[1].on_delete, OnDelete::Review);
        assert!(account.relations[1].semantic);
        assert_eq!(account.max_hold_seconds(), 60);
        assert_eq!(
            account
                .invariant("limit-under-ceiling")
                .map(|i| i.on_violation),
            Some(OnViolation::Reject)
        );
    }

    #[test]
    fn the_critical_fields_name_their_approver() {
        let account = Aggregate::parse(ACCOUNT).expect("contract I");
        let critical = account.critical_among(&["creditLimit".to_string(), "tags".to_string()]);
        assert_eq!(
            critical,
            [(
                "creditLimit".to_string(),
                Some("role:credit-manager".to_string())
            )]
        );
    }

    #[test]
    fn durations_read_as_the_contracts_spell_them() {
        assert_eq!(duration_seconds("1m"), Some(60));
        assert_eq!(duration_seconds("24h"), Some(86_400));
        assert_eq!(duration_seconds("2d"), Some(172_800));
        assert_eq!(duration_seconds("500ms"), Some(0));
        assert_eq!(duration_seconds("5s"), Some(5));
        assert_eq!(duration_seconds("soon"), None);
        assert_eq!(duration_seconds("5"), None);
    }

    #[test]
    fn an_aggregate_without_holds_freezes_for_a_minute() {
        let bare = Aggregate::parse(
            r#"{"aggregate":"X","fields":{"a":{"kind":"derived","criticality":"low"}}}"#,
        )
        .expect("parses");
        assert_eq!(bare.max_hold_seconds(), 60);
        assert!(
            bare.critical_among(&["a".to_string()]).is_empty(),
            "expected nothing"
        );
    }
}
