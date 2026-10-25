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
/// `campaign.saveEntity`), a slice of a unit an id names (all but
/// `infrastructure` and `tooling`) that names nothing it `produces`, even
/// one that evolves a unit already there, two slices producing the same id
/// (#208 and #209 both built `Battlemap`: one task is one Micro-UI). Empty
/// when every slice is whole; a slice for a human is not judged — a human
/// reads it.
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
    gaps.extend(schema_gaps(items));
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

/// Whether a slice is the Data layer's schema: an `infrastructure` slice
/// that writes the migrations.
#[must_use]
pub fn is_schema(item: &TaskItem) -> bool {
    item.declaration().unit == Unit::Infrastructure
        && item.technical.contains("crates/data/migrations/")
}

/// Where a plan lays its aggregates wrong: the migrations belong to one
/// schema slice that comes first, and an aggregate slice writes none —
/// chained on one another's migration, `ForgeDesMondes`' aggregates held
/// three of five lanes.
fn schema_gaps(items: &[TaskItem]) -> Vec<String> {
    let aggregates: Vec<(usize, &TaskItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| !item.needs_human && item.declaration().unit == Unit::Aggregate)
        .collect();
    let schema = items.iter().position(is_schema);
    let mut gaps = Vec::new();
    match schema {
        None if aggregates.len() >= 2 => gaps.push(
            "the milestone lays several aggregates and no `infrastructure` slice writes their \
             migrations: one schema slice, first, lays every migration from the design, and \
             the aggregates then build side by side"
                .to_string(),
        ),
        None => {}
        Some(schema) => {
            for (index, item) in
                items.iter().take(schema).enumerate().filter(|(_, item)| {
                    !item.needs_human && item.declaration().unit == Unit::Aggregate
                })
            {
                gaps.push(format!(
                    "slice {index} ({:?}) comes before the schema slice {schema}, whose tables it \
                     builds on",
                    item.title
                ));
            }
            for (index, item) in aggregates {
                let named = format!("slice {index} ({:?})", item.title);
                if item.technical.contains("crates/data/migrations/") {
                    gaps.push(format!(
                        "{named} names a migration: the schema slice {schema} writes them all, an \
                         aggregate slice installs its `aggregate.json` and invariants on tables \
                         already there"
                    ));
                }
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
            let Some((namespace, _)) = id.split_once('.') else {
                continue;
            };
            if namespace != declaration.system {
                gaps.push(format!(
                    "{named} declares `system: {}` for `{id}`: the system of a {} is its id's \
                     namespace, `{namespace}` (a DataCapability's `owner` is `dataguard`, not its system)",
                    declaration.system,
                    declaration.unit.key()
                ));
            }
            // The folder the sketch names follows the id too: #51–#53 wrote
            // crates/dataguard/data-capabilities/ and the sessions built it
            // there, under a folder the DataGuard owns nothing in.
            if declaration.unit == Unit::DataCapability {
                for folder in
                    data_capability_folders(&format!("{}\n{}", item.brief, item.technical))
                {
                    if folder != namespace {
                        gaps.push(format!(
                            "{named} places `{id}` under crates/{folder}/data-capabilities/: a \
                             DataCapability's folder is its id's namespace, \
                             crates/{namespace}/data-capabilities/<name>/ — the DataGuard owns it, \
                             it does not house it"
                        ));
                    }
                }
            }
        }
    }
    // Every unit a manifest id names says which: the checkout is narrowed to
    // the closure of what it produces, and one with none left the unit's
    // own folder out (ForgeDesMondes' #574, evolving
    // `campaign.exportCampaign`, and #507: halted on their own files).
    if declaration.unit.has_an_id() && declaration.produces.is_empty() {
        gaps.push(format!(
            "{named} is a {} that names nothing it `produces`: the id it builds — or the existing \
             unit it evolves — as its manifest names it (a Micro-UI or a screen as the mock-up \
             names it)",
            declaration.unit.key()
        ));
    }
    gaps
}

/// The systems whose `data-capabilities/` folder `text` names, each once —
/// `crates/dataguard/data-capabilities/save-entity/` gives `dataguard`. A
/// placeholder (`crates/<system>/…`) or a glob names none.
fn data_capability_folders(text: &str) -> Vec<&str> {
    let mut folders: Vec<&str> = Vec::new();
    for rest in text.split("crates/").skip(1) {
        let Some((system, after)) = rest.split_once('/') else {
            continue;
        };
        let named = !system.is_empty() && !system.starts_with('<') && !system.contains('*');
        // The folder's name whole, with or without a trailing slash.
        let is_the_folder = after.strip_prefix("data-capabilities").is_some_and(|tail| {
            tail.chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
        });
        if named && is_the_folder && !folders.contains(&system) {
            folders.push(system);
        }
    }
    folders
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
        assert_eq!(spec_gaps(&[right]), [] as [String; 0]);
        let capability = slice(&format!(
            r#"{{"title":"List","brief":"b","branch":"feat/list","unit":"capability","system":"media","produces":["session.listScenes"],{WHOLE}}}"#
        ));
        assert_eq!(spec_gaps(&[capability]).len(), 1);
    }

    #[test]
    fn a_data_capability_s_folder_is_its_id_s_namespace_never_the_dataguard_s() {
        // ForgeDesMondes milestone 18: #51–#53 sketched
        // crates/dataguard/data-capabilities/ and the sessions built there.
        let save = |technical: &str| {
            slice(&format!(
                r#"{{"title":"Save","brief":"b","branch":"feat/save","unit":"data-capability","system":"campaign","produces":["campaign.saveEntity"],"technical":"{technical}",{WHOLE}}}"#
            ))
        };
        let under_the_dataguard = spec_gaps(&[save(
            "crates/dataguard/data-capabilities/save-entity/ with its data-capability.json",
        )]);
        assert_eq!(under_the_dataguard.len(), 1, "{under_the_dataguard:?}");
        assert!(
            under_the_dataguard[0].contains("under crates/dataguard/data-capabilities/")
                && under_the_dataguard[0].contains("crates/campaign/data-capabilities/<name>/"),
            "{}",
            under_the_dataguard[0]
        );
        let elsewhere = spec_gaps(&[save("crates/media/data-capabilities/save-entity/")]);
        assert_eq!(elsewhere.len(), 1, "{elsewhere:?}");
        for fine in [
            "crates/campaign/data-capabilities/save-entity/ with its data-capability.json",
            "crates/<system>/data-capabilities/<name>/, crates/*/data-capabilities/*",
            "crates/campaign/aggregates/Entity/aggregate.json, crates/dataguard/registry/Cargo.toml",
            "",
        ] {
            assert!(spec_gaps(&[save(fine)]).is_empty(), "{fine}");
        }
        // The brief is read too: a session follows a path written there.
        let in_the_brief = slice(&format!(
            r#"{{"title":"Save","brief":"crates/dataguard/data-capabilities/save-entity/","branch":"feat/save","unit":"data-capability","system":"campaign","produces":["campaign.saveEntity"],{WHOLE}}}"#
        ));
        assert_eq!(spec_gaps(&[in_the_brief]).len(), 1);
        assert_eq!(
            data_capability_folders(
                "crates/dataguard/data-capabilities/a/ and crates/dataguard/data-capabilities/b/, crates/ai/data-capabilities/c/"
            ),
            ["dataguard", "ai"]
        );
    }

    #[test]
    fn a_folder_that_only_starts_with_data_capabilities_is_not_the_folder() {
        // `crates/media/data-capabilities-legacy/` and
        // `crates/media/data-capabilitiesx/` name another folder: the name is
        // read whole, a letter, a digit, `-` or `_` after it is more name.
        assert_eq!(
            data_capability_folders(
                "crates/media/data-capabilities-legacy/x/, crates/media/data-capabilitiesx/, \
                 crates/media/data-capabilities_2/, crates/media/data-capabilities9/"
            ),
            [] as [&str; 0]
        );
        // The folder itself, closed by a slash, a space, the end or punctuation.
        assert_eq!(
            data_capability_folders(
                "crates/media/data-capabilities/, crates/ai/data-capabilities and \
                 crates/credit/data-capabilities, crates/campaign/data-capabilities"
            ),
            ["media", "ai", "credit", "campaign"]
        );
        // So a sketch naming such a folder places nothing wrong.
        let save = slice(&format!(
            r#"{{"title":"Save","brief":"b","branch":"feat/save","unit":"data-capability","system":"campaign","produces":["campaign.saveEntity"],"technical":"crates/dataguard/data-capabilities-old/save-entity/",{WHOLE}}}"#
        ));
        assert_eq!(spec_gaps(&[save]), [] as [String; 0]);
    }

    fn aggregate(name: &str, technical: &str) -> String {
        // Its system's own folder, as the gate `aggregate-placed` has it.
        let system = name.to_lowercase();
        format!(
            r#"{{"title":"{name}","brief":"b","branch":"feat/{name}","unit":"aggregate","system":"{system}","aggregate":"{name}","produces":["{name}"],"technical":"{technical}",{WHOLE}}}"#
        )
    }

    const SCHEMA: &str = r#"{"title":"Schema","brief":"b","branch":"feat/schema","unit":"infrastructure","system":"-","technical":"crates/data/migrations/0005_media.sql, 0006_session.sql","goal":"g","acceptance":["given a, when b, then c"]}"#;

    #[test]
    fn several_aggregates_need_a_schema_slice_first_and_write_no_migration() {
        let media = aggregate("Media", "crates/media/aggregates/Media/");
        let session = aggregate("Session", "crates/session/aggregates/Session/");
        let chained = parse(&format!("[{media},{session}]")).expect("parse");
        let gaps = spec_gaps(&chained);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(gaps[0].contains("one schema slice, first"), "{}", gaps[0]);

        let one = parse(&format!("[{media}]")).expect("parse");
        assert!(
            spec_gaps(&one).is_empty(),
            "a lone aggregate may carry its migration"
        );

        let laid = parse(&format!("[{SCHEMA},{media},{session}]")).expect("parse");
        assert!(spec_gaps(&laid).is_empty(), "{:?}", spec_gaps(&laid));
        assert!(is_schema(&laid[0]) && !is_schema(&laid[1]));

        let tooling = r#"{"title":"Shell","brief":"b","branch":"feat/shell","unit":"tooling","system":"-","technical":"apps/shell/","goal":"g","acceptance":["given a, when b, then c"]}"#;
        let shell_first = parse(&format!("[{tooling},{SCHEMA},{media}]")).expect("parse");
        assert!(
            spec_gaps(&shell_first).is_empty(),
            "only an aggregate before the schema is early"
        );
        let early = parse(&format!("[{media},{SCHEMA},{session}]")).expect("parse");
        let gaps = spec_gaps(&early);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(gaps[0].contains("before the schema slice 1"), "{}", gaps[0]);

        let plain = parse(
            r#"[{"title":"Server","brief":"b","branch":"feat/server","unit":"infrastructure","system":"-","technical":"crates/server/","goal":"g","acceptance":["given a, when b, then c"]}]"#,
        )
        .expect("parse");
        assert!(
            !is_schema(&plain[0]),
            "an infrastructure slice without migrations"
        );
        let own = aggregate("Sound", "crates/data/migrations/0007_sound.sql");
        assert!(
            !is_schema(&parse(&format!("[{own}]")).expect("parse")[0]),
            "an aggregate with one"
        );
        let doubled = parse(&format!("[{SCHEMA},{media},{own}]")).expect("parse");
        let gaps = spec_gaps(&doubled);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(gaps[0].contains("names a migration"), "{}", gaps[0]);
    }

    #[test]
    fn every_unit_an_id_names_says_what_it_produces_even_one_it_evolves() {
        // ForgeDesMondes #574: a split slice evolving the existing
        // `campaign.exportCampaign`, no `produces` — its round halted on
        // its own folder, which the narrowed checkout had left out.
        let evolving = |produces: &str| {
            slice(&format!(
                r#"{{"title":"campaign.exportCampaign carries media, sounds and sessions","brief":"b","branch":"feat/export-media","unit":"data-capability","system":"campaign","effect":"update","touches":["Campaign.archive"],"aggregate":"Campaign"{produces},{WHOLE}}}"#
            ))
        };
        let gaps = spec_gaps(&[evolving("")]);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(
            gaps[0].contains("is a data-capability that names nothing it `produces`")
                && gaps[0].contains("the existing unit it evolves"),
            "{}",
            gaps[0]
        );
        assert_eq!(
            spec_gaps(&[evolving(r#","produces":["campaign.exportCampaign"]"#)]),
            [] as [String; 0]
        );
        // Each kind an id names; the plumbing and the tooling are named by
        // their branch.
        for unit in Unit::ALL {
            let bare = slice(&format!(
                r#"{{"title":"T","brief":"b","branch":"feat/t","unit":"{}","system":"campaign",{WHOLE}}}"#,
                unit.key()
            ));
            assert_eq!(
                spec_gaps(&[bare]).len(),
                usize::from(unit.has_an_id()),
                "{}",
                unit.key()
            );
        }
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
