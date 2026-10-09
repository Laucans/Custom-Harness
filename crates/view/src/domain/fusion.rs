//! Could two units of the capabilities list be one? The comparison is
//! deterministic — kinds, systems, Concepts, contracts, business rules,
//! callers — and only its result goes to the model, which words the answer:
//! a few hundred tokens, never the repository.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::domain::capabilities::{Contract, Entry, Kind, Status};

/// The comparison's conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Same contract, same rules: one is a duplicate of the other.
    Possible,
    /// One unit can serve both once its contract is widened or adapted.
    Adaptable,
    /// The architecture forbids it, or their meaning differs.
    Impossible,
}

/// How much a difference weighs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Weight {
    /// Worth knowing; changes nothing.
    Note,
    /// The merged unit's contract must absorb it.
    Adapts,
    /// It forbids the merge.
    Blocks,
}

/// One aspect where the two differ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Difference {
    /// `input`, `concept`, `business rules`…
    pub aspect: String,
    /// The first unit's side.
    pub a: String,
    /// The second unit's side.
    pub b: String,
    /// How much it weighs.
    pub weight: Weight,
    /// Why, in a sentence.
    pub why: String,
}

/// What the tooling concludes before any model is asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comparison {
    /// The first unit's id.
    pub a: String,
    /// The second unit's id.
    pub b: String,
    /// The conclusion.
    pub verdict: Verdict,
    /// The aspects they share.
    pub same: Vec<String>,
    /// The aspects where they differ, heaviest first.
    pub differences: Vec<Difference>,
    /// The Micro-UIs that would have to point at the merged unit.
    pub callers: Vec<String>,
}

struct Builder {
    same: Vec<String>,
    differences: Vec<Difference>,
}

impl Builder {
    fn differ(&mut self, aspect: &str, a: String, b: String, weight: Weight, why: &str) {
        self.differences.push(Difference {
            aspect: aspect.to_string(),
            a,
            b,
            weight,
            why: why.to_string(),
        });
    }

    /// Two optional values: equal is shared, else `weight`.
    fn option(
        &mut self,
        aspect: &str,
        a: Option<&String>,
        b: Option<&String>,
        weight: Weight,
        why: &str,
    ) {
        if a == b {
            if a.is_some() {
                self.same.push(aspect.to_string());
            }
        } else {
            let show = |v: Option<&String>| v.cloned().unwrap_or_else(|| "—".to_string());
            self.differ(aspect, show(a), show(b), weight, why);
        }
    }

    /// Two lists compared as sets.
    fn set(&mut self, aspect: &str, a: &[String], b: &[String], weight: Weight, why: &str) {
        let left: BTreeSet<&String> = a.iter().collect();
        let right: BTreeSet<&String> = b.iter().collect();
        if left == right {
            if !left.is_empty() {
                self.same.push(aspect.to_string());
            }
            return;
        }
        let only = |x: &BTreeSet<&String>, y: &BTreeSet<&String>| {
            let rest: Vec<&str> = x.difference(y).map(|s| s.as_str()).collect();
            if rest.is_empty() {
                "—".to_string()
            } else {
                format!("only: {}", rest.join(", "))
            }
        };
        self.differ(
            aspect,
            only(&left, &right),
            only(&right, &left),
            weight,
            why,
        );
    }

    /// Two `{ field: type }` contracts: a field one lacks widens the merged
    /// contract; a field typed two ways needs an adapter.
    fn shape(&mut self, aspect: &str, a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) {
        if a == b {
            if !a.is_empty() {
                self.same.push(aspect.to_string());
            }
            return;
        }
        let clash: Vec<String> = a
            .iter()
            .filter_map(|(k, ta)| {
                b.get(k)
                    .filter(|tb| !tb.eq_ignore_ascii_case(ta))
                    .map(|tb| format!("{k}: {ta} / {tb}"))
            })
            .collect();
        let show = |x: &BTreeMap<String, String>, y: &BTreeMap<String, String>| {
            let rest: Vec<String> = x
                .iter()
                .filter(|(k, _)| !y.contains_key(*k))
                .map(|(k, t)| format!("{k}: {t}"))
                .collect();
            if rest.is_empty() {
                "—".to_string()
            } else {
                format!("only: {}", rest.join(", "))
            }
        };
        if !clash.is_empty() {
            self.differ(
                aspect,
                clash.join("; "),
                String::new(),
                Weight::Adapts,
                "the same field is typed two ways: an adapter converts one into the other",
            );
        }
        let (left, right) = (show(a, b), show(b, a));
        if left != "—" || right != "—" {
            self.differ(
                aspect,
                left,
                right,
                Weight::Adapts,
                "the merged contract takes the union; the fields one side lacks become optional",
            );
        }
    }
}

/// Two Capabilities: meaning, contract, dataview.
fn reads(out: &mut Builder, ca: &Contract, cb: &Contract) {
    let concept_weight = if ca.implements.is_some() && cb.implements.is_some() {
        Weight::Blocks
    } else {
        Weight::Adapts
    };
    out.option(
        "concept",
        ca.implements.as_ref(),
        cb.implements.as_ref(),
        concept_weight,
        "two Concepts are two meanings; a unit without one would adopt the other's",
    );
    out.shape("input", &ca.input, &cb.input);
    out.option(
        "output",
        ca.output.as_ref(),
        cb.output.as_ref(),
        Weight::Adapts,
        "the merged unit returns both shapes, or the callers adapt to one",
    );
    out.set(
        "reads (dataview)",
        &ca.reads,
        &cb.reads,
        Weight::Note,
        "the merged unit reads through both persisted queries",
    );
}

/// Two `DataCapabilities`: what they write, how, under which rights.
fn writes(out: &mut Builder, ca: &Contract, cb: &Contract) {
    out.option(
        "target",
        ca.target.as_ref(),
        cb.target.as_ref(),
        Weight::Blocks,
        "a command writes one aggregate",
    );
    out.option(
        "effect",
        ca.effect.as_ref(),
        cb.effect.as_ref(),
        Weight::Blocks,
        "a command declares one effect",
    );
    out.set(
        "touches",
        &ca.touches,
        &cb.touches,
        Weight::Adapts,
        "the merged command touches the union; touching more takes it off the fast path",
    );
    out.shape("payload", &ca.input, &cb.input);
    out.option(
        "mode",
        ca.mode.as_ref(),
        cb.mode.as_ref(),
        Weight::Adapts,
        "the modes confirm a stale write differently: the merged command keeps the stricter",
    );
    out.set(
        "permissions",
        &ca.permissions,
        &cb.permissions,
        Weight::Adapts,
        "the merged command asks the union",
    );
    out.set(
        "callable by",
        &ca.callable_by,
        &cb.callable_by,
        Weight::Note,
        "the merged command is callable by the union",
    );
}

/// Compares two units of the list.
#[must_use]
pub fn compare(a: &Entry, b: &Entry) -> Comparison {
    let mut out = Builder {
        same: Vec::new(),
        differences: Vec::new(),
    };
    if a.kind == b.kind {
        out.same.push("kind".to_string());
    } else {
        out.differ(
            "kind",
            a.kind.label().to_string(),
            b.kind.label().to_string(),
            Weight::Blocks,
            "a Capability reads and a DataCapability writes: the architecture keeps the two sides apart",
        );
    }
    if a.system == b.system {
        out.same.push("system".to_string());
    } else {
        out.differ(
            "system",
            a.system.clone(),
            b.system.clone(),
            Weight::Blocks,
            "two bounded contexts share a Concept, never a unit",
        );
    }
    if a.status == Status::Planned || b.status == Status::Planned {
        let word = |e: &Entry| {
            if e.status == Status::Planned {
                "planned"
            } else {
                "manifest"
            }
        };
        out.differ(
            "contract",
            word(a).to_string(),
            word(b).to_string(),
            Weight::Note,
            "a planned unit has no manifest yet: only its declaration is compared",
        );
    }
    if a.kind == b.kind {
        match a.kind {
            Kind::Capability => reads(&mut out, &a.contract, &b.contract),
            Kind::DataCapability => writes(&mut out, &a.contract, &b.contract),
        }
    }
    let rules = |e: &Entry| -> Vec<String> { e.rules.iter().map(|r| r.id.clone()).collect() };
    out.set(
        "business rules",
        &rules(a),
        &rules(b),
        Weight::Adapts,
        "the merged unit holds every rule of both; one side gains rules it never had",
    );
    out.differences.sort_by_key(|d| std::cmp::Reverse(d.weight));
    let verdict = match out.differences.first().map(|d| d.weight) {
        Some(Weight::Blocks) => Verdict::Impossible,
        Some(Weight::Adapts) => Verdict::Adaptable,
        _ => Verdict::Possible,
    };
    let callers: BTreeSet<String> = a
        .used_by
        .iter()
        .chain(&b.used_by)
        .map(|c| format!("{}.{}", c.system, c.micro_ui))
        .collect();
    Comparison {
        a: a.id.clone(),
        b: b.id.clone(),
        verdict,
        same: out.same,
        differences: out.differences,
        callers: callers.into_iter().collect(),
    }
}

/// The question the model is asked: the comparison, both roles, nothing
/// else. It words the answer; it does not redo the comparison.
#[must_use]
pub fn question(comparison: &Comparison, a: &Entry, b: &Entry) -> String {
    let unit = |e: &Entry| {
        serde_json::json!({
            "id": e.id, "kind": e.kind.label(), "role": e.role,
            "contract": e.contract,
            "rules": e.rules.iter().map(|r| format!("{}: {}", r.id, r.text)).collect::<Vec<_>>(),
        })
    };
    let facts = serde_json::json!({
        "comparison": comparison,
        "a": unit(a),
        "b": unit(b),
    });
    format!(
        "Two units of an agent-native application (read-only Capabilities, \
         DataCapabilities that write behind a DataGuard, Micro-UIs that call them) \
         may be merged into one. A deterministic comparison was already made: trust \
         its verdict and its differences, do not redo them, read nothing else.\n\n\
         Answer in English, plain text, at most 10 short lines:\n\
         1. `Verdict: possible`, `Verdict: possible with an adapted contract` or \
         `Verdict: not possible`, and why in one sentence.\n\
         2. The differences that matter, one line each.\n\
         3. If possible: the merged unit's id, input and output, and which Micro-UIs \
         must change.\n\n{facts}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::capabilities::Rule;

    fn cap(id: &str, input: &[(&str, &str)], implements: Option<&str>) -> Entry {
        Entry {
            id: id.to_string(),
            kind: Kind::Capability,
            system: "campagne".to_string(),
            name: id.to_string(),
            status: Status::Available,
            created: None,
            modified: None,
            role: String::new(),
            path: None,
            version: None,
            branches: Vec::new(),
            contract: Contract {
                input: input
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
                output: Some("[Pj]".to_string()),
                implements: implements.map(str::to_string),
                ..Contract::default()
            },
            used_by: Vec::new(),
            rules: Vec::new(),
            issues: Vec::new(),
        }
    }

    #[test]
    fn identical_contracts_merge() {
        let c = compare(
            &cap("a.x", &[("id", "ID")], None),
            &cap("a.y", &[("id", "ID")], None),
        );
        assert_eq!(c.verdict, Verdict::Possible, "{c:?}");
        assert!(c.same.contains(&"input".to_string()));
    }

    #[test]
    fn a_wider_input_is_an_adaptable_contract() {
        let c = compare(
            &cap("a.x", &[("id", "ID")], None),
            &cap("a.y", &[("id", "ID"), ("archived", "Bool")], None),
        );
        assert_eq!(c.verdict, Verdict::Adaptable);
        assert_eq!(c.differences[0].aspect, "input");
    }

    #[test]
    fn two_concepts_never_merge() {
        let c = compare(
            &cap("a.x", &[], Some("Risk@3")),
            &cap("a.y", &[], Some("Score@1")),
        );
        assert_eq!(c.verdict, Verdict::Impossible);
    }

    #[test]
    fn a_read_and_a_write_never_merge() {
        let mut write = cap("a.y", &[], None);
        write.kind = Kind::DataCapability;
        let c = compare(&cap("a.x", &[], None), &write);
        assert_eq!(c.verdict, Verdict::Impossible);
        assert_eq!(c.differences[0].aspect, "kind");
    }

    #[test]
    fn different_business_rules_adapt_and_the_question_carries_them() {
        let mut a = cap("a.x", &[], None);
        let b = cap("a.y", &[], None);
        a.rules.push(Rule {
            id: "pc-level-range".to_string(),
            text: "1 to 20".to_string(),
            from: "aggregate PJ".to_string(),
        });
        let c = compare(&a, &b);
        assert_eq!(c.verdict, Verdict::Adaptable);
        let q = question(&c, &a, &b);
        assert!(q.contains("pc-level-range: 1 to 20"));
        assert!(q.len() < 4000, "the question stays small: {}", q.len());
    }
}
