//! Capability `{{id}}`: SCAFFOLD: what it answers, for which screen.
//!
//! It reads, and only reads. The rows of its persisted query
//! (`crates/{{system}}/queries/{{folder}}.graphql`, resolved by [`SQL`]) go
//! in; the answer, in the shape of `mock/fixtures/{{id}}.json`, comes out.
//! The filter is the SQL's, every request variable bound there: [`answer`]
//! never filters the rows again.

use std::collections::BTreeMap;

use serde_json::Value;

/// The Capability's identifier, as a Micro-UI cites it.
pub const CAPABILITY: &str = "{{id}}";

/// What the persisted query selects. SCAFFOLD: one SELECT; each request
/// variable bound as `<column>::text = $n` (an id that is no uuid then
/// finds nothing instead of failing), in the order of [`VARIABLES`]; every
/// field the query file selects read here.
pub const SQL: &str = r#"SELECT NULL::text AS id WHERE NULL::text = $1"#;

/// The request's variables [`SQL`] binds, in the order of its `$1`, `$2`…:
/// what the registry hands the query. SCAFFOLD: the input keys it filters on.
pub const VARIABLES: &[&str] = &["scaffold"];

/// The answer to a read, as the registry serves it: the `rows` [`SQL`]
/// selected, already filtered by the request's variables.
#[must_use]
pub fn answer(rows: &Value, _variables: &BTreeMap<String, Value>) -> Value {
    answer_rows(rows)
}

/// The answer `rows` make. SCAFFOLD: each row in the fixture's shape; no
/// row is the empty answer (`[]` for a list, `null` for one item).
fn answer_rows(rows: &Value) -> Value {
    let rows = rows.as_array().map(Vec::as_slice).unwrap_or_default();
    Value::Array(rows.iter().filter(|row| row.is_object()).cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform_test_kit::read;
    use serde_json::json;

    #[test]
    fn no_row_is_the_empty_answer() {
        assert_eq!(answer_rows(&json!([])), json!([]));
        assert_eq!(answer_rows(&Value::Null), json!([]));
    }

    #[test]
    fn the_sql_is_one_read_statement() {
        read::one_read_statement(SQL);
    }

    #[test]
    fn every_variable_is_bound_in_order() {
        read::binds_in_order(SQL, VARIABLES);
    }

    #[test]
    fn the_query_file_names_what_the_sql_selects() {
        let query = read::query("{{system}}", "{{folder}}");
        read::query_file_matches(SQL, &query, VARIABLES);
    }

    #[test]
    fn the_registry_s_answer_is_the_answer_of_its_rows() {
        let rows = json!([{ "id": "scaffold" }]);
        assert_eq!(answer(&rows, &BTreeMap::new()), answer_rows(&rows));
    }

    #[test]
    fn scaffold_the_answer_has_the_mock_up_s_shape() {
        let fixture = read::fixture(CAPABILITY);
        let rows = json!([]);
        assert_eq!(
            answer_rows(&rows),
            fixture,
            "SCAFFOLD: rows as SQL selects them"
        );
    }{{conformance_test}}
}
