//! Split's own item type, parsed via the shared extraction.
//!
//! The session is asked to answer with a JSON array and nothing else, but a
//! model's reply is text, not structure — [`crate::common::json_reply`] is
//! the boundary where that text either becomes a plan or a reason it
//! couldn't; this module only names what one task slice looks like.

use serde::Deserialize;

pub use crate::common::json_reply::ParseError;
use harness_architecture::vocabulary::{Declaration, Effect, Unit};

/// One task slice the plan wants opened, in the order tasks must be taken.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TaskItem {
    /// A short, specific title — becomes the task issue's title.
    pub title: String,
    /// What this slice covers and does not cover — becomes its body,
    /// alongside the `branch:` line.
    pub brief: String,
    /// `<type>/<slug>` — written into the body as a parsable line.
    pub branch: String,
    /// True only for account creation, an interactive login, a payment
    /// decision, or anything else only a human in a browser can do.
    #[serde(default)]
    pub needs_human: bool,
    /// What the slice builds in the agent-native architecture, as
    /// [`Unit::key`] spells it. Required: a slice that is not one unit of
    /// the architecture is not a slice the plant can place.
    pub unit: String,
    /// The bounded context it belongs to (`credit`, `orders`).
    #[serde(default)]
    pub system: String,
    /// The versioned Concept it implements or defines (`Risk@3`), if any.
    #[serde(default)]
    pub concept: Option<String>,
    /// For a `data-capability`: `insert`, `update`, `delete` or `upsert`.
    #[serde(default)]
    pub effect: Option<String>,
    /// For a `data-capability`: the existing fields it touches.
    #[serde(default)]
    pub touches: Vec<String>,
    /// For the data layer: the aggregate it writes or changes the structure
    /// of (`PJ`), as its manifest names it. What serializes two such slices.
    #[serde(default)]
    pub aggregate: Option<String>,
    /// The ids this slice delivers, as the manifests will name them.
    #[serde(default)]
    pub produces: Vec<String>,
    /// The ids this slice builds on: `split` chains it on their producers.
    #[serde(default)]
    pub needs: Vec<String>,
    /// The 0-based indexes, in the same array, of the slices this one builds
    /// on, beyond the layer before its own (that wait is added by `publish`).
    /// Empty: none — a slice then runs in parallel with the rest of its
    /// layer.
    #[serde(default)]
    pub depends_on: Vec<usize>,
    /// The rules (`R3`) whose tests this slice makes pass, as `rules.json`
    /// names them — their tests already written, never changed by the task.
    #[serde(default)]
    pub rules: Vec<String>,
    /// What the user gets, in a few sentences: the task's `## Business Goal`.
    #[serde(default)]
    pub goal: String,
    /// The checks that say it is done, given/when/then, each rule it
    /// completes cited `(R3)`: its `## Acceptance Criteria`.
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// The invariants and rule criteria it must hold: its `## Business Rules`.
    #[serde(default)]
    pub business_rules: Vec<String>,
    /// The paths, manifests and contracts it opens — a sketch: its
    /// `## Technical`.
    #[serde(default)]
    pub technical: String,
}

impl TaskItem {
    /// The slice's place in the architecture.
    ///
    /// A unit the architecture does not name is read as `infrastructure`:
    /// the write side, where an agent review runs before anything merges — the
    /// safe reading of a slice the session could not place, and one that
    /// shows in the issue's own section rather than being silently fixed.
    #[must_use]
    pub fn declaration(&self) -> Declaration {
        Declaration {
            unit: Unit::parse(&self.unit).unwrap_or(Unit::Infrastructure),
            system: self.system.trim().to_string(),
            concept: self
                .concept
                .as_deref()
                .map(str::trim)
                .filter(|concept| !concept.is_empty() && *concept != "-")
                .map(ToString::to_string),
            effect: self.effect.as_deref().and_then(Effect::parse),
            touches: self
                .touches
                .iter()
                .map(|field| field.trim().to_string())
                .filter(|field| !field.is_empty() && field != "-")
                .collect(),
            aggregate: self
                .aggregate
                .as_deref()
                .map(str::trim)
                .filter(|aggregate| !aggregate.is_empty() && *aggregate != "-")
                .map(ToString::to_string),
            produces: ids(&self.produces),
            needs: ids(&self.needs),
            profile: None,
            host: None,
        }
    }
}

/// Ids trimmed, the empty and the `-` dropped.
fn ids(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty() && id != "-")
        .collect()
}

/// Extracts the JSON array of task slices from a reply's text.
///
/// # Errors
/// A [`ParseError`] if no array-shaped substring can be found, or if what
/// was found does not deserialize into `Vec<TaskItem>`.
pub fn parse(text: &str) -> Result<Vec<TaskItem>, ParseError> {
    crate::common::json_reply::parse_array(text)
}

/// What keeps a plan's agent slices from being whole SPECs, in words.
///
/// A slice with no goal, one with no acceptance criterion, a rule it claims
/// that no criterion of it cites; and what would misplace it in the
/// architecture: a Capability or `DataCapability` whose `system` is not its
/// id's namespace (`ForgeDesMondes`' #51 declared `dataguard`, the owner, for
/// `campaign.saveEntity`), a Micro-UI or screen that names nothing it
/// `produces`, two slices producing the same id (#208 and #209 both built
/// `Battlemap`: one task is one Micro-UI). Empty when every slice is whole; a
/// slice for a human is not judged — a human reads it.
#[must_use]
pub fn spec_gaps(items: &[TaskItem]) -> Vec<String> {
    let mut gaps = Vec::new();
    for (index, item) in items.iter().enumerate().filter(|(_, i)| !i.needs_human) {
        let named = format!("slice {index} ({:?})", item.title);
        if item.goal.trim().is_empty() {
            gaps.push(format!("{named} has no `goal`"));
        }
        let criteria: Vec<&str> = item
            .acceptance
            .iter()
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .collect();
        if criteria.is_empty() {
            gaps.push(format!("{named} has no `acceptance` criterion"));
        }
        for rule in item
            .rules
            .iter()
            .map(|r| r.trim())
            .filter(|r| !r.is_empty())
        {
            if !criteria.iter().any(|c| {
                crate::common::dossier::rule_ids(c)
                    .iter()
                    .any(|id| id == rule)
            }) {
                gaps.push(format!(
                    "{named} claims {rule}, which no acceptance criterion cites"
                ));
            }
        }
        gaps.extend(placement_gaps(&named, item));
    }
    let mut producers: Vec<(String, usize)> = Vec::new();
    for (index, item) in items.iter().enumerate().filter(|(_, i)| !i.needs_human) {
        for id in ids(&item.produces) {
            if let Some((_, first)) = producers.iter().find(|(seen, _)| *seen == id) {
                gaps.push(format!(
                    "slice {index} ({:?}) produces `{id}`, as slice {first} does — one slice per unit: \
                     a Micro-UI is one task, whatever components it realises",
                    item.title
                ));
            } else {
                producers.push((id, index));
            }
        }
    }
    gaps
}

/// Where one slice would land wrong in the architecture.
fn placement_gaps(named: &str, item: &TaskItem) -> Vec<String> {
    let declaration = item.declaration();
    let mut gaps = Vec::new();
    if matches!(declaration.unit, Unit::Capability | Unit::DataCapability) {
        for id in &declaration.produces {
            if let Some((namespace, _)) = id.split_once('.')
                && namespace != declaration.system
            {
                gaps.push(format!(
                    "{named} declares `system: {}` for `{id}`: the system of a {} is its id's \
                     namespace, `{namespace}` (a DataCapability's `owner` is `dataguard`, not its system)",
                    declaration.system,
                    declaration.unit.key()
                ));
            }
        }
    }
    if matches!(declaration.unit, Unit::MicroUi | Unit::Composition)
        && declaration.produces.is_empty()
    {
        gaps.push(format!(
            "{named} is a {} that names nothing it `produces`: the Micro-UI (or screen) it builds, \
             as the mock-up names it",
            declaration.unit.key()
        ));
    }
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;

    use harness_architecture::vocabulary::Side;

    #[test]
    fn a_bare_json_array_parses() {
        let items = parse(
            r#"[{"title":"A","brief":"do A","branch":"feat/a","needs_human":false,"unit":"capability","system":"credit"}]"#,
        )
        .expect("parse");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "A");
        assert_eq!(items[0].branch, "feat/a");
        assert!(!items[0].needs_human);
        assert_eq!(items[0].declaration().unit, Unit::Capability);
        assert_eq!(items[0].declaration().system, "credit");
    }

    #[test]
    fn needs_human_and_the_architecture_details_default_when_absent() {
        let items = parse(r#"[{"title":"A","brief":"do A","branch":"feat/a","unit":"micro-ui"}]"#)
            .expect("parse");
        assert!(!items[0].needs_human);
        assert_eq!(items[0].depends_on, [] as [usize; 0]);
        let declaration = items[0].declaration();
        assert_eq!(declaration.unit, Unit::MicroUi);
        assert_eq!(declaration.concept, None);
        assert_eq!(declaration.side(), Side::Read);
    }

    #[test]
    fn a_slice_without_a_unit_does_not_parse() {
        assert!(parse(r#"[{"title":"A","brief":"do A","branch":"feat/a"}]"#).is_err());
    }

    #[test]
    fn a_unit_the_architecture_does_not_name_is_read_as_infrastructure() {
        let items = parse(r#"[{"title":"A","brief":"do A","branch":"feat/a","unit":"widget"}]"#)
            .expect("parse");
        let declaration = items[0].declaration();
        assert_eq!(declaration.unit, Unit::Infrastructure);
        assert_eq!(
            declaration.side(),
            Side::Write,
            "a human looks at what nobody could place"
        );
    }

    #[test]
    fn a_data_capability_carries_its_effect_and_touches() {
        let items = parse(
            r#"[{"title":"A","brief":"a","branch":"feat/a","unit":"data-capability","system":"credit",
                "effect":"update","touches":["Account.creditLimit"," "],"aggregate":" Account ","depends_on":[0]}]"#,
        )
        .expect("parse");
        let declaration = items[0].declaration();
        assert_eq!(declaration.effect, Some(Effect::Update));
        assert_eq!(declaration.touches, ["Account.creditLimit"]);
        assert_eq!(declaration.aggregate.as_deref(), Some("Account"));
        assert_eq!(declaration.held_aggregate(), Some("Account"));
        assert_eq!(declaration.side(), Side::Write);
        assert_eq!(items[0].depends_on, [0]);
    }

    #[test]
    fn an_empty_array_is_valid_and_yields_nothing_to_create() {
        assert_eq!(parse("[]").expect("parse"), Vec::new());
    }

    #[test]
    fn multiple_slices_keep_their_order() {
        let items = parse(
            r#"[{"title":"A","brief":"a","branch":"feat/a","unit":"concept"},{"title":"B","brief":"b","branch":"feat/b","unit":"capability"}]"#,
        )
        .expect("parse");
        assert_eq!(items[0].title, "A");
        assert_eq!(items[1].title, "B");
    }

    fn slice(json: &str) -> TaskItem {
        parse(&format!("[{json}]")).expect("parse").remove(0)
    }

    const WHOLE: &str = r#""goal":"g","acceptance":["given a, when b, then c"]"#;

    #[test]
    fn a_data_capability_s_system_is_its_id_s_namespace() {
        let wrong = slice(&format!(
            r#"{{"title":"Save","brief":"b","branch":"feat/save","unit":"data-capability","system":"dataguard","produces":["campaign.saveEntity"],{WHOLE}}}"#
        ));
        let gaps = spec_gaps(&[wrong]);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(gaps[0].contains("`campaign`"), "{}", gaps[0]);
        let right = slice(&format!(
            r#"{{"title":"Save","brief":"b","branch":"feat/save","unit":"data-capability","system":"campaign","produces":["campaign.saveEntity"],{WHOLE}}}"#
        ));
        assert!(spec_gaps(&[right]).is_empty());
        let capability = slice(&format!(
            r#"{{"title":"List","brief":"b","branch":"feat/list","unit":"capability","system":"media","produces":["session.listScenes"],{WHOLE}}}"#
        ));
        assert_eq!(spec_gaps(&[capability]).len(), 1);
    }

    #[test]
    fn a_micro_ui_names_what_it_produces_and_only_one_slice_does() {
        let bare = slice(&format!(
            r#"{{"title":"UI","brief":"b","branch":"feat/ui","unit":"micro-ui","system":"table",{WHOLE}}}"#
        ));
        let gaps = spec_gaps(&[bare]);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(
            gaps[0].contains("names nothing it `produces`"),
            "{}",
            gaps[0]
        );
        let first = slice(&format!(
            r#"{{"title":"Map","brief":"b","branch":"feat/map","unit":"micro-ui","system":"table","produces":["Battlemap"],{WHOLE}}}"#
        ));
        let second = slice(&format!(
            r#"{{"title":"Sheet","brief":"b","branch":"feat/sheet","unit":"micro-ui","system":"table","produces":[" Battlemap "],{WHOLE}}}"#
        ));
        let other = slice(&format!(
            r#"{{"title":"Grid","brief":"b","branch":"feat/grid","unit":"micro-ui","system":"media","produces":["GridSettings"],{WHOLE}}}"#
        ));
        let gaps = spec_gaps(&[first, second, other]);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(
            gaps[0].starts_with("slice 1 (\"Sheet\") produces `Battlemap`, as slice 0 does"),
            "{}",
            gaps[0]
        );
    }
}
