//! The Resolver (contract J): what a mutation of existing data reaches —
//! who reads it, who holds it, what points at it — and what to do about
//! it.

use serde::{Deserialize, Serialize};

use crate::aggregate::{Aggregate, OnDelete};
use crate::command::{Effect, Entry};
use crate::guard::Verdict;
use crate::queue::Hold;
use crate::registry::Registry;

/// What the plan ends in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Apply.
    Auto,
    /// Apply once the author confirmed.
    Confirm,
    /// A human decides.
    HumanReview,
    /// Not now: the command keeps its place.
    Blocked,
    /// Never: an invariant or a cascade forbids it.
    Rejected,
}

/// A hold the plan found on the row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldBy {
    /// Who holds.
    pub by: String,
    /// The hold's id.
    pub id: String,
    /// When it expires.
    pub expires: String,
}

/// The dependencies on the row: structural and instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependencies {
    /// The Capabilities that read the fields touched.
    pub readers: Vec<String>,
    /// The holds on the partition.
    pub holds: Vec<HeldBy>,
    /// The commands queued behind on the partition.
    pub pending_commands: Vec<String>,
}

/// One consequence on a relation into the row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cascade {
    /// The relation, as `Payment.accountId`.
    pub relation: String,
    /// How many rows point here.
    pub rows: u64,
    /// What the relation declares.
    pub policy: OnDelete,
    /// True when it alone blocks the command.
    pub blocking: bool,
    /// The link exists outside a foreign key.
    #[serde(default)]
    pub semantic: bool,
}

/// The impact plan (contract J).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactPlan {
    /// The command.
    pub command: String,
    /// Its effect.
    pub classification: Effect,
    /// What depends on the row.
    pub dependencies: Dependencies,
    /// What a deletion reaches.
    pub cascade: Vec<Cascade>,
    /// The derived values to recompute.
    pub invalidates: Vec<String>,
    /// Whether the invariants hold on the final state.
    pub invariants_after: String,
    /// What to do.
    pub decision: Decision,
    /// Why.
    pub reasons: Vec<String>,
}

/// What the plan is computed from, beside the command.
pub struct Situation<'a> {
    /// The aggregate's declaration.
    pub aggregate: &'a Aggregate,
    /// The registry of persisted queries.
    pub registry: &'a Registry,
    /// The holds alive on the partition.
    pub holds: &'a [Hold],
    /// The commands queued behind on the partition.
    pub pending_behind: &'a [String],
    /// The rows that point at the row, per relation: `(from, rows)`.
    pub incoming: &'a [(String, u64)],
    /// What the invariants said of the final state.
    pub verdict: &'a Verdict,
    /// Whether the command is already confirmed by its author, or needs no
    /// confirmation.
    pub confirmed: bool,
}

/// The Capabilities that read what `entry` writes: the readers of its
/// fields, and of the whole aggregate for a deletion.
fn readers(entry: &Entry, situation: &Situation<'_>) -> Vec<String> {
    let aggregate = situation.aggregate;
    let mut readers: Vec<String> = entry
        .fields()
        .iter()
        .flat_map(|field| {
            situation.registry.readers_of(
                &aggregate.aggregate,
                Some(&format!("{}.{field}", aggregate.aggregate)),
            )
        })
        .collect();
    if entry.effect == Effect::Delete {
        readers.extend(situation.registry.readers_of(&aggregate.aggregate, None));
    }
    readers.sort();
    readers.dedup();
    readers
}

/// What a deletion reaches through the aggregate's relations; nothing for
/// another effect.
fn cascade(entry: &Entry, situation: &Situation<'_>) -> Vec<Cascade> {
    if entry.effect != Effect::Delete {
        return Vec::new();
    }
    situation
        .aggregate
        .relations
        .iter()
        .map(|relation| {
            let rows = situation
                .incoming
                .iter()
                .find(|(from, _)| *from == relation.from)
                .map_or(0, |(_, rows)| *rows);
            Cascade {
                relation: relation.from.clone(),
                rows,
                policy: relation.on_delete,
                blocking: relation.on_delete == OnDelete::Restrict && rows > 0,
                semantic: relation.semantic,
            }
        })
        .collect()
}

/// The plan of `entry` in this situation.
#[must_use]
pub fn plan(entry: &Entry, situation: &Situation<'_>) -> ImpactPlan {
    let mut reasons = Vec::new();
    let readers = readers(entry, situation);
    let holds: Vec<HeldBy> = situation
        .holds
        .iter()
        .filter(|h| h.partition == entry.partition && h.position <= entry.position)
        .map(|h| HeldBy {
            by: h.by.clone(),
            id: h.id.clone(),
            expires: h.expires.to_string(),
        })
        .collect();
    for hold in &holds {
        reasons.push(format!("active hold {}", hold.id));
    }
    let cascade = cascade(entry, situation);
    for blocked in cascade.iter().filter(|c| c.blocking) {
        reasons.push(format!(
            "{} restrict",
            blocked.relation.split('.').next().unwrap_or("")
        ));
    }
    let needs_review = cascade
        .iter()
        .any(|c| c.policy == OnDelete::Review && c.rows > 0)
        || cascade.iter().any(|c| c.semantic && c.rows > 0);
    let invalidates: Vec<String> = readers
        .iter()
        .map(|reader| format!("{reader}({})", entry.partition))
        .collect();
    let invariants_after = if situation.verdict.holds() {
        "ok"
    } else {
        "violated"
    };
    let decision = if !situation.verdict.rejecting.is_empty() {
        reasons.extend(
            situation
                .verdict
                .rejecting
                .iter()
                .map(|id| format!("invariant {id}")),
        );
        Decision::Rejected
    } else if !holds.is_empty() || cascade.iter().any(|c| c.blocking) {
        Decision::Blocked
    } else if !situation.verdict.reviewing.is_empty() || needs_review {
        reasons.extend(
            situation
                .verdict
                .reviewing
                .iter()
                .map(|id| format!("invariant {id} wants a review")),
        );
        if needs_review {
            reasons.push("a relation under review".to_string());
        }
        Decision::HumanReview
    } else if situation.confirmed {
        Decision::Auto
    } else {
        Decision::Confirm
    };
    ImpactPlan {
        command: entry.command.clone(),
        classification: entry.effect,
        dependencies: Dependencies {
            readers,
            holds,
            pending_commands: situation.pending_behind.to_vec(),
        },
        cascade,
        invalidates,
        invariants_after: invariants_after.to_string(),
        decision,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{BasedOn, Mode, Projection, State};
    use std::collections::BTreeMap;

    fn aggregate() -> Aggregate {
        Aggregate::parse(
            r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"critical"}},
                "invariants":[],
                "relations":[{"from":"Payment.accountId","onDelete":"restrict"},{"from":"Note.accountId","onDelete":"cascade"},{"from":"Document.subject","onDelete":"review","semantic":true}]}"#,
        )
        .expect("contract I")
    }

    fn registry() -> Registry {
        Registry::parse(
            r#"{"queries":[
              {"query":"R","capability":"customer.calculateRisk","file":"a","sha256":"a","fields":["Account.creditLimit"]},
              {"query":"I","capability":"billing.invoiceList","file":"b","sha256":"b","fields":["Account.contactEmail"]},
              {"query":"T","capability":"crm.timeline","file":"c","sha256":"c","fields":["Note.text"]}
            ]}"#,
        )
        .expect("contract C")
    }

    fn entry(effect: Effect) -> Entry {
        Entry {
            command: "c_201".to_string(),
            data_capability: "credit.archiveCustomer@1".to_string(),
            by: "alice".to_string(),
            partition: "Account/881".to_string(),
            position: 3,
            based_on: BasedOn::default(),
            payload: BTreeMap::from([("creditLimit".to_string(), serde_json::Value::from(1))]),
            effect,
            touches: vec!["Account.creditLimit".to_string()],
            mode: Mode::ConfirmOnStale,
            projection: Projection::default(),
            state: State::Confirmed,
            confirmation: None,
            parked: None,
            at_head_since: None,
            requeued_from: None,
            idempotency_key: None,
            attachment: None,
        }
    }

    #[test]
    fn the_contract_s_example_a_deletion_under_a_hold_and_a_restrict_is_blocked() {
        let aggregate = aggregate();
        let registry = registry();
        let holds = vec![Hold {
            id: "w_77".to_string(),
            partition: "Account/881".to_string(),
            by: "credit.limitReview".to_string(),
            position: 1,
            expires: 9999,
            ttl_seconds: 60,
            warned: false,
        }];
        let incoming = vec![
            ("Payment.accountId".to_string(), 42),
            ("Note.accountId".to_string(), 3),
            ("Document.subject".to_string(), 1),
        ];
        let plan = plan(
            &entry(Effect::Delete),
            &Situation {
                aggregate: &aggregate,
                registry: &registry,
                holds: &holds,
                pending_behind: &["c_198".to_string()],
                incoming: &incoming,
                verdict: &Verdict::default(),
                confirmed: true,
            },
        );
        assert_eq!(plan.decision, Decision::Blocked);
        assert_eq!(plan.reasons, ["active hold w_77", "Payment restrict"]);
        assert_eq!(
            plan.dependencies.readers,
            ["billing.invoiceList", "customer.calculateRisk"],
            "a deletion reaches every reader of the aggregate"
        );
        assert_eq!(plan.dependencies.pending_commands, ["c_198"]);
        assert_eq!(plan.cascade.len(), 3);
        assert!(plan.cascade[0].blocking);
        assert!(!plan.cascade[1].blocking);
        assert!(plan.cascade[2].semantic);
        assert_eq!(plan.invalidates[0], "billing.invoiceList(Account/881)");
        assert_eq!(plan.invariants_after, "ok");
        let json = serde_json::to_value(&plan).expect("json");
        assert_eq!(json["classification"], "delete");
        assert_eq!(json["decision"], "blocked");
        assert_eq!(
            json["dependencies"]["pendingCommands"],
            serde_json::json!(["c_198"])
        );
    }

    #[test]
    fn an_update_of_one_field_reaches_its_readers_only_and_applies_when_confirmed() {
        let aggregate = aggregate();
        let registry = registry();
        let situation = Situation {
            aggregate: &aggregate,
            registry: &registry,
            holds: &[],
            pending_behind: &[],
            incoming: &[],
            verdict: &Verdict::default(),
            confirmed: true,
        };
        let plan = plan(&entry(Effect::Update), &situation);
        assert_eq!(plan.decision, Decision::Auto);
        assert_eq!(plan.dependencies.readers, ["customer.calculateRisk"]);
        assert!(plan.cascade.is_empty(), "expected nothing");
        let unconfirmed = Situation {
            confirmed: false,
            ..situation
        };
        assert_eq!(plan_of(&unconfirmed).decision, Decision::Confirm);
    }

    fn plan_of(situation: &Situation<'_>) -> ImpactPlan {
        plan(&entry(Effect::Update), situation)
    }

    #[test]
    fn a_violated_invariant_rejects_and_a_reviewing_one_or_a_semantic_link_asks_a_human() {
        let aggregate = aggregate();
        let registry = registry();
        let rejecting = Verdict {
            rejecting: vec!["limit-under-ceiling".to_string()],
            reviewing: Vec::new(),
        };
        let base = Situation {
            aggregate: &aggregate,
            registry: &registry,
            holds: &[],
            pending_behind: &[],
            incoming: &[],
            verdict: &rejecting,
            confirmed: true,
        };
        let plan = plan_of(&base);
        assert_eq!(plan.decision, Decision::Rejected);
        assert_eq!(plan.invariants_after, "violated");
        assert_eq!(plan.reasons, ["invariant limit-under-ceiling"]);
        let reviewing = Verdict {
            rejecting: Vec::new(),
            reviewing: vec!["limit-change-requires-risk".to_string()],
        };
        let review = plan_of(&Situation {
            verdict: &reviewing,
            ..base
        });
        assert_eq!(review.decision, Decision::HumanReview);
        let incoming = vec![("Document.subject".to_string(), 1)];
        let semantic = super::plan(
            &entry(Effect::Delete),
            &Situation {
                verdict: &Verdict::default(),
                incoming: &incoming,
                ..base
            },
        );
        assert_eq!(semantic.decision, Decision::HumanReview);
        assert!(
            semantic
                .reasons
                .contains(&"a relation under review".to_string())
        );
    }

    #[test]
    fn a_relation_asks_a_human_when_it_is_under_review_or_semantic_and_has_rows() {
        let aggregate = Aggregate::parse(
            r#"{"aggregate":"Account","fields":{},"invariants":[],
                "relations":[{"from":"Note.accountId","onDelete":"cascade"},{"from":"Memo.subject","onDelete":"review"},{"from":"Tag.target","onDelete":"cascade","semantic":true}]}"#,
        )
        .expect("contract I");
        let registry = Registry::default();
        let verdict = Verdict::default();
        let decide = |incoming: &[(String, u64)]| {
            super::plan(
                &entry(Effect::Delete),
                &Situation {
                    aggregate: &aggregate,
                    registry: &registry,
                    holds: &[],
                    pending_behind: &[],
                    incoming,
                    verdict: &verdict,
                    confirmed: true,
                },
            )
            .decision
        };
        assert_eq!(
            decide(&[("Note.accountId".to_string(), 5)]),
            Decision::Auto,
            "a cascade asks nobody"
        );
        assert_eq!(
            decide(&[("Memo.subject".to_string(), 0)]),
            Decision::Auto,
            "a review of no row"
        );
        assert_eq!(
            decide(&[("Tag.target".to_string(), 0)]),
            Decision::Auto,
            "a semantic link to no row"
        );
        assert_eq!(
            decide(&[("Memo.subject".to_string(), 1)]),
            Decision::HumanReview,
            "under review, one row"
        );
        assert_eq!(
            decide(&[("Tag.target".to_string(), 1)]),
            Decision::HumanReview,
            "semantic, one row"
        );
    }

    #[test]
    fn a_hold_behind_the_command_does_not_block_it() {
        let aggregate = aggregate();
        let registry = registry();
        let behind = vec![Hold {
            id: "w_1".to_string(),
            partition: "Account/881".to_string(),
            by: "x".to_string(),
            position: 9,
            expires: 9999,
            ttl_seconds: 60,
            warned: false,
        }];
        let plan = plan_of(&Situation {
            aggregate: &aggregate,
            registry: &registry,
            holds: &behind,
            pending_behind: &[],
            incoming: &[],
            verdict: &Verdict::default(),
            confirmed: true,
        });
        assert_eq!(plan.decision, Decision::Auto);
        assert!(plan.dependencies.holds.is_empty(), "expected nothing");
    }
}
