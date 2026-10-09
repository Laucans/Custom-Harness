//! Two data models side by side — two branches, or two versions of one —
//! and what the second has that the first has not, table by table, field by
//! field, link by link.
//!
//! `compare(base, other)` reads as "what `other` changes relative to
//! `base`": a table in `other` only is **added**, one in `base` only is
//! **removed**, one in both whose fields, purpose or rules differ is
//! **changed**. Pure, on the merged [`DataModel`] both sides were built into.

use serde::Serialize;

use crate::model::{DataModel, Field, Link, Table};

/// What happened to one part between the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// In `other` only.
    Added,
    /// In `base` only.
    Removed,
    /// In both, not the same.
    Changed,
}

/// One table that differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableChange {
    /// Its name.
    pub name: String,
    /// What happened to it.
    pub change: Change,
    /// What differs on the table itself: its purpose, its rules.
    pub notes: Vec<String>,
    /// Its fields that differ — every field of a table added or removed.
    pub fields: Vec<FieldChange>,
    /// The table as `base` has it, when it does.
    pub was: Option<Table>,
    /// The table as `other` has it, when it does.
    pub now: Option<Table>,
}

/// One field that differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldChange {
    /// Its name.
    pub name: String,
    /// What happened to it.
    pub change: Change,
    /// What differs, a sentence each: `type integer → text`.
    pub notes: Vec<String>,
    /// The field as `base` has it, when it does.
    pub was: Option<Field>,
    /// The field as `other` has it, when it does.
    pub now: Option<Field>,
}

/// One link that differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkChange {
    /// What happened to it.
    pub change: Change,
    /// What differs on a link both sides have: its kind, its rules.
    pub notes: Vec<String>,
    /// The link, from the side that has it — `other` when both do.
    pub link: Link,
}

/// One view that differs: a view is read or not, nothing finer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewChange {
    /// Its name.
    pub name: String,
    /// Added or removed.
    pub change: Change,
}

/// Everything that differs between two data models.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Comparison {
    /// The tables that differ, `base`'s order then `other`'s additions.
    pub tables: Vec<TableChange>,
    /// The links that differ.
    pub links: Vec<LinkChange>,
    /// The views that differ.
    pub views: Vec<ViewChange>,
}

impl Comparison {
    /// Nothing differs.
    #[must_use]
    pub const fn is_same(&self) -> bool {
        self.tables.is_empty() && self.links.is_empty() && self.views.is_empty()
    }
}

/// What `other` changes relative to `base`.
#[must_use]
pub fn compare(base: &DataModel, other: &DataModel) -> Comparison {
    let mut tables = Vec::new();
    for was in &base.tables {
        match other.tables.iter().find(|t| t.name == was.name) {
            None => tables.push(TableChange {
                name: was.name.clone(),
                change: Change::Removed,
                notes: Vec::new(),
                fields: was
                    .fields
                    .iter()
                    .map(|f| gone_or_new(f, Change::Removed))
                    .collect(),
                was: Some(was.clone()),
                now: None,
            }),
            Some(now) => {
                if let Some(changed) = table_change(was, now) {
                    tables.push(changed);
                }
            }
        }
    }
    for now in &other.tables {
        if !base.tables.iter().any(|t| t.name == now.name) {
            tables.push(TableChange {
                name: now.name.clone(),
                change: Change::Added,
                notes: Vec::new(),
                fields: now
                    .fields
                    .iter()
                    .map(|f| gone_or_new(f, Change::Added))
                    .collect(),
                was: None,
                now: Some(now.clone()),
            });
        }
    }
    Comparison {
        tables,
        links: links(&base.links, &other.links),
        views: views(base, other),
    }
}

/// A table on both sides: `None` when nothing differs.
fn table_change(was: &Table, now: &Table) -> Option<TableChange> {
    let mut notes = Vec::new();
    if was.purpose != now.purpose {
        notes.push(worded("purpose", &was.purpose, &now.purpose));
    }
    if was.business != now.business {
        notes.push("business rules differ".to_string());
    }
    if was.technical != now.technical {
        notes.push("technical rules differ".to_string());
    }
    let mut fields = Vec::new();
    for f in &was.fields {
        match now.fields.iter().find(|x| x.name == f.name) {
            None => fields.push(gone_or_new(f, Change::Removed)),
            Some(g) => {
                if let Some(changed) = field_change(f, g) {
                    fields.push(changed);
                }
            }
        }
    }
    for g in &now.fields {
        if !was.fields.iter().any(|f| f.name == g.name) {
            fields.push(gone_or_new(g, Change::Added));
        }
    }
    if notes.is_empty() && fields.is_empty() {
        return None;
    }
    Some(TableChange {
        name: was.name.clone(),
        change: Change::Changed,
        notes,
        fields,
        was: Some(was.clone()),
        now: Some(now.clone()),
    })
}

/// A field one side only has.
fn gone_or_new(field: &Field, change: Change) -> FieldChange {
    let (was, now) = match change {
        Change::Removed => (Some(field.clone()), None),
        _ => (None, Some(field.clone())),
    };
    FieldChange {
        name: field.name.clone(),
        change,
        notes: Vec::new(),
        was,
        now,
    }
}

/// A field on both sides: `None` when nothing differs.
fn field_change(was: &Field, now: &Field) -> Option<FieldChange> {
    let mut notes = Vec::new();
    if was.data_type != now.data_type {
        notes.push(format!("type {} → {}", was.data_type, now.data_type));
    }
    if was.nullable != now.nullable {
        let words = |n: Option<bool>| match n {
            Some(true) => "may be empty",
            Some(false) => "required",
            None => "unsaid",
        };
        notes.push(format!("{} → {}", words(was.nullable), words(now.nullable)));
    }
    if was.default != now.default {
        notes.push(match (&was.default, &now.default) {
            (None, Some(d)) => format!("default {d} added"),
            (Some(_), None) => "default removed".to_string(),
            (Some(a), Some(b)) => format!("default {a} → {b}"),
            (None, None) => String::new(),
        });
    }
    for (name, before, after) in [
        ("primary key", was.primary, now.primary),
        ("unique", was.unique, now.unique),
        ("foreign key", was.foreign, now.foreign),
    ] {
        if before != after {
            notes.push(format!("{name} {}", if after { "gained" } else { "lost" }));
        }
    }
    if was.purpose != now.purpose {
        notes.push(worded("purpose", &was.purpose, &now.purpose));
    }
    if was.technical != now.technical {
        notes.push("technical rules differ".to_string());
    }
    if was.business != now.business {
        notes.push("business rules differ".to_string());
    }
    notes.retain(|n| !n.is_empty());
    if notes.is_empty() {
        return None;
    }
    Some(FieldChange {
        name: was.name.clone(),
        change: Change::Changed,
        notes,
        was: Some(was.clone()),
        now: Some(now.clone()),
    })
}

/// `purpose written`, `purpose dropped`, `purpose changed`.
fn worded(what: &str, before: &str, after: &str) -> String {
    match (before.trim().is_empty(), after.trim().is_empty()) {
        (true, false) => format!("{what} written"),
        (false, true) => format!("{what} dropped"),
        _ => format!("{what} changed"),
    }
}

fn same_ends(a: &Link, b: &Link) -> bool {
    a.from_table == b.from_table
        && a.from_field == b.from_field
        && a.to_table == b.to_table
        && a.to_field == b.to_field
}

fn links(base: &[Link], other: &[Link]) -> Vec<LinkChange> {
    let mut out = Vec::new();
    for was in base {
        match other.iter().find(|l| same_ends(l, was)) {
            None => out.push(LinkChange {
                change: Change::Removed,
                notes: Vec::new(),
                link: was.clone(),
            }),
            Some(now) => {
                let mut notes = Vec::new();
                if was.kind != now.kind {
                    notes.push("kind differs".to_string());
                }
                if was.on_delete != now.on_delete {
                    notes.push(format!(
                        "on delete {} → {}",
                        was.on_delete.as_deref().unwrap_or("unsaid"),
                        now.on_delete.as_deref().unwrap_or("unsaid")
                    ));
                }
                if was.on_update != now.on_update {
                    notes.push(format!(
                        "on update {} → {}",
                        was.on_update.as_deref().unwrap_or("unsaid"),
                        now.on_update.as_deref().unwrap_or("unsaid")
                    ));
                }
                if !notes.is_empty() {
                    out.push(LinkChange {
                        change: Change::Changed,
                        notes,
                        link: now.clone(),
                    });
                }
            }
        }
    }
    for now in other {
        if !base.iter().any(|l| same_ends(l, now)) {
            out.push(LinkChange {
                change: Change::Added,
                notes: Vec::new(),
                link: now.clone(),
            });
        }
    }
    out
}

fn views(base: &DataModel, other: &DataModel) -> Vec<ViewChange> {
    let mut out: Vec<ViewChange> = base
        .views
        .iter()
        .filter(|v| !other.views.iter().any(|w| w.name == v.name))
        .map(|v| ViewChange {
            name: v.name.clone(),
            change: Change::Removed,
        })
        .collect();
    out.extend(
        other
            .views
            .iter()
            .filter(|v| !base.views.iter().any(|w| w.name == v.name))
            .map(|v| ViewChange {
                name: v.name.clone(),
                change: Change::Added,
            }),
    );
    out
}

#[cfg(test)]
mod tests {
    use crate::model::build;

    use super::*;

    fn model(sql: &str) -> DataModel {
        build("t", &Ok(Some(sql.to_string())), &Ok(None))
    }

    const BASE: &str = r"
CREATE TABLE campagne (id uuid PRIMARY KEY, nom text NOT NULL);
CREATE TABLE pj (id uuid PRIMARY KEY, campagne_id uuid REFERENCES campagne(id), niveau integer, classe text);
CREATE TABLE old_thing (id uuid);
CREATE VIEW pj_actif AS SELECT p.id FROM pj p;
";

    const OTHER: &str = r"
CREATE TABLE campagne (id uuid PRIMARY KEY, nom text NOT NULL, archivee_le timestamptz);
CREATE TABLE pj (id uuid PRIMARY KEY, campagne_id uuid REFERENCES campagne(id) ON DELETE CASCADE, niveau text NOT NULL DEFAULT '1');
CREATE TABLE sort (id uuid PRIMARY KEY, pj_id uuid REFERENCES pj(id));
";

    #[test]
    fn the_same_model_has_no_difference() {
        let m = model(BASE);
        assert!(compare(&m, &m).is_same());
    }

    #[test]
    fn tables_fields_links_and_views_are_told_apart() {
        let c = compare(&model(BASE), &model(OTHER));
        let table = |name: &str| c.tables.iter().find(|t| t.name == name).expect(name);
        assert_eq!(table("old_thing").change, Change::Removed);
        assert_eq!(table("old_thing").fields[0].change, Change::Removed);
        assert_eq!(table("sort").change, Change::Added);
        assert_eq!(table("sort").fields.len(), 2);

        let campagne = table("campagne");
        assert_eq!(campagne.change, Change::Changed);
        assert_eq!(campagne.fields.len(), 1);
        assert_eq!(campagne.fields[0].name, "archivee_le");
        assert_eq!(campagne.fields[0].change, Change::Added);

        let pj = table("pj");
        let field = |name: &str| pj.fields.iter().find(|f| f.name == name).expect(name);
        assert_eq!(field("classe").change, Change::Removed);
        let niveau = field("niveau");
        assert_eq!(niveau.change, Change::Changed);
        assert_eq!(
            niveau.notes,
            [
                "type integer → text",
                "may be empty → required",
                "default '1' added",
                "technical rules differ"
            ]
        );
        assert!(
            niveau
                .was
                .as_ref()
                .is_some_and(|f| f.data_type == "integer")
                && niveau.now.as_ref().is_some_and(|f| f.data_type == "text")
        );

        let link = |from: &str| {
            c.links
                .iter()
                .find(|l| l.link.from_table == from)
                .expect(from)
        };
        assert_eq!(link("sort").change, Change::Added);
        assert_eq!(link("pj").change, Change::Changed);
        assert_eq!(link("pj").notes, ["on delete unsaid → cascade"]);
        assert_eq!(c.views.len(), 1);
        assert_eq!(
            (c.views[0].name.as_str(), c.views[0].change),
            ("pj_actif", Change::Removed)
        );
    }
}
