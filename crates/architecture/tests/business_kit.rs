//! The rules' business kit's mounting checks (`tests/rules/business/src/
//! mounting.rs`, as `init-repo` installs it), compiled here as they are: a
//! unit a test reaches that its needs do not name, a read's variable its
//! `capability.json` `input` does not take, the engine's undeclared-key
//! warning in a command's answer — each a line, the rule mis-mounted. And
//! the kit's `lib.rs` wires them: a mis-mounted rule is recorded so and
//! passes, never red. The kit's HTTP side (ureq) is a live check
//! (`docs/toBeCheck/h5-mounting-in-the-kits.md`).

#[path = "../assets/rules/business/src/mounting.rs"]
mod mounting;

use harness_architecture::assets::content_of;
use proptest::prelude::*;
use serde_json::{Value, json};

fn needs(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

const VERSIONS: &str = "crates/campaign/capabilities/list-entity-versions/capability.json";

#[test]
fn a_unit_reached_is_named_in_the_needs_by_its_id_whatever_the_version() {
    let declared = needs(&[
        "capability:campaign.listEntityVersions",
        "data-capability:campaign.saveEntity@1",
    ]);
    assert_eq!(
        mounting::unnamed_need(&declared, "capability", "campaign.listEntityVersions"),
        None
    );
    assert_eq!(
        mounting::unnamed_need(&declared, "data-capability", "campaign.saveEntity@2"),
        None,
        "the version is no part of a need"
    );
    assert_eq!(
        mounting::unnamed_need(&declared, "data-capability", "campaign.saveEntity"),
        None
    );
    assert_eq!(
        mounting::unnamed_need(&declared, "capability", "campaign.saveEntity").as_deref(),
        Some(
            "reaches capability:campaign.saveEntity, which its needs do not name: a test lists every unit it uses"
        ),
        "a need of another kind does not count"
    );
    assert!(
        mounting::unnamed_need(
            &needs(&["data-capability:ArchiveCampaign"]),
            "data-capability",
            "campaign.archiveCampaign@1"
        )
        .is_some_and(|line| line.contains("data-capability:campaign.archiveCampaign,")),
        "a short name is no id"
    );
    assert!(mounting::unnamed_need(&needs(&["nonsense"]), "capability", "a.b").is_some());
    assert_eq!(mounting::unversioned("a.b@3"), "a.b");
    assert_eq!(mounting::unversioned("a.b"), "a.b");
}

#[test]
fn a_variable_off_the_input_is_named_with_what_the_manifest_takes() {
    let manifest =
        json!({ "capability": "campaign.listEntityVersions", "input": { "entity": "string" } });
    assert_eq!(
        mounting::off_input(
            "campaign.listEntityVersions",
            &json!({ "id": "x" }),
            &manifest,
            VERSIONS
        )
        .as_deref(),
        Some(
            "campaign.listEntityVersions has no input \"id\"; its capability.json takes entity (crates/campaign/capabilities/list-entity-versions/capability.json)"
        ),
        "R9 (#470), as the line reads"
    );
    assert_eq!(
        mounting::off_input(
            "campaign.listEntityVersions",
            &json!({ "entity": "elowen" }),
            &manifest,
            VERSIONS
        ),
        None,
        "a fixture's id is a value, never judged"
    );
    let dotted = json!({ "input": { "filter.kind": "string", "id": "uuid" } });
    assert_eq!(
        mounting::off_input("a.b", &json!({ "filter": {}, "id": "x" }), &dotted, "p"),
        None,
        "a dotted input declares its head"
    );
    assert_eq!(
        mounting::off_input("a.b", &json!({ "filt": 1 }), &dotted, "p").as_deref(),
        Some("a.b has no input \"filt\"; its capability.json takes filter.kind, id (p)"),
        "a prefix that is no dotted head is off"
    );
    assert_eq!(
        mounting::off_input(
            "a.b",
            &json!({ "x": 1, "y": 2 }),
            &json!({ "input": {} }),
            "p"
        )
        .as_deref(),
        Some("a.b has no input \"x\", \"y\"; its capability.json takes no input (p)")
    );
    for unjudged in [
        json!({}),
        json!({ "input": "string" }),
        json!({ "input": null }),
    ] {
        assert_eq!(
            mounting::off_input("a.b", &json!({ "x": 1 }), &unjudged, "p"),
            None,
            "no input object, nothing judged: {unjudged}"
        );
    }
    for variables in [Value::Null, json!([1]), json!("x")] {
        assert_eq!(
            mounting::off_input("a.b", &variables, &manifest, "p"),
            None,
            "{variables}"
        );
    }
}

#[test]
fn only_the_engine_s_undeclared_key_warning_is_a_payload_off_its_manifest() {
    let answer = json!({
        "commandId": "c1",
        "warnings": [
            3,
            "invariant(s) at risk: x",
            "payload key(s) portrait not declared by campaign.saveEntity",
            "payload key(s) not declared"
        ],
        "then": [{ "submitted": { "warnings": ["payload key(s) seal not declared by a.c"] } }]
    });
    assert_eq!(
        mounting::off_payload(&answer),
        [
            "sends a payload off its manifest: payload key(s) portrait not declared by campaign.saveEntity"
        ]
    );
    assert!(mounting::off_payload(&json!({ "warnings": "no" })).is_empty());
    assert!(mounting::off_payload(&Value::Null).is_empty());
}

#[test]
fn the_kit_records_a_mis_mounted_rule_so_and_never_red() {
    let kit = content_of("tests/rules/business/src/lib.rs").unwrap_or_default();
    let installed = content_of("tests/rules/business/src/mounting.rs").unwrap_or_default();
    assert_eq!(
        installed,
        include_str!("../assets/rules/business/src/mounting.rs")
    );
    for wired in [
        "mod mounting;",
        "mounting::unnamed_need(&self.needs, \"capability\", capability)",
        "mounting::off_input(",
        "mounting::unnamed_need(&self.needs, \"data-capability\", data_capability)",
        "self.mounted(mounting::off_payload(&accepted))?;",
        "record(id, \"mis-mounted\", &why);",
    ] {
        assert!(kit.contains(wired), "lib.rs wires {wired}");
    }
    let checked = kit.find("let mis_mounted = server.mis_mounted.take();");
    let judged = kit.find("match outcome {");
    assert!(
        checked.is_some() && checked < judged,
        "the mounting is judged before the body's outcome"
    );
    assert!(
        !kit.contains("data-capability:ArchiveCampaign"),
        "the example names ids, never short names"
    );
}

proptest! {
    #[test]
    fn variables_within_the_input_pass_and_any_other_key_is_named(
        input in prop::collection::btree_set("[a-z]{1,6}", 0..5),
        extra in "[A-Z]{1,6}",
    ) {
        let declared: serde_json::Map<String, Value> =
            input.iter().map(|k| (k.clone(), json!("string"))).collect();
        let manifest = json!({ "input": declared });
        let within: serde_json::Map<String, Value> =
            input.iter().map(|k| (k.clone(), json!(1))).collect();
        prop_assert_eq!(mounting::off_input("a.b", &Value::Object(within.clone()), &manifest, "p"), None);
        let mut off = within;
        off.insert(extra.clone(), json!(1));
        let line = mounting::off_input("a.b", &Value::Object(off), &manifest, "p");
        let quoted = format!("\"{extra}\"");
        prop_assert!(line.is_some_and(|l| l.contains(&quoted)));
    }
}
