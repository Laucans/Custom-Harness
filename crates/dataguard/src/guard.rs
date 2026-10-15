//! The `DataGuard`'s own judgements, pure.
//!
//! Who may trigger a command, what effect it really has, which invariants
//! hold on a state. A rule an `aggregate.json` writes as
//! `<field> <op> <value>` is judged here, as the mock judges it; any other
//! rule is code the product writes behind a trait, and the two add up.

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

/// Whether a rule of the shape `<field> <op> <value>` holds on `state`.
///
/// `None` when the rule is not of that shape (the product's code judges
/// it) or the field is absent (a rule on a field the command does not
/// set is not its violation). `op` is one of `<=`, `>=`, `<`, `>`, `==`,
/// `=`, `!=`; the value a number, `true`/`false`, or a word, quoted or not.
#[must_use]
pub fn rule_holds(rule: &str, state: &BTreeMap<String, Value>) -> Option<bool> {
    let mut parts = rule.split_whitespace();
    let field = parts.next()?;
    let op = parts.next()?;
    let raw = parts.collect::<Vec<_>>().join(" ");
    if raw.is_empty()
        || !field
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    let left = state.get(field)?;
    let right = literal(&raw);
    let ordering = match (left.as_f64(), right.as_f64()) {
        (Some(a), Some(b)) => a.partial_cmp(&b),
        _ => match (left.as_str(), right.as_str()) {
            (Some(a), Some(b)) => Some(a.cmp(b)),
            _ => None,
        },
    };
    let equal = left == &right;
    Some(match op {
        "==" | "=" => equal,
        "!=" => !equal,
        "<" => ordering.is_some_and(std::cmp::Ordering::is_lt),
        "<=" => ordering.is_some_and(std::cmp::Ordering::is_le),
        ">" => ordering.is_some_and(std::cmp::Ordering::is_gt),
        ">=" => ordering.is_some_and(std::cmp::Ordering::is_ge),
        _ => return None,
    })
}

/// The value a rule's right-hand side names.
fn literal(raw: &str) -> Value {
    if let Ok(n) = raw.parse::<i64>() {
        return Value::from(n);
    }
    if let Ok(n) = raw.parse::<f64>() {
        return Value::from(n);
    }
    match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::from(raw.trim_matches(|c| c == '"' || c == '\'')),
    }
}

/// Runs the aggregate's invariants on `state` — its own rules of the simple
/// shape, then the product's code — and sorts the violations by what the
/// aggregate says of each.
#[must_use]
pub fn check(
    aggregate: &Aggregate,
    invariants: &dyn Invariants,
    state: &BTreeMap<String, Value>,
    entry: &Entry,
) -> Verdict {
    let mut verdict = Verdict::default();
    let declared = aggregate
        .invariants
        .iter()
        .filter(|i| rule_holds(&i.rule, state) == Some(false))
        .map(|i| i.id.clone());
    let coded = invariants.violated(&aggregate.aggregate, state, entry);
    for id in declared.chain(coded) {
        if verdict.rejecting.contains(&id) || verdict.reviewing.contains(&id) {
            continue;
        }
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

    #[test]
    fn a_rule_of_the_simple_shape_is_judged_on_the_state() {
        let state = BTreeMap::from([
            ("creditLimit".to_string(), Value::from(5000)),
            ("status".to_string(), Value::from("open")),
            ("vip".to_string(), Value::Bool(true)),
            ("ratio".to_string(), Value::from(0.5)),
        ]);
        let holds = |rule: &str| rule_holds(rule, &state);
        assert_eq!(holds("creditLimit <= 10000"), Some(true));
        assert_eq!(holds("creditLimit <= 4999"), Some(false));
        assert_eq!(holds("creditLimit < 5000"), Some(false));
        assert_eq!(holds("creditLimit >= 5000"), Some(true));
        assert_eq!(holds("creditLimit > 5000"), Some(false));
        assert_eq!(holds("creditLimit == 5000"), Some(true));
        assert_eq!(holds("creditLimit = 5000"), Some(true));
        assert_eq!(holds("creditLimit != 5000"), Some(false));
        assert_eq!(holds("ratio < 0.75"), Some(true));
        assert_eq!(holds("status == open"), Some(true));
        assert_eq!(holds("status != \"closed\""), Some(true));
        assert_eq!(
            holds("status < closed"),
            Some(false),
            "words order as words"
        );
        assert_eq!(holds("vip == true"), Some(true));
        assert_eq!(holds("vip != false"), Some(true));
        assert_eq!(
            holds("status < 3"),
            Some(false),
            "a word against a number orders nowhere"
        );
        assert_eq!(holds("status == 3"), Some(false));
        assert_eq!(holds("missing <= 1"), None, "a field the state lacks");
        assert_eq!(holds("creditLimit ~ 1"), None, "an unknown operator");
        assert_eq!(holds("creditLimit <="), None, "no value");
        assert_eq!(
            holds("the limit stays under the ceiling"),
            None,
            "words: the product's code"
        );
        assert_eq!(holds("credit-limit <= 1"), None, "not a field name");
        assert_eq!(holds(""), None);
    }

    /// A product's own rule, beside the aggregate's.
    struct Coded;

    impl Invariants for Coded {
        fn violated(
            &self,
            _aggregate: &str,
            state: &BTreeMap<String, Value>,
            _entry: &Entry,
        ) -> Vec<String> {
            let limit = state
                .get("creditLimit")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let mut found = vec!["unknown-to-the-aggregate".to_string()];
            if limit > 7000 {
                found.push("risk-fresh".to_string());
            }
            if limit > 10_000 {
                found.push("limit-under-ceiling".to_string());
            }
            found
        }
    }

    #[test]
    fn declared_rules_and_the_product_s_code_add_up_once_each() {
        let aggregate = Aggregate::parse(
            r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"normal"}},"invariants":[{"id":"limit-under-ceiling","rule":"creditLimit <= 10000","onViolation":"reject"},{"id":"risk-fresh","rule":"the risk score is younger than the raise","onViolation":"human_review"},{"id":"positive","rule":"creditLimit >= 0","onViolation":"reject"}]}"#,
        )
        .expect("contract I");
        let state = |limit: i64| BTreeMap::from([("creditLimit".to_string(), Value::from(limit))]);
        let fine = check(
            &aggregate,
            &Coded,
            &state(5000),
            &entry(Mode::Overwrite, "creditLimit", 5000),
        );
        assert!(fine.holds(), "{fine:?}");
        let risky = check(
            &aggregate,
            &Coded,
            &state(8000),
            &entry(Mode::Overwrite, "creditLimit", 8000),
        );
        assert_eq!(risky.reviewing, ["risk-fresh"], "the product's code");
        assert!(risky.rejecting.is_empty(), "expected nothing");
        let over = check(
            &aggregate,
            &Coded,
            &state(20_000),
            &entry(Mode::Overwrite, "creditLimit", 20_000),
        );
        assert_eq!(
            over.rejecting,
            ["limit-under-ceiling"],
            "declared and coded alike, once"
        );
        assert_eq!(over.reviewing, ["risk-fresh"]);
        let negative = check(
            &aggregate,
            &NoInvariants,
            &state(-1),
            &entry(Mode::Overwrite, "creditLimit", -1),
        );
        assert_eq!(negative.rejecting, ["positive"], "the declared rule alone");
    }

    proptest::proptest! {
        #[test]
        fn any_rule_is_judged_or_left_to_the_product_without_a_panic(rule in "[a-zA-Z0-9_. <>=!~\"'-]{0,24}", limit in -20_000i64..20_000) {
            let state = BTreeMap::from([("creditLimit".to_string(), Value::from(limit))]);
            let _ = rule_holds(&rule, &state);
            let ceiling = rule_holds("creditLimit <= 10000", &state);
            proptest::prop_assert_eq!(ceiling, Some(limit <= 10_000));
        }
    }
}
