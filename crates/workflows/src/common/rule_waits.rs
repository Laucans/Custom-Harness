//! What the rules' tests wait for, and which task unblocks them.
//!
//! A rule's test is pending while a unit it needs is not built: the kit
//! records `waiting for kind:name, …` (`kit.ts rule()`, the business kit's
//! `rule()`), `rules-gate.mjs status` folds it into `rules-status.json`, and
//! `gates.yml` keeps that file on the integration branch as the
//! `rules-status` artifact. The router reads it once per integration head
//! ([`crate::common::routing::rules_status_due`]) and weighs each task by the
//! rules it unblocks ([`bonus`]): `ForgeDesMondes`' #157 held 17 rules and
//! waited behind lone Micro-UIs of its milestone.
//!
//! Pure: the text is read by the launcher, through the `GitHub` port. A
//! shape this module does not know reads as no wait at all — it orders
//! work, it never holds any.

use std::collections::BTreeMap;

use harness_architecture::vocabulary::Unit;
use harness_core::domain::Issue;
use serde_json::Value;

/// The most a task's rules add to its weight.
pub const RULES_CAP: u32 = 100;

/// What each rule a task brings closer to running adds: it builds one of
/// the units the rule's test waits for.
pub const PER_TOUCHED: u32 = 4;

/// What each rule a task completes adds on top: it builds every unit the
/// rule's test still waits for.
pub const PER_COMPLETED: u32 = 8;

/// What a `DataCapability` adds whatever its rules: the owner's
/// "`DataCapabilities` first", a tiebreak among tasks of one weight.
pub const DATA_CAPABILITY_FIRST: u32 = 5;

/// The pending rules and the units each one's tests wait for (`kind:name`,
/// as the tests declare them), by rule id (`R3`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Waits(pub BTreeMap<String, Vec<String>>);

/// The rules a task brings closer to running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unblocked {
    /// The pending rules one of whose missing units the task builds.
    pub touched: u32,
    /// Those whose every missing unit the task builds: it runs once the
    /// task lands.
    pub completed: u32,
}

/// The waits a `rules-status.json` text holds.
///
/// Its `pending` rules, each with the units its tests' `why` names, each
/// once. A rule pending for another reason (`no app: …`) waits on no unit
/// and is left out; a text that does not read, or a shape this does not
/// know, is no wait.
#[must_use]
pub fn parse(text: &str) -> Waits {
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return Waits::default();
    };
    let mut waits = BTreeMap::new();
    for rule in json
        .get("rules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if rule.get("state").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let Some(id) = rule.get("id").and_then(Value::as_str) else {
            continue;
        };
        let mut missing: Vec<String> = Vec::new();
        for test in rule
            .get("tests")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let why = test.get("why").and_then(Value::as_str).unwrap_or_default();
            for need in waiting_for(why) {
                if !missing.contains(&need) {
                    missing.push(need);
                }
            }
        }
        if !missing.is_empty() {
            waits.insert(id.to_string(), missing);
        }
    }
    Waits(waits)
}

/// The units a test's `why` says it waits for: `waiting for a:b, c:d` gives
/// `a:b` and `c:d`; any other `why` (`no app: …`, empty) none.
#[must_use]
pub fn waiting_for(why: &str) -> Vec<String> {
    let Some(list) = why.trim().strip_prefix("waiting for ") else {
        return Vec::new();
    };
    list.split(',')
        .map(str::trim)
        .filter(|need| {
            need.split_once(':')
                .is_some_and(|(kind, name)| !kind.is_empty() && !name.is_empty())
        })
        .map(str::to_string)
        .collect()
}

/// The pending rules `task` brings closer to running: a `kind:name` it
/// waits for is the task's when the task's unit is that kind (`screen` is
/// a composition) and it produces that name, case aside.
#[must_use]
pub fn unblocked(task: &Issue, waits: &Waits) -> Unblocked {
    let Some(declaration) = crate::dev_loop::data::tasks::declaration(task) else {
        return Unblocked::default();
    };
    let builds = |need: &String| {
        need.split_once(':').is_some_and(|(kind, name)| {
            (kind == declaration.unit.key()
                || (kind == "screen" && declaration.unit == Unit::Composition))
                && declaration
                    .produces
                    .iter()
                    .any(|produced| produced.eq_ignore_ascii_case(name))
        })
    };
    let mut found = Unblocked::default();
    for missing in waits.0.values() {
        let built = missing.iter().filter(|need| builds(need)).count();
        if built > 0 {
            found.touched = found.touched.saturating_add(1);
        }
        if built > 0 && built == missing.len() {
            found.completed = found.completed.saturating_add(1);
        }
    }
    found
}

/// What `task`'s rules add to its weight: [`PER_TOUCHED`] a rule touched,
/// [`PER_COMPLETED`] more a rule completed, at most [`RULES_CAP`] — and
/// [`DATA_CAPABILITY_FIRST`] for a `DataCapability`, whatever its rules.
#[must_use]
pub fn bonus(task: &Issue, waits: &Waits) -> u32 {
    let Unblocked { touched, completed } = unblocked(task, waits);
    let rules = PER_TOUCHED
        .saturating_mul(touched)
        .saturating_add(PER_COMPLETED.saturating_mul(completed))
        .min(RULES_CAP);
    let first = crate::dev_loop::data::tasks::declaration(task)
        .is_some_and(|declaration| declaration.unit == Unit::DataCapability);
    rules.saturating_add(if first { DATA_CAPABILITY_FIRST } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The shape `rules-gate.mjs status` wrote on 2026-10-24.
    const STATUS: &str = r#"{
      "at": "2026-10-24T21:04:11.118Z",
      "rules": [
        {"id": "R3", "title": "a campaign opens", "family": "journey", "conform": false, "state": "pending",
         "tests": [{"test": "R3 a campaign opens from the list", "state": "pending",
                    "why": "waiting for screen:campagne, micro-ui:CampaignList"}]},
        {"id": "R5", "title": "notes", "family": "business", "conform": false, "state": "pending",
         "tests": [{"test": "r5_notes", "state": "pending", "why": "waiting for data-capability:campaign.createCampaign"},
                   {"test": "r5_notes_bis", "state": "pending",
                    "why": "waiting for capability:campaign.listCampaigns, data-capability:campaign.createCampaign"}]},
        {"id": "R7", "title": "no app", "family": "journey", "conform": false, "state": "pending",
         "tests": [{"test": "R7", "state": "pending", "why": "no app: apps/shell is not built, or the runtime serves none"}]},
        {"id": "R9", "title": "green", "family": "business", "conform": false, "state": "green",
         "tests": [{"test": "r9", "state": "green", "why": ""}]},
        {"id": "R11", "title": "untested", "family": "business", "conform": true, "state": "untested", "tests": []}
      ]
    }"#;

    fn task(unit: &str, produces: &str) -> Issue {
        Issue {
            number: 157,
            state: "open".to_string(),
            body: format!(
                "## Architecture\n\nunit: {unit}\nsystem: campaign\nproduces: {produces}"
            ),
            ..Issue::default()
        }
    }

    #[test]
    fn the_pending_rules_are_read_with_every_unit_their_tests_wait_for() {
        let waits = parse(STATUS);
        assert_eq!(
            waits.0.keys().map(String::as_str).collect::<Vec<_>>(),
            ["R3", "R5"],
            "green, untested and a rule pending on no unit are no wait"
        );
        assert_eq!(
            waits.0["R5"],
            [
                "data-capability:campaign.createCampaign",
                "capability:campaign.listCampaigns"
            ],
            "each unit once, across the rule's tests"
        );
        assert_eq!(parse("not json"), Waits::default());
        assert_eq!(parse(r#"{"rules": 3}"#), Waits::default());
        assert_eq!(
            parse(r#"{"rules": [{"state": "pending", "tests": []}, 4]}"#),
            Waits::default(),
            "a rule without an id is skipped, not a failure"
        );
    }

    #[test]
    fn a_why_names_its_units_only_after_waiting_for() {
        assert_eq!(
            waiting_for(" waiting for capability:a.b, micro-ui:X "),
            ["capability:a.b", "micro-ui:X"]
        );
        assert!(
            waiting_for("no app: apps/shell is not built").is_empty(),
            "should be empty"
        );
        assert!(waiting_for("").is_empty(), "should be empty");
        assert!(
            waiting_for("waiting for nothing, :x, y:").is_empty(),
            "should be empty"
        );
    }

    #[test]
    fn a_task_touches_the_rules_it_builds_a_unit_of_and_completes_those_it_builds_all_of() {
        let waits = parse(STATUS);
        let create = task("data-capability", "campaign.createcampaign");
        assert_eq!(
            unblocked(&create, &waits),
            Unblocked {
                touched: 1,
                completed: 0
            },
            "R5 still waits on the list"
        );
        // The ids compare case aside, the kind does not.
        let list = task(
            "capability",
            "campaign.listCampaigns, campaign.createCampaign",
        );
        assert_eq!(unblocked(&list, &waits).touched, 1);
        assert_eq!(
            unblocked(&list, &waits).completed,
            0,
            "a capability is not a write"
        );
        // A screen produces `screen:<name>`, a Micro-UI `micro-ui:<name>`.
        let screen = task("composition", "campagne");
        let ui = task("micro-ui", "CampaignList");
        assert_eq!(
            unblocked(&screen, &waits),
            Unblocked {
                touched: 1,
                completed: 0
            }
        );
        assert_eq!(unblocked(&ui, &waits).touched, 1);
        let both = Waits(BTreeMap::from([(
            "R3".to_string(),
            vec!["screen:campagne".to_string()],
        )]));
        assert_eq!(
            unblocked(&screen, &both),
            Unblocked {
                touched: 1,
                completed: 1
            }
        );
        assert_eq!(
            unblocked(&Issue::default(), &waits),
            Unblocked::default(),
            "a task that declares nothing unblocks nothing"
        );
    }

    #[test]
    fn a_screen_need_is_a_composition_s_and_no_other_kind_is() {
        let waits = Waits(BTreeMap::from([(
            "R3".to_string(),
            vec!["micro-ui:CampaignList".to_string()],
        )]));
        assert_eq!(
            unblocked(&task("composition", "CampaignList"), &waits),
            Unblocked::default(),
            "a composition builds no Micro-UI"
        );
    }

    #[test]
    fn a_rule_waiting_on_no_unit_is_never_completed() {
        let waits = Waits(BTreeMap::from([("R1".to_string(), Vec::new())]));
        assert_eq!(
            unblocked(&task("capability", "x.list"), &waits),
            Unblocked::default()
        );
    }

    #[test]
    fn the_bonus_counts_rules_up_to_the_cap_and_a_data_capability_goes_first() {
        let many = |count: usize, completed: bool| {
            Waits(
                (0..count)
                    .map(|i| {
                        let mut missing = vec!["data-capability:x.make".to_string()];
                        if !completed {
                            missing.push("capability:y.list".to_string());
                        }
                        (format!("R{i}"), missing)
                    })
                    .collect(),
            )
        };
        let write = task("data-capability", "x.make");
        assert_eq!(bonus(&write, &Waits::default()), DATA_CAPABILITY_FIRST);
        assert_eq!(bonus(&write, &many(2, false)), 2 * PER_TOUCHED + 5);
        assert_eq!(
            bonus(&write, &many(2, true)),
            2 * (PER_TOUCHED + PER_COMPLETED) + 5
        );
        assert_eq!(bonus(&write, &many(17, false)), 68 + 5);
        assert_eq!(bonus(&write, &many(17, true)), RULES_CAP + 5, "capped");
        let read = task("capability", "y.list");
        assert_eq!(bonus(&read, &many(3, false)), 3 * PER_TOUCHED);
    }

    fn need() -> impl Strategy<Value = String> {
        ("[a-z-]{1,12}", "[A-Za-z0-9._@-]{1,30}").prop_map(|(kind, name)| format!("{kind}:{name}"))
    }

    proptest! {
        #[test]
        fn a_why_the_kit_writes_reads_back_its_units(needs in prop::collection::vec(need(), 1..6)) {
            prop_assert_eq!(waiting_for(&format!("waiting for {}", needs.join(", "))), needs);
        }

        #[test]
        fn any_text_reads_without_a_panic(text in ".*") {
            let _ = waiting_for(&text);
            let _ = parse(&text);
        }

        #[test]
        fn a_status_written_reads_back_its_pending_rules(
            rules in prop::collection::btree_map("R[0-9]{1,3}", (prop::collection::vec(need(), 0..4), any::<bool>()), 0..8)
        ) {
            let written = serde_json::json!({
                "at": "now",
                "rules": rules.iter().map(|(id, (needs, pending))| {
                    let why = if needs.is_empty() { String::new() } else { format!("waiting for {}", needs.join(", ")) };
                    serde_json::json!({
                        "id": id,
                        "state": if *pending { "pending" } else { "red" },
                        "tests": [{"test": id, "state": "pending", "why": why}],
                    })
                }).collect::<Vec<_>>(),
            });
            let expected: BTreeMap<String, Vec<String>> = rules
                .iter()
                .filter(|(_, (needs, pending))| *pending && !needs.is_empty())
                .map(|(id, (needs, _))| {
                    let mut once: Vec<String> = Vec::new();
                    for need in needs {
                        if !once.contains(need) {
                            once.push(need.clone());
                        }
                    }
                    (id.clone(), once)
                })
                .collect();
            prop_assert_eq!(parse(&written.to_string()), Waits(expected));
        }
    }
}
