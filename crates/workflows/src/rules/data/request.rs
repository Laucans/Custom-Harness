//! What a rules' issue asks, read from its title and body.
//!
//! Two kinds. An issue opened by the factory carries the whole of
//! `rules.json` (contract P) in a fenced `json` block — every rule, the
//! changed ones marked `conform` — and asks for it to be written with its
//! tests. One the CI opens from a development's `CONTEST.md` is titled
//! [`CONTESTED`] and carries what the development said: it asks for the
//! contest to be settled.
//!
//! The factory writes the body with [`body`], this module reads it back:
//! one format, one place.

use serde_json::Value;

/// The title of the first rules' task, opened at Initialize.
pub const FIRST_TITLE: &str = "Rules: write the rules and their tests";

/// How the title of a rules' task opened by a changed rule starts.
pub const CHANGED_TITLE: &str = "Rules changed:";

/// How the title of a contest starts — the CI writes it
/// (`scripts/rules-gate.mjs contest`), followed by `: PR #<n>`.
pub const CONTESTED: &str = "Rules contested";

/// What a rules' issue asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Write `rules.json` as given, and the tests of every rule.
    Write(Rules),
    /// Settle a contest: what the development said, as the CI copied it.
    Contest {
        /// The issue's body.
        said: String,
    },
}

/// The rules an issue carries, checked against contract P.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    /// `rules.json` as the task writes it.
    pub json: String,
    /// Every rule's id, in order.
    pub ids: Vec<String>,
    /// The rules marked `conform`: changed, their tests ahead of the code.
    pub conform: Vec<String>,
}

/// The body of a rules' issue: what it is about, then the rules.
///
/// # Errors
///
/// The rules do not keep contract P.
pub fn body(intro: &str, rules: &Value) -> Result<String, String> {
    let checked = checked(rules)?;
    Ok(format!(
        "{}\n\nThe rules, as `rules.json` (contract P) is to hold them:\n\n```json\n{}\n```\n",
        intro.trim(),
        checked.json
    ))
}

/// What an issue asks, from its title and body.
///
/// # Errors
///
/// A rules' issue that is not a contest carries no `json` block, or one that
/// does not keep contract P.
pub fn read(title: &str, body: &str) -> Result<Ask, String> {
    if title.trim_start().starts_with(CONTESTED) {
        return Ok(Ask::Contest {
            said: body.trim().to_string(),
        });
    }
    let block = json_block(body).ok_or("the issue carries no ```json block of rules")?;
    let value: Value =
        serde_json::from_str(block).map_err(|e| format!("the rules are not JSON: {e}"))?;
    checked(&value).map(Ask::Write)
}

/// The first fenced `json` block of a text.
fn json_block(text: &str) -> Option<&str> {
    let start = text.find("```json")?;
    let after = text.get(start + "```json".len()..)?;
    let open = after.find('\n')?;
    let inner = after.get(open + 1..)?;
    let end = inner.find("```")?;
    inner.get(..end)
}

/// Whether an id is `R` and a number, never zero-led.
fn is_id(id: &str) -> bool {
    id.strip_prefix('R').is_some_and(|n| {
        !n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())
    })
}

/// A non-empty string field.
fn text<'a>(of: &'a Value, field: &str) -> Option<&'a str> {
    of.get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// The rules, checked against contract P.
fn checked(value: &Value) -> Result<Rules, String> {
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("the rules are not contract P's version 1".to_string());
    }
    let all = value
        .get("rules")
        .and_then(Value::as_array)
        .ok_or("the rules carry no `rules` list")?;
    let mut ids: Vec<String> = Vec::new();
    let mut conform = Vec::new();
    for rule in all {
        let id = rule.get("id").and_then(Value::as_str).unwrap_or_default();
        if !is_id(id) {
            return Err(format!("`{id}` is not a rule's id (R and a number)"));
        }
        if ids.iter().any(|known| known == id) {
            return Err(format!("{id} is there twice"));
        }
        if text(rule, "title").is_none() {
            return Err(format!("{id} has no title"));
        }
        if !matches!(
            rule.get("family").and_then(Value::as_str),
            Some("journey" | "business")
        ) {
            return Err(format!("{id} is neither journey nor business"));
        }
        let criterion = rule.get("criterion").unwrap_or(&Value::Null);
        for part in ["given", "when", "then"] {
            if text(criterion, part).is_none() {
                return Err(format!("{id}'s criterion has no `{part}`"));
            }
        }
        match rule.get("state").map(Value::as_str) {
            None => {}
            Some(Some("conform")) => conform.push(id.to_string()),
            Some(_) => return Err(format!("{id}'s state is not `conform`")),
        }
        ids.push(id.to_string());
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    Ok(Rules { json, ids, conform })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn rule(id: &str) -> Value {
        json!({
            "id": id,
            "title": "A campaign archived leaves the list",
            "family": "business",
            "criterion": { "given": "a campaign", "when": "archived", "then": "not listed" }
        })
    }

    #[test]
    fn a_body_written_reads_back_as_the_same_rules() {
        let mut changed = rule("R3");
        changed["state"] = json!("conform");
        let rules = json!({ "version": 1, "rules": [rule("R1"), changed] });
        let text = body("The first task.", &rules).expect("written");
        assert!(text.starts_with("The first task."));
        let Ok(Ask::Write(read)) = read(FIRST_TITLE, &text) else {
            panic!("read back");
        };
        assert_eq!(read.ids, ["R1", "R3"]);
        assert_eq!(read.conform, ["R3"]);
        assert_eq!(
            serde_json::from_str::<Value>(&read.json).expect("json"),
            rules
        );
    }

    #[test]
    fn a_contest_is_read_from_its_title_whatever_its_body() {
        assert_eq!(
            read("Rules contested: PR #12", "  R3: cannot hold \n"),
            Ok(Ask::Contest {
                said: "R3: cannot hold".to_string()
            })
        );
    }

    #[test]
    fn rules_that_break_contract_p_are_refused_with_the_reason() {
        let cases = [
            (json!({ "version": 2, "rules": [] }), "version 1"),
            (json!({ "version": 1 }), "no `rules`"),
            (
                json!({ "version": 1, "rules": [rule("R0")] }),
                "not a rule's id",
            ),
            (
                json!({ "version": 1, "rules": [rule("R1"), rule("R1")] }),
                "twice",
            ),
            (
                json!({ "version": 1, "rules": [{ "id": "R1", "title": "t", "family": "ui", "criterion": {} }] }),
                "neither journey",
            ),
            (
                json!({ "version": 1, "rules": [{ "id": "R1", "title": "t", "family": "journey", "criterion": { "given": "a", "when": " " } }] }),
                "no `when`",
            ),
            (
                json!({ "version": 1, "rules": [{ "id": "R1", "family": "journey" }] }),
                "no title",
            ),
        ];
        for (rules, why) in cases {
            let refused = body("x", &rules).expect_err("refused");
            assert!(refused.contains(why), "{refused} says {why}");
        }
        let mut odd = rule("R2");
        odd["state"] = json!("draft");
        assert!(body("x", &json!({ "version": 1, "rules": [odd] })).is_err());
    }

    #[test]
    fn an_issue_without_its_block_or_with_broken_json_is_refused() {
        assert!(read(FIRST_TITLE, "no rules here").is_err());
        assert!(
            read(FIRST_TITLE, "```json\n{ not json\n```")
                .expect_err("refused")
                .contains("not JSON")
        );
    }

    proptest! {
        #[test]
        fn any_number_not_zero_led_is_an_id(n in 1u64..1_000_000) {
            let id = format!("R{n}");
            prop_assert!(is_id(&id));
            let zero_led = format!("R0{n}");
            prop_assert!(!is_id(&zero_led));
            let lower = format!("r{n}");
            prop_assert!(!is_id(&lower));
        }

        #[test]
        fn reading_any_text_never_panics(title in ".{0,40}", body in ".{0,200}") {
            let _ = read(&title, &body);
        }

        #[test]
        fn the_block_is_found_wherever_the_body_puts_it(
            before in "[a-z ]{0,30}",
            after in "[a-z ]{0,30}",
        ) {
            let rules = json!({ "version": 1, "rules": [rule("R1")] });
            let text = format!("{before}\n```json\n{rules}\n```\n{after}");
            let read = read(FIRST_TITLE, &text);
            prop_assert!(matches!(read, Ok(Ask::Write(r)) if r.ids == ["R1"]));
        }
    }
}
