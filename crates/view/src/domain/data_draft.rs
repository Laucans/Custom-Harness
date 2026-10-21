//! A creation's data model: the one the proof of concept needs — the
//! tables and fields (contract M) and the aggregates' integrity rules
//! (contract I: criticality, invariants, relation policies).
//!
//! Drafted **before the mock-up is drawn**, off the brief, the decisions
//! and the scope, so every page and fixture the sessions draw is shaped
//! after it (a mock-up drawn first gave `ForgeDesMondes` slug ids where the
//! columns were uuid). Then, at the data step after the rules, refined off
//! the rules, the cut and the fixtures, and reviewed by the human before
//! the factory is initialized.
//!
//! Four gates keep it from validation: a business rule no invariant or
//! derived value carries, a foreign key with no policy, a write touching a
//! field the schema has not, a field of an aggregate with no criticality.
//! The structure changes through the AI only; the human retouches the
//! words and picks the policies. Pure: the asking is the desk's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::creation::{Context, Creation, Family};

/// The data model drafted for the proof of concept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataDraft {
    /// The tables, as contract M describes them.
    pub tables: Vec<Table>,
    /// The aggregates' integrity rules, after contract I.
    pub aggregates: Vec<Aggregate>,
    /// The fields each `DataCapability` writes, `table.field`.
    pub writes: BTreeMap<String, Vec<String>>,
    /// The rules it was drafted from (`rules_print`).
    pub from: String,
    /// The human validated it: the gates pass and nothing moved since.
    pub validated: bool,
}

/// One table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Table {
    /// Its name, `snake_case`.
    pub name: String,
    /// What it is for, in a sentence.
    pub purpose: String,
    /// Business rules on the whole table.
    pub business: Vec<String>,
    /// Its fields.
    pub fields: Vec<Field>,
}

/// One field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Field {
    /// Its name.
    pub name: String,
    /// Its Postgres type.
    #[serde(rename = "type")]
    pub kind: String,
    /// It may be empty.
    pub nullable: bool,
    /// What it is for, in a sentence.
    pub purpose: String,
    /// Business rules on it.
    pub business: Vec<String>,
    /// `table.field` it is a foreign key to; empty when none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub references: String,
}

/// One aggregate's integrity rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Aggregate {
    /// Its name, `PascalCase` — `aggregate` as contract I writes it.
    #[serde(rename = "aggregate")]
    pub name: String,
    /// The tables it is made of.
    pub tables: Vec<String>,
    /// Each field's rule, by `table.field`.
    pub fields: BTreeMap<String, FieldRule>,
    /// What the `DataGuard` checks on it.
    pub invariants: Vec<Invariant>,
    /// What happens to a link when the row it points at goes or changes.
    pub relations: Vec<Relation>,
}

/// How a field is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldRule {
    /// Derived (recomputed) or authoritative (written).
    pub kind: Option<ValueKind>,
    /// How hard a change to it is looked at.
    pub criticality: Option<Criticality>,
    /// The business rules a derived value carries, by number.
    pub rules: Vec<u32>,
}

/// Recomputed, or read as canonical data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    /// Recomputed from other data.
    Derived,
    /// Canonical: written through a `DataCapability` only.
    Authoritative,
}

/// How hard a change to a field is looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Criticality {
    /// Merged without asking.
    Low,
    /// The usual checks.
    Normal,
    /// An approver reviews it.
    Critical,
}

/// One invariant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Invariant {
    /// Its id, `kebab-case`.
    pub id: String,
    /// The rule, in a sentence.
    pub rule: String,
    /// What a violation does.
    #[serde(rename = "onViolation")]
    pub on_violation: Option<OnViolation>,
    /// The business rules it carries, by number.
    pub rules: Vec<u32>,
}

/// What a violated invariant does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnViolation {
    /// The command is refused.
    Reject,
    /// A human decides.
    HumanReview,
}

/// A link's policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Relation {
    /// The foreign key, `table.field`.
    pub from: String,
    /// When the row it points at goes.
    #[serde(rename = "onDelete")]
    pub on_delete: Option<OnDelete>,
    /// When the row it points at changes its key.
    #[serde(rename = "onUpdate")]
    pub on_update: Option<OnUpdate>,
}

/// When the row a link points at goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnDelete {
    /// The deletion is refused.
    Restrict,
    /// The linked rows go too.
    Cascade,
    /// The link is emptied.
    Nullify,
    /// The linked rows are archived.
    Archive,
    /// A human decides.
    Review,
}

/// When the row a link points at changes its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnUpdate {
    /// The link follows.
    Propagate,
    /// The change is refused.
    Restrict,
}

impl DataDraft {
    /// What keeps it from validation, in words; empty when nothing.
    #[must_use]
    pub fn gaps(&self, c: &Creation) -> Vec<String> {
        if self.tables.is_empty() {
            return vec!["no table yet".to_string()];
        }
        let mut gaps = Vec::new();
        let carried: BTreeSet<u32> = self
            .aggregates
            .iter()
            .flat_map(|a| {
                a.invariants
                    .iter()
                    .flat_map(|i| i.rules.iter())
                    .chain(a.fields.values().flat_map(|f| f.rules.iter()))
            })
            .copied()
            .collect();
        for r in c
            .requirements
            .iter()
            .filter(|r| r.family == Family::Business && !carried.contains(&r.id))
        {
            gaps.push(format!(
                "R{} is carried by no invariant and no derived value",
                r.id
            ));
        }
        let policies: BTreeMap<&str, &Relation> = self
            .aggregates
            .iter()
            .flat_map(|a| a.relations.iter())
            .map(|r| (r.from.as_str(), r))
            .collect();
        for t in &self.tables {
            for f in t.fields.iter().filter(|f| !f.references.is_empty()) {
                let key = format!("{}.{}", t.name, f.name);
                let set = policies
                    .get(key.as_str())
                    .is_some_and(|r| r.on_delete.is_some() && r.on_update.is_some());
                if !set {
                    gaps.push(format!("{key} has no onDelete and onUpdate policy"));
                }
            }
        }
        let fields = self.field_names();
        for (write, touched) in &self.writes {
            for field in touched.iter().filter(|f| !fields.contains(f.as_str())) {
                gaps.push(format!("{write} writes {field}, which the schema has not"));
            }
        }
        for a in &self.aggregates {
            if a.fields.is_empty() {
                gaps.push(format!("{} declares no field", a.name));
            }
            for (field, rule) in &a.fields {
                if rule.criticality.is_none() {
                    gaps.push(format!("{}: {field} has no criticality", a.name));
                }
            }
        }
        gaps
    }

    /// Every field, `table.field`.
    fn field_names(&self) -> BTreeSet<String> {
        self.tables
            .iter()
            .flat_map(|t| {
                t.fields
                    .iter()
                    .map(move |f| format!("{}.{}", t.name, f.name))
            })
            .collect()
    }

    /// The tables as `data/model.json` writes them (contract M).
    ///
    /// # Errors
    ///
    /// Never in practice: plain data.
    pub fn model_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "tables": self.tables,
            "links": [],
        }))
        .map_err(|e| e.to_string())
    }

    /// Each aggregate as contract I has it — one `aggregate.json` apiece,
    /// by aggregate name: every field of its tables keyed in `camelCase`
    /// (the table's name joined in when two of its tables share a field's
    /// name), its kind and criticality as drafted — authoritative and normal
    /// where the draft left them open —, its invariants with a `kebab-case`
    /// id, each relation from `<Aggregate>.<field>`, and holds of a quarter
    /// of an hour, renewable. What the task building the aggregate installs
    /// at `crates/<system>/aggregates/<Aggregate>/aggregate.json`.
    #[must_use]
    pub fn aggregate_manifests(&self) -> Vec<(String, serde_json::Value)> {
        self.aggregates
            .iter()
            .map(|a| (a.name.clone(), self.manifest_of(a)))
            .collect()
    }

    /// One aggregate as contract I has it.
    fn manifest_of(&self, a: &Aggregate) -> serde_json::Value {
        let owned: Vec<(&str, &str)> = self
            .tables
            .iter()
            .filter(|t| a.tables.contains(&t.name))
            .flat_map(|t| {
                t.fields
                    .iter()
                    .map(move |f| (t.name.as_str(), f.name.as_str()))
            })
            .collect();
        let keys = field_keys(&owned);
        let key_of = |table: &str, field: &str| {
            keys.get(&(table.to_string(), field.to_string()))
                .cloned()
                .unwrap_or_else(|| camel(field))
        };
        let mut fields = serde_json::Map::new();
        for (table, field) in &owned {
            let rule = a.fields.get(&format!("{table}.{field}"));
            let kind = rule
                .and_then(|r| r.kind)
                .unwrap_or(ValueKind::Authoritative);
            let criticality = rule
                .and_then(|r| r.criticality)
                .unwrap_or(Criticality::Normal);
            fields.insert(
                key_of(table, field),
                serde_json::json!({ "kind": kind, "criticality": criticality }),
            );
        }
        let invariants: Vec<serde_json::Value> = a
            .invariants
            .iter()
            .map(|i| {
                serde_json::json!({
                    "id": kebab(&i.id),
                    "rule": i.rule.trim(),
                    "onViolation": i.on_violation.unwrap_or(OnViolation::Reject),
                })
            })
            .collect();
        let relations: Vec<serde_json::Value> = a
            .relations
            .iter()
            .map(|r| {
                let (table, field) = r.from.split_once('.').unwrap_or(("", &r.from));
                let mut relation = serde_json::json!({
                    "from": format!("{}.{}", a.name, key_of(table, field)),
                    "onDelete": r.on_delete.unwrap_or(OnDelete::Restrict),
                });
                if let Some(on_update) = r.on_update {
                    relation["onUpdate"] = serde_json::json!(on_update);
                }
                relation
            })
            .collect();
        serde_json::json!({
            "aggregate": a.name,
            "fields": fields,
            "invariants": invariants,
            "relations": relations,
            "holds": { "maxTtl": "15m", "renewable": true },
        })
    }

    /// The human's retouches applied: words on what exists, the policies
    /// picked. Anything it names that does not exist is left alone; any
    /// change takes the validation back.
    pub fn edit(&mut self, edit: DataEdit) {
        let before = self.clone();
        for t in edit.texts {
            let Some(table) = self.tables.iter_mut().find(|x| x.name == t.table) else {
                continue;
            };
            if t.field.is_empty() {
                table.purpose = t.purpose;
            } else if let Some(field) = table.fields.iter_mut().find(|f| f.name == t.field) {
                field.purpose = t.purpose;
            }
        }
        for p in edit.criticalities {
            if let Some(rule) = self
                .aggregates
                .iter_mut()
                .find(|a| a.name == p.aggregate)
                .and_then(|a| a.fields.get_mut(&p.field))
            {
                rule.criticality = Some(p.criticality);
            }
        }
        for r in edit.relations {
            if let Some(kept) = self
                .aggregates
                .iter_mut()
                .flat_map(|a| a.relations.iter_mut())
                .find(|x| x.from == r.from)
            {
                kept.on_delete = r.on_delete.or(kept.on_delete);
                kept.on_update = r.on_update.or(kept.on_update);
            }
        }
        if *self != before {
            self.validated = false;
        }
    }
}

/// The human's retouches, as the page sends them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct DataEdit {
    /// What a table or a field is for.
    pub texts: Vec<TextEdit>,
    /// A field's criticality.
    pub criticalities: Vec<CriticalityEdit>,
    /// A link's policies.
    pub relations: Vec<Relation>,
}

/// What a table, or one of its fields, is for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TextEdit {
    /// The table.
    pub table: String,
    /// The field; empty for the table itself.
    pub field: String,
    /// The new purpose.
    pub purpose: String,
}

/// A field's criticality, picked.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CriticalityEdit {
    /// The aggregate.
    pub aggregate: String,
    /// The field, `table.field`.
    pub field: String,
    /// The criticality.
    pub criticality: Criticality,
}

/// What the rules are, hashed: a draft made from other rules is stale.
#[must_use]
pub fn rules_print(c: &Creation) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for r in &c.requirements {
        (r.id, &r.title, &r.body, &r.criterion, r.family).hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

/// The shape of the answer, given to the AI.
const SHAPE: &str = r#"{"tables": [{"name": "campaign", "purpose": "…", "business": ["…"], "fields": [{"name": "id", "type": "uuid", "nullable": false, "purpose": "…", "business": [], "references": "other_table.id"}]}], "aggregates": [{"aggregate": "Campaign", "tables": ["campaign"], "fields": {"campaign.name": {"kind": "authoritative", "criticality": "normal", "rules": []}}, "invariants": [{"id": "name-unique", "rule": "…", "onViolation": "reject", "rules": [3]}], "relations": [{"from": "session.campaign_id", "onDelete": "cascade", "onUpdate": "propagate"}]}], "writes": {"system.verbThing": ["campaign.name"]}}"#;

/// The data model as a drawing session reads it, read-only: the tables and
/// fields its fixtures take their shape from, the writes its actuators name.
#[must_use]
pub fn model_part(draft: &DataDraft) -> String {
    let mut text = String::from(
        "\nThe data model the proof of concept is built on, drafted already — read it, never \
         write under `mock/data/`: shape every fixture after its tables and fields (its ids as \
         the id columns are typed, its names as the fields are named), and name the \
         DataCapabilities an actuator triggers after its `writes`:\n\n",
    );
    for t in &draft.tables {
        let fields: Vec<String> = t
            .fields
            .iter()
            .map(|f| {
                let mut field = format!("{} {}", f.name, f.kind);
                if !f.references.is_empty() {
                    let _ = write!(field, " → {}", f.references);
                }
                field
            })
            .collect();
        let _ = writeln!(
            text,
            "- `{}` — {} — {}",
            t.name,
            t.purpose.trim(),
            fields.join(", ")
        );
    }
    if !draft.writes.is_empty() {
        text.push_str("\nThe writes (DataCapability — fields):\n\n");
        for (id, fields) in &draft.writes {
            let _ = writeln!(text, "- `{id}` — {}", fields.join(", "));
        }
    }
    text
}

/// The question that drafts the data model first, before any page is
/// drawn: off the brief, the decisions and the scope only — the rules and
/// the mock-up come after it, and refine it.
#[must_use]
pub fn early_data_prompt(c: &Creation) -> String {
    let mut text = String::from(
        "You lay the data model of a proof of concept, before its mock-up is drawn, for a \
         vertical-slice architecture whose data is protected by a single write frontier (a \
         DataGuard): the tables and fields in PostgreSQL terms, then the aggregates' integrity \
         rules. Every thing the application keeps is a table; every field with its Postgres type \
         (ids are `uuid`), its nullability and what it is for; every foreign key named under \
         `references` and given, in its aggregate's `relations`, an `onDelete` (restrict, \
         cascade, nullify, archive, review) and an `onUpdate` (propagate, restrict) policy. \
         Group the tables into aggregates (one per thing a command changes as a whole); give \
         every field of an aggregate a `kind` (derived or authoritative) and a `criticality` \
         (low, normal, critical). Prefer derived values to stored totals. List under `writes` \
         the commands the screens will trigger, `<system>.<verbThing>`, each with the fields it \
         changes. The pages, fixtures and micro-frontends are drawn after this model and take \
         their shape from it: name things as the screens will. Write purposes and rules in the \
         language of the brief. Answer with one JSON object and nothing else, shaped like \
         this:\n\n",
    );
    text.push_str(SHAPE);
    text.push('\n');
    text.push_str(&super::creation::product(c));
    let _ = write!(
        text,
        "\nThe proof of concept's scope:\n\n{}\n",
        c.scope.trim()
    );
    text
}

/// The question that drafts the data model at the data step — refining the
/// one laid before the mock-up, when there is one, off the rules, the cut
/// and the fixtures; drafting it whole otherwise.
#[must_use]
pub fn data_prompt(
    c: &Creation,
    ctx: &Context,
    fixtures: &[(String, String)],
    draft: Option<&DataDraft>,
) -> String {
    let mut text = String::from(
        "You draft the data model of a proof of concept, for a vertical-slice architecture \
         whose data is protected by a single write frontier (a DataGuard): the tables and \
         fields in PostgreSQL terms, then the aggregates' integrity rules. Every table a \
         micro-frontend's Capabilities read or its DataCapabilities write; every field with its \
         Postgres type, its nullability and what it is for; every foreign key named under \
         `references` and given, in its aggregate's `relations`, an `onDelete` (restrict, \
         cascade, nullify, archive, review) and an `onUpdate` (propagate, restrict) policy. \
         Group the tables into aggregates (one per thing a command changes as a whole); give \
         every field of an aggregate a `kind` (derived or authoritative) and a `criticality` \
         (low, normal, critical). Every business rule below is carried by an invariant or by \
         a derived value, citing its number under `rules`. Prefer derived values to stored \
         totals, so adding a row stays a pure addition. List under `writes` the fields every \
         DataCapability changes. Write purposes and rules in the language of the brief. Answer \
         with one JSON object and nothing else, shaped like this:\n\n",
    );
    text.push_str(SHAPE);
    if let Some(draft) = draft {
        let current = serde_json::to_string_pretty(&serde_json::json!({
            "tables": draft.tables,
            "aggregates": draft.aggregates,
            "writes": draft.writes,
        }))
        .unwrap_or_default();
        let _ = write!(
            text,
            "\n\nThe model laid before the mock-up was drawn — refine it off the rules, the cut \
             and the example data below; **keep its table and field names** (the fixtures and \
             the micro-frontends are shaped after them), add what the rules need, drop nothing \
             a fixture reads:\n\n```json\n{current}\n```\n"
        );
    }
    let _ = write!(
        text,
        "\n\nWhat the application does:\n\n{}\n",
        c.brief.trim()
    );
    text.push_str("\nThe rules (number — family — rule — criterion):\n\n");
    for r in &c.requirements {
        let family = if r.family == Family::Business {
            "business"
        } else {
            "journey"
        };
        let _ = writeln!(
            text,
            "- R{} — {family} — {} — {}",
            r.id,
            r.title.trim(),
            r.criterion.line()
        );
    }
    text.push_str("\nThe micro-frontends (name — system — needs — actions):\n\n");
    for m in &ctx.components {
        let _ = writeln!(
            text,
            "- `{}` — {} — {} — {}",
            m.name,
            m.system,
            m.needs.join(", "),
            m.actions.join(", ")
        );
    }
    if !fixtures.is_empty() {
        text.push_str("\nThe example data the mock-up shows:\n");
        for (path, body) in fixtures {
            let _ = write!(text, "\n`{path}`:\n```json\n{}\n```\n", body.trim());
        }
    }
    text
}

/// The question that changes the draft's structure as the human asks.
///
/// # Errors
///
/// Never in practice: the draft is plain data.
pub fn change_prompt(c: &Creation, draft: &DataDraft, request: &str) -> Result<String, String> {
    let current = serde_json::to_string_pretty(&serde_json::json!({
        "tables": draft.tables,
        "aggregates": draft.aggregates,
        "writes": draft.writes,
    }))
    .map_err(|e| e.to_string())?;
    let mut text = String::from(
        "You keep the data model of a proof of concept. The human asks for a change: make it, \
         keep everything else as it is, keep every foreign key with its policies, every field \
         of an aggregate with its kind and criticality, every business rule carried by an \
         invariant or a derived value. Answer with the whole model as one JSON object and \
         nothing else, the same shape as the current one.\n\n",
    );
    let _ = write!(
        text,
        "The change asked:\n\n{}\n\nThe current model:\n\n{current}\n",
        request.trim()
    );
    text.push_str("\nThe business rules (number — rule):\n\n");
    for r in c
        .requirements
        .iter()
        .filter(|r| r.family == Family::Business)
    {
        let _ = writeln!(text, "- R{} — {}", r.id, r.title.trim());
    }
    Ok(text)
}

/// The draft out of the AI's answer, made from `from`.
///
/// # Errors
///
/// The answer carries no JSON object, one that does not read, or no table.
pub fn parse_data(answer: &str, from: &str) -> Result<DataDraft, String> {
    let start = answer.find('{').ok_or("the AI answered no data model")?;
    let end = answer.rfind('}').ok_or("the AI answered no data model")?;
    let object = answer
        .get(start..=end)
        .ok_or("the AI answered no data model")?;
    let mut draft: DataDraft = serde_json::from_str(object)
        .map_err(|e| format!("the AI's data model is not readable: {e}"))?;
    if draft.tables.is_empty() {
        return Err("the AI drafted no table".to_string());
    }
    draft.from = from.to_string();
    draft.validated = false;
    Ok(draft)
}

/// Each `(table, field)` of an aggregate keyed once in `camelCase`: by the
/// field's name, the table's joined in where two would share a key, a
/// number appended where even that is not enough.
fn field_keys(owned: &[(&str, &str)]) -> BTreeMap<(String, String), String> {
    let mut keys: Vec<((String, String), String, bool)> = owned
        .iter()
        .map(|(t, f)| (((*t).to_string(), (*f).to_string()), camel(f), false))
        .collect();
    for _ in 0..owned.len() {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (_, key, _) in &keys {
            *counts.entry(key.clone()).or_default() += 1;
        }
        let clashing = |key: &String| counts.get(key).is_some_and(|n| *n > 1);
        if !keys.iter().any(|(_, key, _)| clashing(key)) {
            break;
        }
        // The plainer ones yield first: a field's own name is joined to its
        // table's; one already joined takes a number.
        let plain_clash = keys.iter().any(|(_, key, joined)| !joined && clashing(key));
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for ((table, field), key, joined) in &mut keys {
            if !clashing(key) {
                continue;
            }
            if !*joined {
                *key = camel(&format!("{table}_{field}"));
                *joined = true;
            } else if !plain_clash {
                let n = seen.entry(key.clone()).or_default();
                *n += 1;
                if *n > 1 {
                    *key = format!("{key}{n}");
                }
            }
        }
    }
    keys.into_iter().map(|(at, key, _)| (at, key)).collect()
}

/// A `snake_case` or spaced name in `camelCase`: `credit_limit` → `creditLimit`.
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if out.is_empty() {
                out.push(c.to_ascii_lowercase());
            } else if upper {
                out.push(c.to_ascii_uppercase());
            } else {
                out.push(c);
            }
            upper = false;
        } else {
            upper = true;
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'f');
    }
    out
}

/// A name in `kebab-case`, starting with a letter: `Max 3 PJ` → `max-3-pj`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.starts_with(|c: char| c.is_ascii_lowercase()) {
        out
    } else {
        format!("i-{out}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::creation::{ComponentRef, Requirement};

    fn business(id: u32) -> Requirement {
        Requirement {
            id,
            title: format!("rule {id}"),
            family: Family::Business,
            criterion: crate::domain::creation::Criterion::from_sentence("given a, when b, then c"),
            ..Requirement::default()
        }
    }

    fn draft() -> DataDraft {
        let field = |name: &str, references: &str| Field {
            name: name.to_string(),
            kind: "uuid".to_string(),
            references: references.to_string(),
            ..Field::default()
        };
        DataDraft {
            tables: vec![
                Table {
                    name: "campaign".to_string(),
                    fields: vec![field("id", ""), field("name", "")],
                    ..Table::default()
                },
                Table {
                    name: "session".to_string(),
                    fields: vec![field("id", ""), field("campaign_id", "campaign.id")],
                    ..Table::default()
                },
            ],
            aggregates: vec![Aggregate {
                name: "Campaign".to_string(),
                tables: vec!["campaign".to_string(), "session".to_string()],
                fields: BTreeMap::from([(
                    "campaign.name".to_string(),
                    FieldRule {
                        kind: Some(ValueKind::Authoritative),
                        criticality: Some(Criticality::Normal),
                        rules: vec![2],
                    },
                )]),
                invariants: vec![Invariant {
                    id: "one".to_string(),
                    rule: "r".to_string(),
                    on_violation: Some(OnViolation::Reject),
                    rules: vec![1],
                }],
                relations: vec![Relation {
                    from: "session.campaign_id".to_string(),
                    on_delete: Some(OnDelete::Cascade),
                    on_update: Some(OnUpdate::Propagate),
                }],
            }],
            writes: BTreeMap::from([(
                "campagne.renommer".to_string(),
                vec!["campaign.name".to_string()],
            )]),
            ..DataDraft::default()
        }
    }

    fn creation() -> Creation {
        Creation {
            requirements: vec![
                business(1),
                business(2),
                Requirement {
                    id: 3,
                    ..Requirement::default()
                },
            ],
            ..Creation::default()
        }
    }

    /// Contract I, mirrored strictly: a key the schema has not is refused.
    mod contract_i {
        use std::collections::BTreeMap;

        use serde::Deserialize;

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        // The contract's own names, mirrored.
        #[allow(dead_code, clippy::struct_field_names)]
        pub struct Aggregate {
            pub aggregate: String,
            pub fields: BTreeMap<String, Field>,
            pub invariants: Vec<Invariant>,
            pub relations: Vec<Relation>,
            pub holds: Holds,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        pub struct Field {
            pub kind: String,
            pub criticality: String,
            pub review: Option<String>,
            pub merge: Option<String>,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        #[allow(dead_code)]
        pub struct Invariant {
            pub id: String,
            pub rule: String,
            pub on_violation: String,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        #[allow(dead_code)]
        pub struct Relation {
            pub from: String,
            pub on_delete: String,
            pub on_update: Option<String>,
            pub semantic: Option<bool>,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        #[allow(dead_code)]
        pub struct Holds {
            pub max_ttl: String,
            pub renewable: bool,
        }
    }

    #[test]
    fn each_aggregate_is_drafted_as_contract_i_has_it() {
        let mut d = draft();
        // Two tables sharing a field's name: the table's name is joined in.
        d.aggregates[0].invariants.push(Invariant {
            id: "Max 3 PJ".to_string(),
            rule: " at most three ".to_string(),
            ..Invariant::default()
        });
        d.aggregates[0].relations[0].on_update = None;
        let manifests = d.aggregate_manifests();
        assert_eq!(manifests.len(), 1);
        let (name, manifest) = &manifests[0];
        assert_eq!(name, "Campaign");
        let strict: contract_i::Aggregate =
            serde_json::from_value(manifest.clone()).expect("contract I, strictly");
        assert_eq!(strict.aggregate, "Campaign");
        assert_eq!(
            strict.fields.keys().map(String::as_str).collect::<Vec<_>>(),
            ["campaignId", "name", "sessionCampaignId", "sessionId"],
            "two ids join their tables, and the link then joins its own"
        );
        assert_eq!(manifest["fields"]["name"]["criticality"], "normal");
        assert_eq!(manifest["fields"]["sessionId"]["kind"], "authoritative");
        assert_eq!(strict.invariants[0].id, "one");
        assert_eq!(strict.invariants[1].id, "max-3-pj");
        assert_eq!(strict.invariants[1].rule, "at most three");
        assert_eq!(strict.invariants[1].on_violation, "reject");
        assert_eq!(strict.relations[0].from, "Campaign.sessionCampaignId");
        assert_eq!(strict.relations[0].on_delete, "cascade");
        assert!(strict.relations[0].on_update.is_none());
        assert_eq!(
            (strict.holds.max_ttl.as_str(), strict.holds.renewable),
            ("15m", true)
        );
    }

    #[test]
    fn every_field_gets_one_key_however_names_clash() {
        let keys = field_keys(&[("a", "x"), ("b", "x"), ("a_x", "y"), ("c", "a_x")]);
        let mut all: Vec<&String> = keys.values().collect();
        all.sort();
        let n = all.len();
        all.dedup();
        assert_eq!(all.len(), n, "{keys:?}");
        assert_eq!(keys[&("c".to_string(), "a_x".to_string())], "cAX");
    }

    #[test]
    fn names_become_the_case_the_contract_asks_for() {
        assert_eq!(camel("credit_limit"), "creditLimit");
        assert_eq!(camel("Name"), "name");
        assert_eq!(camel("3d_view"), "f3dView");
        assert_eq!(kebab("Max 3 PJ"), "max-3-pj");
        assert_eq!(kebab("--a__b--"), "a-b");
        assert_eq!(kebab("1st"), "i-1st");
    }

    #[test]
    fn a_complete_draft_has_no_gap_and_each_gate_names_what_fails() {
        let c = creation();
        assert!(draft().gaps(&c).is_empty(), "{:?}", draft().gaps(&c));
        assert_eq!(
            DataDraft::default().gaps(&c),
            vec!["no table yet".to_string()]
        );

        let mut d = draft();
        d.aggregates[0].invariants[0].rules.clear();
        assert_eq!(
            d.gaps(&c),
            vec!["R1 is carried by no invariant and no derived value".to_string()]
        );
        let mut d = draft();
        d.aggregates[0]
            .fields
            .values_mut()
            .for_each(|f| f.rules.clear());
        assert_eq!(
            d.gaps(&c),
            vec!["R2 is carried by no invariant and no derived value".to_string()]
        );

        let mut d = draft();
        d.aggregates[0].relations[0].on_update = None;
        assert_eq!(
            d.gaps(&c),
            vec!["session.campaign_id has no onDelete and onUpdate policy".to_string()]
        );
        let mut d = draft();
        d.aggregates[0].relations[0].on_delete = None;
        assert_eq!(d.gaps(&c).len(), 1);
        let mut d = draft();
        d.aggregates[0].relations.clear();
        assert_eq!(d.gaps(&c).len(), 1);

        let mut d = draft();
        d.writes.insert(
            "campagne.noter".to_string(),
            vec!["campaign.note".to_string(), "campaign.id".to_string()],
        );
        assert_eq!(
            d.gaps(&c),
            vec!["campagne.noter writes campaign.note, which the schema has not".to_string()]
        );

        let mut d = draft();
        d.aggregates[0]
            .fields
            .values_mut()
            .for_each(|f| f.criticality = None);
        assert_eq!(
            d.gaps(&c),
            vec!["Campaign: campaign.name has no criticality".to_string()]
        );
        let mut d = draft();
        d.aggregates[0].fields.clear();
        d.aggregates[0].invariants[0].rules = vec![1, 2];
        assert_eq!(d.gaps(&c), vec!["Campaign declares no field".to_string()]);
    }

    #[test]
    fn the_human_retouches_words_and_policies_and_nothing_that_is_not_there() {
        let mut d = draft();
        d.validated = true;
        d.edit(DataEdit {
            texts: vec![
                TextEdit {
                    table: "campaign".to_string(),
                    field: String::new(),
                    purpose: "the campaigns".to_string(),
                },
                TextEdit {
                    table: "campaign".to_string(),
                    field: "name".to_string(),
                    purpose: "its name".to_string(),
                },
                TextEdit {
                    table: "ghost".to_string(),
                    field: String::new(),
                    purpose: "x".to_string(),
                },
                TextEdit {
                    table: "campaign".to_string(),
                    field: "ghost".to_string(),
                    purpose: "x".to_string(),
                },
            ],
            criticalities: vec![
                CriticalityEdit {
                    aggregate: "Campaign".to_string(),
                    field: "campaign.name".to_string(),
                    criticality: Criticality::Critical,
                },
                CriticalityEdit {
                    aggregate: "Ghost".to_string(),
                    field: "campaign.name".to_string(),
                    criticality: Criticality::Low,
                },
            ],
            relations: vec![Relation {
                from: "session.campaign_id".to_string(),
                on_delete: Some(OnDelete::Restrict),
                on_update: None,
            }],
        });
        assert!(!d.validated);
        assert_eq!(d.tables[0].purpose, "the campaigns");
        assert_eq!(d.tables[0].fields[1].purpose, "its name");
        assert_eq!(d.tables.len(), 2);
        assert_eq!(
            d.aggregates[0].fields["campaign.name"].criticality,
            Some(Criticality::Critical)
        );
        let r = &d.aggregates[0].relations[0];
        assert_eq!(
            (r.on_delete, r.on_update),
            (Some(OnDelete::Restrict), Some(OnUpdate::Propagate))
        );

        let mut same = draft();
        same.validated = true;
        same.edit(DataEdit::default());
        assert!(same.validated, "nothing changed");
    }

    #[test]
    fn the_rules_print_moves_with_the_rules_only() {
        let mut c = creation();
        let print = rules_print(&c);
        c.brief = "another brief".to_string();
        assert_eq!(rules_print(&c), print);
        c.requirements[0].criterion.given = "more".to_string();
        assert_ne!(rules_print(&c), print);
        let again = rules_print(&c);
        c.requirements[2].family = Family::Business;
        assert_ne!(rules_print(&c), again);
    }

    #[test]
    fn the_draft_is_read_off_the_answer_and_written_as_contract_m() {
        let answer = format!("Here it is: {SHAPE} done");
        let d = parse_data(&answer, "print").expect("read");
        assert_eq!(d.from, "print");
        assert!(!d.validated);
        assert_eq!(d.tables[0].fields[0].references, "other_table.id");
        assert_eq!(
            d.aggregates[0].invariants[0].on_violation,
            Some(OnViolation::Reject)
        );
        assert_eq!(
            d.aggregates[0].relations[0].on_delete,
            Some(OnDelete::Cascade)
        );
        assert_eq!(
            d.writes["system.verbThing"],
            vec!["campaign.name".to_string()]
        );
        assert!(parse_data("nothing", "p").is_err());
        assert!(parse_data("{\"tables\": 3}", "p").is_err());
        assert!(parse_data("{\"tables\": []}", "p").is_err());

        let model: serde_json::Value =
            serde_json::from_str(&draft().model_json().expect("json")).expect("json");
        assert_eq!(model["version"], 1);
        assert_eq!(model["tables"][1]["fields"][1]["references"], "campaign.id");
        assert_eq!(model["tables"][1]["fields"][1]["type"], "uuid");
        assert!(
            model["tables"][0]["fields"][0].get("references").is_none(),
            "absent when none"
        );
    }

    #[test]
    fn the_model_part_lists_every_table_its_fields_and_the_writes_read_only() {
        let mut d = draft();
        let part = model_part(&d);
        assert!(part.starts_with("\nThe data model the proof of concept is built on"));
        assert!(part.contains("never write under `mock/data/`"), "{part}");
        for t in &d.tables {
            assert!(part.contains(&format!("- `{}` — ", t.name)), "{part}");
            for f in &t.fields {
                assert!(part.contains(&format!("{} {}", f.name, f.kind)), "{part}");
                if !f.references.is_empty() {
                    assert!(part.contains(&format!("→ {}", f.references)), "{part}");
                }
            }
        }
        assert!(
            d.tables
                .iter()
                .flat_map(|t| &t.fields)
                .any(|f| !f.references.is_empty()),
            "the fixture has a foreign key to show the arrow on"
        );
        assert!(
            part.contains("The writes (DataCapability — fields)"),
            "{part}"
        );
        for (id, fields) in &d.writes {
            assert!(
                part.contains(&format!("- `{id}` — {}", fields.join(", "))),
                "{part}"
            );
        }
        assert!(!d.writes.is_empty(), "the fixture has writes");
        d.writes.clear();
        let bare = model_part(&d);
        assert!(
            !bare.contains("The writes"),
            "no writes, no writes block: {bare}"
        );
        assert!(bare.len() < part.len());
    }

    #[test]
    fn the_questions_carry_the_brief_the_rules_the_cut_the_fixtures_and_the_change() {
        let mut c = creation();
        c.brief = "a campaign manager".to_string();
        let ctx = Context {
            components: vec![ComponentRef {
                name: "List".to_string(),
                system: "campagne".to_string(),
                needs: vec!["campagne.lister".to_string()],
                actions: vec!["campagne.creer".to_string()],
                ..ComponentRef::default()
            }],
            ..Context::default()
        };
        let asked = data_prompt(
            &c,
            &ctx,
            &[("fixtures/List.json".to_string(), " [1] ".to_string())],
            None,
        );
        assert!(asked.contains("a campaign manager"));
        assert!(asked.contains("- R1 — business — rule 1 — given"));
        assert!(asked.contains("- R3 — journey —"));
        assert!(asked.contains("- `List` — campagne — campagne.lister — campagne.creer"));
        assert!(asked.contains("`fixtures/List.json`:\n```json\n[1]\n```"));
        assert!(!data_prompt(&c, &ctx, &[], None).contains("example data"));
        let change = change_prompt(&c, &draft(), " add a note ").expect("asked");
        assert!(change.contains("The change asked:\n\nadd a note\n"));
        assert!(change.contains("\"session.campaign_id\""));
        assert!(change.contains("- R2 — rule 2") && !change.contains("- R3"));
    }
}
