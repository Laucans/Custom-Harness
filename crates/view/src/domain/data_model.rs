//! The product's data, as one picture: the tables the database has, told
//! by its DDL, and the tables the agents describe, told by their model —
//! merged field by field, with where the two disagree.
//!
//! Two files in the product's repository, side by side:
//!
//! - [`SCHEMA_PATH`] — `pg_dump --schema-only`: the structure, the truth of
//!   what is deployed. Types, keys, `NOT NULL`, checks, indexes, comments.
//! - [`MODEL_PATH`] — the agents' model, JSON: what each table and field is
//!   *for*, the business rules on it, the links no foreign key carries. The
//!   agents refine it task after task.
//!
//! Either file alone draws the diagram. With both, every table, field and
//! link says which side has it, and every disagreement is listed — what a
//! static check reads to decide whether an agent must look.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::schema_sql::{self, Rule, Schema, SqlColumn, SqlTable};

/// Where the database's side lies in the product's repository.
pub const SCHEMA_PATH: &str = "data/schema.sql";

/// Where the agents' side lies in the product's repository.
pub const MODEL_PATH: &str = "data/model.json";

// ---- the agents' model, as the file has it ----------------------------------------

/// `data/model.json`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Declared {
    /// The format's version; 1.
    pub version: u32,
    /// The tables the agents describe.
    pub tables: Vec<DeclaredTable>,
    /// Links that exist outside any foreign key.
    pub links: Vec<DeclaredLink>,
}

/// One table of the agents' model.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DeclaredTable {
    /// As the database names it.
    pub name: String,
    /// What the table is for, in a sentence.
    pub purpose: String,
    /// Business rules on the whole table.
    pub business: Vec<String>,
    /// Its fields.
    pub fields: Vec<DeclaredField>,
}

/// One field of the agents' model.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DeclaredField {
    /// As the database names it.
    pub name: String,
    /// The type the agents intend, in Postgres words.
    #[serde(rename = "type")]
    pub data_type: Option<String>,
    /// Whether it may be empty.
    pub nullable: Option<bool>,
    /// What the field is for, in a sentence.
    pub purpose: String,
    /// Technical rules the DDL cannot say (or says, repeated here).
    pub technical: Vec<String>,
    /// Business rules on the field.
    pub business: Vec<String>,
    /// `table.field` this one points at — a foreign key.
    pub references: Option<String>,
}

/// A link outside any foreign key: `character.class` names a row of `rulebook`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DeclaredLink {
    /// `table.field`.
    pub from: String,
    /// `table.field`.
    pub to: String,
    /// Why the two are linked.
    pub purpose: String,
}

// ---- the picture the page draws ----------------------------------------------------

/// Which side has a table, a field or a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Seen {
    /// The database and the model.
    Both,
    /// The database only.
    Database,
    /// The model only.
    Model,
}

/// One file read, or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Source {
    /// Its path in the product's repository.
    pub path: String,
    /// Whether it was there.
    pub found: bool,
    /// Why it could not be read, when it could not.
    pub problem: Option<String>,
}

/// The data model, merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataModel {
    /// Where the files were read: `owner/name @ main_agent`.
    pub origin: String,
    /// The two files.
    pub sources: Vec<Source>,
    /// Both sides were read, so `seen` and `divergences` mean something.
    pub compared: bool,
    /// The tables, the database's order first.
    pub tables: Vec<Table>,
    /// Every link, foreign keys and declared ones.
    pub links: Vec<Link>,
    /// Where the two sides disagree, a sentence each.
    pub divergences: Vec<String>,
}

/// One table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Table {
    /// As the database names it.
    pub name: String,
    /// The model's purpose, or the database's comment.
    pub purpose: String,
    /// Business rules on the whole table.
    pub business: Vec<String>,
    /// Rules on several fields at once: a composite key, a table check.
    pub technical: Vec<String>,
    /// In the database's order, then the model's.
    pub fields: Vec<Field>,
    /// Which side has it.
    pub seen: Seen,
}

/// One field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Field {
    /// As the database names it.
    pub name: String,
    /// The database's type, or the model's when the database has none.
    pub data_type: String,
    /// The model's type, when it disagrees with the database's.
    pub declared_type: Option<String>,
    /// Whether it may be empty; `None` when nobody said.
    pub nullable: Option<bool>,
    /// The `DEFAULT` expression.
    pub default: Option<String>,
    /// The model's purpose, or the database's comment.
    pub purpose: String,
    /// Keys, checks, indexes, defaults — a sentence each.
    pub technical: Vec<String>,
    /// The business rules the model puts on it.
    pub business: Vec<String>,
    /// Part of the primary key.
    pub primary: bool,
    /// Unique on its own.
    pub unique: bool,
    /// Points at another field.
    pub foreign: bool,
    /// Which side has it.
    pub seen: Seen,
}

/// What a link is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// A foreign key, in the database or intended by the model.
    ForeignKey,
    /// A link the model declares outside any key.
    Semantic,
}

/// One field pointing at another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    /// The table that points.
    pub from_table: String,
    /// Its field.
    pub from_field: String,
    /// The table pointed at.
    pub to_table: String,
    /// Its field.
    pub to_field: String,
    /// A key, or a declared link.
    pub kind: LinkKind,
    /// The constraint's name.
    pub name: Option<String>,
    /// `ON DELETE …`.
    pub on_delete: Option<String>,
    /// `ON UPDATE …`.
    pub on_update: Option<String>,
    /// Why, when the model says.
    pub purpose: String,
    /// Which side has it.
    pub seen: Seen,
}

/// One file as the adapter read it: `None` when absent.
pub type FileRead = Result<Option<String>, String>;

/// Merges what the two files say into the picture the page draws.
#[must_use]
pub fn build(origin: &str, schema: &FileRead, model: &FileRead) -> DataModel {
    let source = |path: &str, read: &FileRead| Source {
        path: path.to_string(),
        found: matches!(read, Ok(Some(_))),
        problem: read.as_ref().err().cloned(),
    };
    let mut sources = vec![source(SCHEMA_PATH, schema), source(MODEL_PATH, model)];
    let sql = schema
        .as_ref()
        .ok()
        .cloned()
        .flatten()
        .map(|text| schema_sql::parse(&text));
    let declared = match model {
        Ok(Some(text)) => match serde_json::from_str::<Declared>(text) {
            Ok(declared) => Some(declared),
            Err(e) => {
                if let Some(s) = sources.get_mut(1) {
                    s.problem = Some(format!("not the model's JSON: {e}"));
                }
                None
            }
        },
        _ => None,
    };
    let mut model = merge(sql.as_ref(), declared.as_ref());
    model.origin = origin.to_string();
    model.sources = sources;
    model
}

/// The merge itself, on what was read.
fn merge(sql: Option<&Schema>, declared: Option<&Declared>) -> DataModel {
    let compared = sql.is_some() && declared.is_some();
    let empty = Schema::default();
    let schema = sql.unwrap_or(&empty);
    let decl_tables: Vec<&DeclaredTable> = declared
        .map(|d| d.tables.iter().collect())
        .unwrap_or_default();
    let mut out = DataModel {
        origin: String::new(),
        sources: Vec::new(),
        compared,
        tables: Vec::new(),
        links: Vec::new(),
        divergences: Vec::new(),
    };

    let mut names: Vec<String> = schema.tables.iter().map(|t| t.name.clone()).collect();
    for t in &decl_tables {
        if !names.contains(&t.name) {
            names.push(t.name.clone());
        }
    }
    for name in &names {
        let s = schema.tables.iter().find(|t| &t.name == name);
        let d = decl_tables.iter().find(|t| &t.name == name).copied();
        let seen = seen_of(s.is_some(), d.is_some());
        if compared {
            match seen {
                Seen::Database => out
                    .divergences
                    .push(format!("table {name} is in the database, not in the model")),
                Seen::Model => out
                    .divergences
                    .push(format!("table {name} is in the model, not in the database")),
                Seen::Both => {}
            }
        }
        let (table, links) = table_of(name, s, d, schema, compared, &mut out.divergences);
        out.tables.push(table);
        out.links.extend(links);
    }
    intended_keys(&mut out, &decl_tables);
    if compared {
        for link in out.links.iter().filter(|l| l.seen == Seen::Database) {
            out.divergences.push(format!(
                "{}.{} references {}.{} in the database; the model does not say so",
                link.from_table, link.from_field, link.to_table, link.to_field
            ));
        }
    }
    for link in declared.map(|d| d.links.as_slice()).unwrap_or_default() {
        let (Some((from_table, from_field)), Some((to_table, to_field))) =
            (link.from.split_once('.'), link.to.split_once('.'))
        else {
            continue;
        };
        out.links.push(Link {
            from_table: from_table.to_string(),
            from_field: from_field.to_string(),
            to_table: to_table.to_string(),
            to_field: to_field.to_string(),
            kind: LinkKind::Semantic,
            name: None,
            on_delete: None,
            on_update: None,
            purpose: link.purpose.clone(),
            seen: Seen::Model,
        });
    }
    out
}

/// The keys the model intends: one the database has is seen by both; one
/// it has not is a link of the model's, and a divergence when compared.
fn intended_keys(out: &mut DataModel, decl_tables: &[&DeclaredTable]) {
    for d in decl_tables {
        for f in &d.fields {
            let Some((to_table, to_field)) =
                f.references.as_deref().and_then(|r| r.split_once('.'))
            else {
                continue;
            };
            if let Some(link) = out.links.iter_mut().find(|l| {
                l.kind == LinkKind::ForeignKey
                    && l.from_table == d.name
                    && l.from_field == f.name
                    && l.to_table == to_table
            }) {
                link.seen = Seen::Both;
                if link.to_field.is_empty() {
                    link.to_field = to_field.to_string();
                }
                continue;
            }
            if out.compared {
                out.divergences.push(format!(
                    "{}.{} references {to_table}.{to_field} in the model; the database has no such foreign key",
                    d.name, f.name
                ));
            }
            out.links.push(Link {
                from_table: d.name.clone(),
                from_field: f.name.clone(),
                to_table: to_table.to_string(),
                to_field: to_field.to_string(),
                kind: LinkKind::ForeignKey,
                name: None,
                on_delete: None,
                on_update: None,
                purpose: String::new(),
                seen: Seen::Model,
            });
        }
    }
}

const fn seen_of(database: bool, model: bool) -> Seen {
    match (database, model) {
        (true, false) => Seen::Database,
        (false, true) => Seen::Model,
        _ => Seen::Both,
    }
}

/// A field's place in the keys.
#[derive(Debug, Clone, Copy, Default)]
struct Flags {
    primary: bool,
    unique: bool,
    foreign: bool,
}

/// What a table's constraints and indexes say, field by field.
#[derive(Debug, Default)]
struct Keys {
    /// The rules on each field, in the order met.
    rules: BTreeMap<String, Vec<String>>,
    flags: BTreeMap<String, Flags>,
    /// Rules on several fields at once.
    technical: Vec<String>,
    /// The foreign keys, as links.
    links: Vec<Link>,
}

impl Keys {
    /// Reads a table's constraints and indexes.
    fn of(s: &SqlTable, schema: &Schema) -> Self {
        let mut keys = Self::default();
        for constraint in &s.constraints {
            let named = suffix(constraint.name.as_deref());
            match &constraint.rule {
                Rule::Primary(cols) => {
                    for c in cols {
                        keys.flag(c).primary = true;
                        let rule = if cols.len() == 1 {
                            "primary key".to_string()
                        } else {
                            format!("primary key, with {}", others(cols, c))
                        };
                        keys.push(c, rule);
                    }
                    if cols.len() > 1 {
                        keys.technical
                            .push(format!("primary key ({}){named}", cols.join(", ")));
                    }
                }
                Rule::Unique(cols) => keys.unique(cols, &named),
                Rule::Foreign { .. } => keys.foreign(&s.name, constraint, schema),
                Rule::Check(expr) => {
                    let touched: Vec<&str> = s
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .filter(|c| mentions(expr, c))
                        .collect();
                    if touched.is_empty() {
                        keys.technical.push(format!("check: {expr}{named}"));
                    }
                    for c in touched {
                        keys.push(c, format!("check: {expr}{named}"));
                    }
                }
                Rule::Other(text) => keys.technical.push(text.clone()),
            }
        }
        for index in &s.indexes {
            let named = suffix(index.name.as_deref());
            if index.unique {
                keys.unique(&index.columns, &named);
            } else {
                for c in &index.columns {
                    keys.push(c, format!("indexed{named}"));
                }
            }
        }
        keys
    }

    fn push(&mut self, field: &str, rule: String) {
        self.rules.entry(field.to_string()).or_default().push(rule);
    }

    fn flag(&mut self, field: &str) -> &mut Flags {
        self.flags.entry(field.to_string()).or_default()
    }

    /// A unique constraint or index, on one field or several.
    fn unique(&mut self, cols: &[String], named: &str) {
        if let [c] = cols {
            self.flag(c).unique = true;
            self.push(c, format!("unique{named}"));
            return;
        }
        for c in cols {
            self.push(
                c,
                format!("unique together with {}{named}", others(cols, c)),
            );
        }
        self.technical
            .push(format!("unique ({}){named}", cols.join(", ")));
    }

    /// A foreign key: a rule on each of its fields, and a link each.
    fn foreign(&mut self, from: &str, constraint: &schema_sql::Constraint, schema: &Schema) {
        let Rule::Foreign {
            columns,
            table,
            references,
            on_delete,
            on_update,
        } = &constraint.rule
        else {
            return;
        };
        let named = suffix(constraint.name.as_deref());
        let actions: String = [("on delete", on_delete), ("on update", on_update)]
            .iter()
            .filter_map(|(k, v)| v.as_ref().map(|v| format!(" · {k} {v}")))
            .collect();
        for (k, c) in columns.iter().enumerate() {
            let target = references
                .get(k)
                .cloned()
                .or_else(|| key_of(schema, table))
                .unwrap_or_default();
            self.flag(c).foreign = true;
            let to = if target.is_empty() {
                format!("{table} (its key)")
            } else {
                format!("{table}.{target}")
            };
            self.push(c, format!("references {to}{actions}{named}"));
            self.links.push(Link {
                from_table: from.to_string(),
                from_field: c.clone(),
                to_table: table.clone(),
                to_field: target,
                kind: LinkKind::ForeignKey,
                name: constraint.name.clone(),
                on_delete: on_delete.clone(),
                on_update: on_update.clone(),
                purpose: String::new(),
                seen: Seen::Database,
            });
        }
    }
}

/// ` (name)` after a rule, when the constraint is named.
fn suffix(name: Option<&str>) -> String {
    name.map(|n| format!(" ({n})")).unwrap_or_default()
}

/// One table merged, and the foreign keys the database puts on it.
fn table_of(
    name: &str,
    s: Option<&SqlTable>,
    d: Option<&DeclaredTable>,
    schema: &Schema,
    compared: bool,
    divergences: &mut Vec<String>,
) -> (Table, Vec<Link>) {
    let keys = s.map(|t| Keys::of(t, schema)).unwrap_or_default();
    let columns: Vec<&SqlColumn> = s.map(|t| t.columns.iter().collect()).unwrap_or_default();
    let decl_fields: Vec<&DeclaredField> = d.map(|t| t.fields.iter().collect()).unwrap_or_default();
    let mut names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    for f in &decl_fields {
        if !names.contains(&f.name.as_str()) {
            names.push(&f.name);
        }
    }
    let fields = names
        .into_iter()
        .map(|field| {
            let col = columns.iter().find(|c| c.name == field).copied();
            let decl = decl_fields.iter().find(|f| f.name == field).copied();
            let merged = field_of(field, col, decl, &keys, schema);
            if compared {
                field_divergences(
                    &format!("{name}.{field}"),
                    &merged,
                    col,
                    decl,
                    s.is_some() && d.is_some(),
                    divergences,
                );
            }
            merged
        })
        .collect();
    let table = Table {
        name: name.to_string(),
        purpose: d
            .map(|t| t.purpose.clone())
            .filter(|p| !p.is_empty())
            .or_else(|| s.and_then(|t| t.comment.clone()))
            .unwrap_or_default(),
        business: d.map(|t| t.business.clone()).unwrap_or_default(),
        technical: keys.technical,
        fields,
        seen: seen_of(s.is_some(), d.is_some()),
    };
    (table, keys.links)
}

/// One field merged: the database's structure, the model's words.
fn field_of(
    name: &str,
    col: Option<&SqlColumn>,
    decl: Option<&DeclaredField>,
    keys: &Keys,
    schema: &Schema,
) -> Field {
    let flags = keys.flags.get(name).copied().unwrap_or_default();
    let mut technical = Vec::new();
    if let Some(col) = col {
        if !col.nullable && !flags.primary {
            technical.push("not null".to_string());
        }
        if let Some(default) = &col.default {
            technical.push(format!("default {default}"));
        }
        if let Some(generated) = &col.generated {
            technical.push(generated.clone());
        }
        let bare = col
            .data_type
            .strip_prefix("public.")
            .unwrap_or(&col.data_type);
        if let Some(values) = schema.enums.get(bare) {
            technical.push(format!("one of: {}", values.join(", ")));
        }
    }
    technical.extend(keys.rules.get(name).cloned().unwrap_or_default());
    for rule in decl.map(|f| f.technical.as_slice()).unwrap_or_default() {
        if !technical.contains(rule) {
            technical.push(rule.clone());
        }
    }
    let sql_type = col.map(|c| c.data_type.clone());
    let decl_type = decl.and_then(|f| f.data_type.clone());
    let declared_type = match (&sql_type, &decl_type) {
        (Some(a), Some(b)) if canonical_type(a) != canonical_type(b) => Some(b.clone()),
        _ => None,
    };
    Field {
        name: name.to_string(),
        data_type: sql_type.or(decl_type).unwrap_or_default(),
        declared_type,
        nullable: col
            .map(|c| c.nullable)
            .or_else(|| decl.and_then(|f| f.nullable)),
        default: col.and_then(|c| c.default.clone()),
        purpose: decl
            .map(|f| f.purpose.clone())
            .filter(|p| !p.is_empty())
            .or_else(|| col.and_then(|c| c.comment.clone()))
            .unwrap_or_default(),
        technical,
        business: decl.map(|f| f.business.clone()).unwrap_or_default(),
        primary: flags.primary,
        unique: flags.unique,
        foreign: flags.foreign || decl.is_some_and(|f| f.references.is_some()),
        seen: seen_of(col.is_some(), decl.is_some()),
    }
}

/// Where the two sides disagree on one field. A table only one side has is
/// one divergence, said once for the table, not once per field.
fn field_divergences(
    at: &str,
    field: &Field,
    col: Option<&SqlColumn>,
    decl: Option<&DeclaredField>,
    both_tables: bool,
    out: &mut Vec<String>,
) {
    if let Some(b) = &field.declared_type {
        out.push(format!(
            "{at}: the database says {}, the model says {b}",
            field.data_type
        ));
    }
    if let (Some(a), Some(b)) = (col.map(|c| c.nullable), decl.and_then(|f| f.nullable))
        && a != b
        && !field.primary
    {
        out.push(format!(
            "{at}: the database {} it empty, the model {}",
            if a { "lets" } else { "forbids" },
            if b { "lets it" } else { "forbids it" }
        ));
    }
    if both_tables {
        match field.seen {
            Seen::Database => out.push(format!("field {at} is in the database, not in the model")),
            Seen::Model => out.push(format!("field {at} is in the model, not in the database")),
            Seen::Both => {}
        }
    }
}

fn others(cols: &[String], c: &str) -> String {
    cols.iter()
        .filter(|x| *x != c)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

/// The single-column primary key of `table`, for a reference that leaves it implicit.
fn key_of(schema: &Schema, table: &str) -> Option<String> {
    schema
        .tables
        .iter()
        .find(|t| t.name == table)?
        .constraints
        .iter()
        .find_map(|k| match &k.rule {
            Rule::Primary(cols) if cols.len() == 1 => cols.first().cloned(),
            _ => None,
        })
}

/// Whether `expr` names `column` as a whole word.
fn mentions(expr: &str, column: &str) -> bool {
    expr.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word.eq_ignore_ascii_case(column))
}

/// A type in one spelling, so `int4` and `integer` agree.
#[must_use]
pub fn canonical_type(t: &str) -> String {
    let t = t.trim().to_lowercase();
    let mut t = t.as_str();
    while let Some(rest) = t.strip_prefix("public.") {
        t = rest;
    }
    let cut = t.find(['(', '[']).unwrap_or(t.len());
    let (base, rest) = t.split_at(cut);
    let base = base.split_whitespace().collect::<Vec<_>>().join(" ");
    let base = match base.as_str() {
        "int" | "int4" | "serial" | "serial4" => "integer",
        "int8" | "bigserial" | "serial8" => "bigint",
        "int2" | "smallserial" | "serial2" => "smallint",
        "bool" => "boolean",
        "varchar" => "character varying",
        "char" => "character",
        "timestamptz" => "timestamp with time zone",
        "timestamp" => "timestamp without time zone",
        "timetz" => "time with time zone",
        "time" => "time without time zone",
        "float8" | "float" => "double precision",
        "float4" => "real",
        "decimal" => "numeric",
        other => other,
    };
    let rest: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    format!("{base}{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SQL: &str = "
        CREATE TYPE public.class_kind AS ENUM ('fighter', 'wizard');
        CREATE TABLE public.campaign (id uuid NOT NULL, name text NOT NULL);
        CREATE TABLE public.character (
            id integer NOT NULL,
            campaign_id uuid NOT NULL,
            class public.class_kind NOT NULL,
            name varchar(40),
            CONSTRAINT name_set CHECK (length(name) > 0)
        );
        CREATE TABLE public.log (at timestamptz);
        ALTER TABLE ONLY public.campaign ADD CONSTRAINT campaign_pkey PRIMARY KEY (id);
        ALTER TABLE ONLY public.character ADD CONSTRAINT character_pkey PRIMARY KEY (id);
        ALTER TABLE ONLY public.character ADD CONSTRAINT fk FOREIGN KEY (campaign_id) REFERENCES public.campaign(id) ON DELETE CASCADE;
        CREATE UNIQUE INDEX one_name ON public.character (campaign_id, name);
        COMMENT ON COLUMN public.character.name IS 'from the database';
    ";

    const MODEL: &str = r#"{
        "version": 1,
        "tables": [
            {"name": "campaign", "purpose": "A story", "fields": [
                {"name": "id", "type": "uuid"},
                {"name": "name", "type": "text", "nullable": false, "business": ["shown on the cover"]}
            ]},
            {"name": "character", "purpose": "A hero", "business": ["at most 6 per campaign"], "fields": [
                {"name": "id", "type": "int4"},
                {"name": "campaign_id", "type": "uuid", "references": "campaign.id"},
                {"name": "class", "type": "class_kind"},
                {"name": "name", "type": "text", "purpose": "What the table calls them"},
                {"name": "inspiration", "type": "boolean", "purpose": "Not built yet"}
            ]},
            {"name": "rulebook", "fields": [{"name": "code", "type": "text"}]}
        ],
        "links": [{"from": "character.class", "to": "rulebook.code", "purpose": "the rules of the class"}]
    }"#;

    fn field<'a>(model: &'a DataModel, table: &str, field: &str) -> &'a Field {
        model
            .tables
            .iter()
            .find(|t| t.name == table)
            .and_then(|t| t.fields.iter().find(|f| f.name == field))
            .expect("the field is drawn")
    }

    #[test]
    fn both_sides_merge_and_their_disagreements_are_listed() {
        let model = build(
            "acme/dnd @ main_agent",
            &Ok(Some(SQL.into())),
            &Ok(Some(MODEL.into())),
        );
        assert!(model.compared);
        assert_eq!(
            model
                .tables
                .iter()
                .map(|t| (t.name.as_str(), t.seen))
                .collect::<Vec<_>>(),
            [
                ("campaign", Seen::Both),
                ("character", Seen::Both),
                ("log", Seen::Database),
                ("rulebook", Seen::Model)
            ]
        );

        let id = field(&model, "character", "id");
        assert!(id.primary);
        assert_eq!(id.declared_type, None, "int4 is integer");

        let name = field(&model, "character", "name");
        assert_eq!(
            name.purpose, "What the table calls them",
            "the model's purpose wins"
        );
        assert_eq!(name.declared_type.as_deref(), Some("text"));
        assert!(
            name.technical
                .contains(&"check: length(name) > 0 (name_set)".to_string())
        );
        assert!(
            name.technical
                .contains(&"unique together with campaign_id (one_name)".to_string())
        );

        let class = field(&model, "character", "class");
        assert!(
            class
                .technical
                .contains(&"one of: fighter, wizard".to_string())
        );

        let campaign_id = field(&model, "character", "campaign_id");
        assert!(campaign_id.foreign);
        assert!(
            campaign_id
                .technical
                .contains(&"references campaign.id · on delete cascade (fk)".to_string())
        );

        assert_eq!(field(&model, "character", "inspiration").seen, Seen::Model);
        assert_eq!(
            field(&model, "campaign", "name").business,
            ["shown on the cover"]
        );

        let fk = model
            .links
            .iter()
            .find(|l| l.kind == LinkKind::ForeignKey)
            .expect("the key is a link");
        assert_eq!(fk.seen, Seen::Both, "the model intends the database's key");
        assert!(
            model
                .links
                .iter()
                .any(|l| l.kind == LinkKind::Semantic && l.to_table == "rulebook")
        );

        assert_eq!(
            model.divergences,
            [
                "character.name: the database says varchar(40), the model says text",
                "field character.inspiration is in the model, not in the database",
                "table log is in the database, not in the model",
                "table rulebook is in the model, not in the database",
            ]
        );
    }

    #[test]
    fn one_side_alone_draws_without_divergences() {
        let model = build("here", &Ok(Some(SQL.into())), &Ok(None));
        assert!(!model.compared);
        assert_eq!(model.divergences, Vec::<String>::new());
        assert_eq!(
            field(&model, "character", "name").purpose,
            "from the database"
        );
        assert!(!model.sources[1].found);

        let model = build("here", &Ok(None), &Ok(Some(MODEL.into())));
        assert_eq!(field(&model, "character", "id").data_type, "int4");
        assert_eq!(model.divergences, Vec::<String>::new());
    }

    #[test]
    fn a_broken_model_says_why_and_the_database_still_draws() {
        let model = build("here", &Ok(Some(SQL.into())), &Ok(Some("{ nope".into())));
        assert!(
            model.sources[1]
                .problem
                .as_deref()
                .is_some_and(|p| p.starts_with("not the model's JSON"))
        );
        assert_eq!(model.tables.len(), 3);
        let model = build("here", &Err("gh refused".into()), &Ok(None));
        assert_eq!(model.sources[0].problem.as_deref(), Some("gh refused"));
        assert_eq!(model.tables.len(), 0);
    }

    #[test]
    fn spellings_of_a_type_agree() {
        assert_eq!(canonical_type("INT4"), "integer");
        assert_eq!(canonical_type("varchar( 40 )"), "character varying(40)");
        assert_eq!(
            canonical_type("character varying(40)"),
            "character varying(40)"
        );
        assert_eq!(
            canonical_type("timestamptz"),
            canonical_type("timestamp with time zone")
        );
        assert_eq!(canonical_type("public.class_kind"), "class_kind");
        assert_eq!(canonical_type("int[]"), "integer[]");
        assert_ne!(canonical_type("text"), canonical_type("varchar"));
    }

    proptest! {
        #[test]
        fn a_canonical_type_is_its_own_canonical(t in "[a-zA-Z0-9 ()\\[\\],.]{0,30}") {
            let once = canonical_type(&t);
            prop_assert_eq!(canonical_type(&once), once);
        }

        #[test]
        fn any_pair_of_files_builds(sql in "\\PC{0,200}", json in "\\PC{0,200}") {
            let _ = build("x", &Ok(Some(sql)), &Ok(Some(json)));
        }
    }
}
