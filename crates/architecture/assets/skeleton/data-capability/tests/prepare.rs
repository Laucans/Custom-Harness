//! `{{id}}`'s pure core, no engine: [`request`] reads the wire, [`prepare`]
//! judges the row. SCAFFOLD: what these tests prove, one clause per rule.

use serde_json::{Value, json};

use {{lib}}::{Refusal, Row, prepare, request};

/// A row, from a JSON object.
fn row(value: Value) -> Row {
    serde_json::from_value(value).expect("an object")
}

/// SCAFFOLD: the house ids the tests name (`platform_test_kit::uuid`).
const ID: &str = "0a000000-0000-4000-8000-000000000001";

#[test]
fn every_refusal_says_why_in_words() {
    for refusal in [Refusal::NotAnId("id"), Refusal::Unknown, Refusal::Unwritten] {
        assert!(!refusal.to_string().trim().is_empty(), "{refusal:?}");
    }
}

#[test]
fn only_what_is_not_there_is_a_404() {
    assert!(Refusal::Unknown.not_found());
    assert!(!Refusal::NotAnId("id").not_found());
}

#[test]
fn an_id_that_is_no_uuid_is_refused() {
    assert_eq!(
        request(&row(json!({ "id": "x" })), None),
        Err(Refusal::NotAnId("id"))
    );
    let read = request(&row(json!({})), Some(&ID.to_uppercase())).expect("an id");
    assert_eq!(read.id, ID);
}

#[test]
fn a_row_that_is_not_there_is_unknown() {
    let read = request(&row(json!({})), Some(ID)).expect("an id");
    assert_eq!(prepare(&read, &Row::new()), Err(Refusal::Unknown));
}

#[test]
fn scaffold_one_test_per_acceptance_criterion() {
    panic!("SCAFFOLD: one test per acceptance criterion of the SPEC, named after it");
}
