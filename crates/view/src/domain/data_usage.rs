//! Who uses a table or a field of the product's database, on one branch:
//!
//! - the Capabilities that read it, through a persisted query — what each
//!   `queries/registry.json` says the query reads, `<entity>.<field>`, an
//!   entity being a table or a view the data model knows the tables behind;
//! - the `DataCapabilities` that write it — the aggregate a
//!   `data-capability.json` targets, read as the table of the same name, and
//!   the fields its payload carries.
//!
//! Pure: the files are the `Units` port's reading, the tables the data model's.

use std::collections::BTreeMap;

use harness_architecture::model::DataModel;
use serde::Serialize;
use serde_json::Value;

use crate::domain::capabilities::Kind;
use crate::ports::UnitFile;

/// How a unit uses what it is listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Use {
    /// A Capability's persisted query reads it.
    Reads,
    /// A `DataCapability` writes it.
    Writes,
}

/// One unit that uses a table or a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    /// `campagne.listerPjs`.
    pub id: String,
    /// Read side or write side.
    pub kind: Kind,
    /// Reads or writes.
    pub how: Use,
    /// Through what: `query ListePjs`, `insert`.
    pub via: String,
    /// The file that says so, relative to the product's root.
    pub path: String,
}

/// The users, by what they use: `table:pj`, `field:pj.niveau` — the keys
/// the page's pins and boxes are named by.
pub type UsedBy = BTreeMap<String, Vec<User>>;

/// Who uses what, from one branch's files.
#[must_use]
pub fn used_by(model: &DataModel, files: &[UnitFile]) -> UsedBy {
    let mut out = UsedBy::new();
    for file in files {
        let Ok(json) = serde_json::from_str::<Value>(&file.text) else {
            continue;
        };
        if file.path.ends_with("queries/registry.json") {
            registry(model, file, &json, &mut out);
        } else if file.path.ends_with("data-capability.json") {
            data_capability(model, file, &json, &mut out);
        }
    }
    for users in out.values_mut() {
        users.sort_by(|a, b| (a.how, &a.id).cmp(&(b.how, &b.id)));
    }
    out
}

/// A registry's queries: each Capability under every table and field its
/// query reads.
fn registry(model: &DataModel, file: &UnitFile, json: &Value, out: &mut UsedBy) {
    let queries = json.get("queries").and_then(Value::as_array);
    for query in queries.into_iter().flatten() {
        let Some(id) = query.get("capability").and_then(Value::as_str) else {
            continue;
        };
        let name = query.get("query").and_then(Value::as_str).unwrap_or("");
        let user = User {
            id: id.to_string(),
            kind: Kind::Capability,
            how: Use::Reads,
            via: format!("query {name}"),
            path: file.path.clone(),
        };
        let fields = query.get("fields").and_then(Value::as_array);
        for read in fields.into_iter().flatten().filter_map(Value::as_str) {
            let Some((entity, field)) = read.rsplit_once('.') else {
                continue;
            };
            for (table, column) in behind(model, entity, field) {
                add(out, format!("table:{table}"), &user);
                if let Some(column) = column {
                    add(out, format!("field:{table}.{column}"), &user);
                }
            }
        }
    }
}

/// The tables, and the column when it is known, behind `entity.field`: the
/// table itself, or a view's — its column's own table, else every table it
/// reads.
fn behind(model: &DataModel, entity: &str, field: &str) -> Vec<(String, Option<String>)> {
    if let Some(table) = model.tables.iter().find(|t| t.name == entity) {
        let column = table
            .fields
            .iter()
            .any(|f| f.name == field)
            .then(|| field.to_string());
        return vec![(table.name.clone(), column)];
    }
    let Some(view) = model.views.iter().find(|v| v.name == entity) else {
        return Vec::new();
    };
    if let Some(column) = view.columns.iter().find(|c| c.name == field) {
        return behind(model, &column.table, &column.field);
    }
    view.sources
        .iter()
        .filter(|s| model.tables.iter().any(|t| &t.name == *s))
        .map(|s| (s.clone(), None))
        .collect()
}

/// A `DataCapability`: under the table its aggregate names, and the fields
/// its payload and its `touches` carry.
fn data_capability(model: &DataModel, file: &UnitFile, json: &Value, out: &mut UsedBy) {
    let Some(id) = json.get("dataCapability").and_then(Value::as_str) else {
        return;
    };
    let effect = json
        .get("effect")
        .and_then(Value::as_str)
        .unwrap_or("writes");
    let user = User {
        id: id.to_string(),
        kind: Kind::DataCapability,
        how: Use::Writes,
        via: effect.to_string(),
        path: file.path.clone(),
    };
    let aggregate = json
        .get("target")
        .and_then(|t| t.get("aggregate"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let target = table_of(model, aggregate);
    if let Some(table) = &target {
        add(out, format!("table:{table}"), &user);
    }
    let payload = json.get("payload").and_then(Value::as_object);
    let carried = payload.into_iter().flat_map(|p| p.keys().cloned());
    let listed = json.get("touches").and_then(Value::as_array);
    let touched = listed
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string);
    for name in carried.chain(touched) {
        // `PJ.niveau` names another aggregate's field; `niveau`, the target's.
        let (table, field) = match name.rsplit_once('.') {
            Some((aggregate, field)) => (table_of(model, aggregate), field.to_string()),
            None => (target.clone(), name),
        };
        let Some(table) = table else {
            continue;
        };
        let known = model
            .tables
            .iter()
            .any(|t| t.name == table && t.fields.iter().any(|f| f.name == field));
        if known {
            add(out, format!("table:{table}"), &user);
            add(out, format!("field:{table}.{field}"), &user);
        }
    }
}

/// The table an aggregate is kept in: the same name, any case, or its
/// `snake_case` (`PlayerCharacter` → `player_character`).
fn table_of(model: &DataModel, aggregate: &str) -> Option<String> {
    if aggregate.is_empty() {
        return None;
    }
    let mut snake = String::new();
    for (i, c) in aggregate.chars().enumerate() {
        if c.is_uppercase()
            && i > 0
            && !aggregate.chars().nth(i - 1).is_some_and(char::is_uppercase)
        {
            snake.push('_');
        }
        snake.extend(c.to_lowercase());
    }
    model
        .tables
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(aggregate) || t.name == snake)
        .map(|t| t.name.clone())
}

/// Lists `user` under `key`, once.
fn add(out: &mut UsedBy, key: String, user: &User) {
    let users = out.entry(key).or_default();
    if !users.iter().any(|u| u.id == user.id && u.how == user.how) {
        users.push(user.clone());
    }
}

#[cfg(test)]
mod tests {
    use harness_architecture::model;

    use super::*;

    const SCHEMA: &str = r#"
CREATE TABLE public.campagne (id uuid, nom text, "creeLe" timestamptz);
CREATE TABLE public.pj (id uuid, "campagneId" uuid, nom text, niveau integer);
CREATE TABLE public.player_character (id uuid);
CREATE VIEW public.pj_actif AS
 SELECT p.id, p."campagneId", p.niveau AS level
   FROM (public.pj p JOIN public.campagne c ON ((c.id = p."campagneId")));
"#;

    fn file(path: &str, text: &str) -> UnitFile {
        UnitFile {
            path: path.to_string(),
            text: text.to_string(),
        }
    }

    fn ids(used: &UsedBy, key: &str) -> Vec<String> {
        used.get(key)
            .map(|users| {
                users
                    .iter()
                    .map(|u| format!("{} {:?}", u.id, u.how))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn reads_go_through_views_and_writes_through_aggregates() {
        let model = model::build("here", &Ok(Some(SCHEMA.to_string())), &Ok(None));
        let files = [
            file(
                "crates/campagne/queries/registry.json",
                r#"{"queries": [
                    {"query": "ListePjs", "capability": "campagne.listerPjs",
                     "fields": ["pj_actif.level", "pj_actif.campagneId", "campagne.nom", "pj_actif.computed"]}
                ]}"#,
            ),
            file(
                "crates/dataguard/data-capabilities/ajouter-pj/data-capability.json",
                r#"{"dataCapability": "campagne.ajouterPj", "effect": "insert",
                    "target": {"aggregate": "PJ"}, "touches": ["Campagne.nom"],
                    "payload": {"campagneId": "uuid", "niveau": "integer", "unknown": "x"}}"#,
            ),
            file(
                "crates/x/data-capabilities/y/data-capability.json",
                r#"{"dataCapability": "x.y", "target": {"aggregate": "PlayerCharacter"}}"#,
            ),
            file("crates/broken/queries/registry.json", "not json"),
        ];
        let used = used_by(&model, &files);
        assert_eq!(
            ids(&used, "field:pj.niveau"),
            ["campagne.listerPjs Reads", "campagne.ajouterPj Writes"],
            "a view's column is its table's"
        );
        assert_eq!(
            ids(&used, "table:pj"),
            ["campagne.listerPjs Reads", "campagne.ajouterPj Writes"],
            "listed once per unit"
        );
        assert_eq!(
            ids(&used, "field:campagne.nom"),
            ["campagne.listerPjs Reads", "campagne.ajouterPj Writes"],
            "a touched field of another aggregate"
        );
        assert!(ids(&used, "field:pj.unknown").is_empty(), "no such column");
        assert_eq!(ids(&used, "table:player_character"), ["x.y Writes"]);
        assert_eq!(
            used.get("field:pj.niveau").map(|u| u[0].via.as_str()),
            Some("query ListePjs")
        );
    }
}
