//! The `DataGuard`'s own judgements: who may trigger a command, what effect
//! it really has, which invariants hold on a state. Pure; the invariants'
//! code is the product's, behind a trait.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::aggregate::{Aggregate, OnViolation};
use crate::command::Entry;
use crate::manifest::DataCapability;
use crate::store::Collision;
use crate::{Error, Result};

/// The product's invariants, executed: the rules an `aggregate.json`
/// declares in words are code here, one implementation per product.
pub trait Invariants: Send + Sync {
    /// The ids of the invariants of `aggregate` that `state` — the row's
    /// fields after the command — violates. Only the ids the aggregate
    /// declares count; an id it does not declare is ignored.
    fn violated(
        &self,
        aggregate: &str,
        state: &BTreeMap<String, Value>,
        entry: &Entry,
    ) -> Vec<String>;
}

/// Invariants that always hold: what a product runs before it writes any.
pub struct NoInvariants;

impl Invariants for NoInvariants {
    fn violated(
        &self,
        _aggregate: &str,
        _state: &BTreeMap<String, Value>,
        _entry: &Entry,
    ) -> Vec<String> {
        Vec::new()
    }
}

/// Who triggers a command, and with what rights.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caller {
    /// The author, who hears the messages.
    #[serde(default)]
    pub by: String,
    /// The Micro-UI or system that triggers: what `callableBy` admits.
    #[serde(default)]
    pub from: String,
    /// The permissions the author holds.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// The roles the author holds: what a critical field's review needs.
    #[serde(default)]
    pub roles: Vec<String>,
}

/// Authorisation: the caller is admitted and holds every permission.
///
/// # Errors
///
/// [`Error::Forbidden`], naming what is missing.
pub fn authorise(manifest: &DataCapability, caller: &Caller) -> Result<()> {
    if !manifest.admits(&caller.from) {
        return Err(Error::Forbidden(format!(
            "{} is not callable by {} (callableBy: {})",
            manifest.data_capability,
            caller.from,
            manifest.callable_by.join(", ")
        )));
    }
    let missing = manifest.missing_permissions(&caller.permissions);
    if !missing.is_empty() {
        return Err(Error::Forbidden(format!(
            "{} needs the permission(s) {} — {} lacks them",
            manifest.data_capability,
            missing.join(", "),
            caller.by
        )));
    }
    Ok(())
}

/// What the effect of a command turns out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    /// A pure addition: the fast path, no Resolver.
    PureAddition,
    /// A mutation of existing data: the Resolver's.
    Mutation,
    /// Declared a pure addition, found not to be one at execution: routed
    /// to the Resolver instead of failing, and the reason kept.
    Reclassified(Collision),
}

/// The classification: static from the manifest, then dynamic from what
/// the store found at execution.
#[must_use]
pub fn classify(manifest: &DataCapability, collision: &Collision) -> Classification {
    if !manifest.is_pure_addition() {
        return Classification::Mutation;
    }
    if collision.is_clean() {
        Classification::PureAddition
    } else {
        Classification::Reclassified(collision.clone())
    }
}

/// The row after the command: the confirmed row, the command's payload on
/// top — a relative command adds its numbers instead of replacing them.
#[must_use]
pub fn projected(row: &BTreeMap<String, Value>, entry: &Entry) -> BTreeMap<String, Value> {
    let mut state = row.clone();
    for (field, value) in &entry.payload {
        let combined = match (entry.mode, state.get(field), value) {
            (crate::command::Mode::Relative, Some(Value::Number(now)), Value::Number(delta)) => {
                match (now.as_i64(), delta.as_i64()) {
                    (Some(a), Some(b)) => Value::from(a.saturating_add(b)),
                    _ => now
                        .as_f64()
                        .zip(delta.as_f64())
                        .map_or_else(|| value.clone(), |(a, b)| Value::from(a + b)),
                }
            }
            _ => value.clone(),
        };
        state.insert(field.clone(), combined);
    }
    state
}

/// What the invariants say of a state: the violated ids, split by what
/// their violation does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Violated invariants that reject the command.
    pub rejecting: Vec<String>,
    /// Violated invariants that send it to a human.
    pub reviewing: Vec<String>,
}

impl Verdict {
    /// True when nothing is violated.
    #[must_use]
    pub const fn holds(&self) -> bool {
        self.rejecting.is_empty() && self.reviewing.is_empty()
    }
}

/// Runs the aggregate's invariants on `state` and sorts the violations by
/// what the aggregate says of each.
#[must_use]
pub fn check(
    aggregate: &Aggregate,
    invariants: &dyn Invariants,
    state: &BTreeMap<String, Value>,
    entry: &Entry,
) -> Verdict {
    let mut verdict = Verdict::default();
    for id in invariants.violated(&aggregate.aggregate, state, entry) {
        match aggregate.invariant(&id).map(|i| i.on_violation) {
            Some(OnViolation::Reject) => verdict.rejecting.push(id),
            Some(OnViolation::HumanReview) => verdict.reviewing.push(id),
            None => {}
        }
    }
    verdict
}

/// The approver roles a command's critical fields require, and whether
/// the caller holds them all.
#[must_use]
pub fn review_needed(aggregate: &Aggregate, entry: &Entry, caller: &Caller) -> Vec<String> {
    aggregate
        .critical_among(&entry.fields())
        .into_iter()
        .filter_map(|(field, role)| {
            let role = role.unwrap_or_else(|| "role:approver".to_string());
            (!caller.roles.contains(&role)).then_some(format!("{field} ({role})"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{BasedOn, Effect, Mode, Projection, State};

    fn manifest() -> DataCapability {
        DataCapability::parse(
            r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard","version":2,"effect":"update","target":{"aggregate":"Account"},"touches":["Account.creditLimit"],"permissions":["credit.limit.write"],"callableBy":["credit.*"]}"#,
        )
        .expect("contract F")
    }

    fn aggregate() -> Aggregate {
        Aggregate::parse(
            r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"critical","review":"role:credit-manager"},"tags":{"kind":"authoritative","criticality":"low"}},"invariants":[{"id":"limit-under-ceiling","rule":"<= 10000","onViolation":"reject"},{"id":"limit-change-requires-risk","rule":"risk fresh","onViolation":"human_review"}]}"#,
        )
        .expect("contract I")
    }

    fn entry(mode: Mode, field: &str, value: i64) -> Entry {
        Entry {
            command: "c".to_string(),
            data_capability: "credit.requestLimitChange@2".to_string(),
            by: "carol".to_string(),
            partition: "Account/881".to_string(),
            position: 0,
            based_on: BasedOn::default(),
            payload: BTreeMap::from([(field.to_string(), Value::from(value))]),
            effect: Effect::Update,
            touches: vec![format!("Account.{field}")],
            mode,
            projection: Projection::default(),
            state: State::Queued,
            confirmation: None,
            parked: None,
            requeued_from: None,
            idempotency_key: None,
        }
    }

    struct Ceiling;

    impl Invariants for Ceiling {
        fn violated(
            &self,
            _aggregate: &str,
            state: &BTreeMap<String, Value>,
            _entry: &Entry,
        ) -> Vec<String> {
            let mut violated = Vec::new();
            if state
                .get("creditLimit")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                > 10_000
            {
                violated.push("limit-under-ceiling".to_string());
            }
            if state
                .get("riskStale")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                violated.push("limit-change-requires-risk".to_string());
            }
            violated.push("not-declared".to_string());
            violated
        }
    }

    #[test]
    fn a_caller_is_refused_by_namespace_or_by_permission() {
        let admitted = Caller {
            by: "carol".to_string(),
            from: "credit.LimitReviewForm".to_string(),
            permissions: vec!["credit.limit.write".to_string()],
            roles: Vec::new(),
        };
        authorise(&manifest(), &admitted).expect("admitted");
        let elsewhere = Caller {
            from: "orders.Form".to_string(),
            ..admitted.clone()
        };
        let err = authorise(&manifest(), &elsewhere).expect_err("refused");
        assert!(
            err.to_string().contains("not callable by orders.Form"),
            "{err}"
        );
        let unpermitted = Caller {
            permissions: Vec::new(),
            ..admitted
        };
        let err = authorise(&manifest(), &unpermitted).expect_err("refused");
        assert!(err.to_string().contains("credit.limit.write"), "{err}");
    }

    #[test]
    fn the_effect_is_classified_statically_then_dynamically() {
        assert_eq!(
            classify(&manifest(), &Collision::default()),
            Classification::Mutation
        );
        let add = DataCapability::parse(
            r#"{"dataCapability":"credit.addPayment","owner":"dataguard","version":1,"effect":"insert","target":{"aggregate":"Payment"}}"#,
        )
        .expect("a pure insert");
        assert_eq!(
            classify(&add, &Collision::default()),
            Classification::PureAddition
        );
        let collision = Collision {
            unique_key: Some("Payment.reference".to_string()),
            ..Collision::default()
        };
        assert_eq!(
            classify(&add, &collision),
            Classification::Reclassified(collision.clone()),
            "routed to the Resolver, not failed"
        );
    }

    #[test]
    fn the_projected_state_replaces_or_adds_as_the_mode_says() {
        let row = BTreeMap::from([("stock".to_string(), Value::from(10))]);
        assert_eq!(
            projected(&row, &entry(Mode::Relative, "stock", -2))["stock"],
            Value::from(8)
        );
        assert_eq!(
            projected(&row, &entry(Mode::Overwrite, "stock", 7))["stock"],
            Value::from(7)
        );
        assert_eq!(
            projected(&row, &entry(Mode::Relative, "fresh", 3))["fresh"],
            Value::from(3),
            "nothing to add to: set"
        );
        let floats = BTreeMap::from([("stock".to_string(), Value::from(1.5))]);
        assert_eq!(
            projected(&floats, &entry(Mode::Relative, "stock", 2))["stock"],
            Value::from(3.5)
        );
    }

    #[test]
    fn the_verdict_sorts_violations_by_what_the_aggregate_says_and_ignores_the_rest() {
        let aggregate = aggregate();
        let fine = check(
            &aggregate,
            &Ceiling,
            &BTreeMap::new(),
            &entry(Mode::Overwrite, "creditLimit", 1),
        );
        assert!(fine.holds(), "{fine:?}");
        let state = BTreeMap::from([
            ("creditLimit".to_string(), Value::from(20_000)),
            ("riskStale".to_string(), Value::from(true)),
        ]);
        let verdict = check(
            &aggregate,
            &Ceiling,
            &state,
            &entry(Mode::Overwrite, "creditLimit", 20_000),
        );
        assert_eq!(verdict.rejecting, ["limit-under-ceiling"]);
        assert_eq!(verdict.reviewing, ["limit-change-requires-risk"]);
        assert!(!verdict.holds());
        assert!(
            NoInvariants
                .violated("X", &state, &entry(Mode::Overwrite, "x", 1))
                .is_empty(),
            "expected nothing"
        );
    }

    #[test]
    fn a_critical_field_needs_its_approver_unless_the_caller_is_one() {
        let aggregate = aggregate();
        let carol = Caller {
            by: "carol".to_string(),
            ..Caller::default()
        };
        assert_eq!(
            review_needed(
                &aggregate,
                &entry(Mode::Overwrite, "creditLimit", 1),
                &carol
            ),
            ["creditLimit (role:credit-manager)"]
        );
        assert!(
            review_needed(&aggregate, &entry(Mode::Overwrite, "tags", 1), &carol).is_empty(),
            "expected nothing"
        );
        let manager = Caller {
            roles: vec!["role:credit-manager".to_string()],
            ..carol
        };
        assert!(
            review_needed(
                &aggregate,
                &entry(Mode::Overwrite, "creditLimit", 1),
                &manager
            )
            .is_empty(),
            "expected nothing"
        );
    }
}
