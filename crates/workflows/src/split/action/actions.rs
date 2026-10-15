//! What split reads for free, and what it asks of its paid session.
//!
//! [`AskForSlice`] receives its template rather than fetching it: texts live
//! with the table that sends them (`orchestration::stages`), and an action
//! that went to read them would reverse the composition's direction.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Issue, Outcome, Verdict, prompts};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::common::dossier::{Dossier, Filter};
use crate::split::data::plan;
use crate::split::data::state::SplitState;
use harness_architecture::tree::Tree;

/// Reads, for free, what the paid step must not repeat: the task slices
/// already open under this milestone.
pub struct ReadExistingTasks {
    /// What lists the milestone's existing sub-issues.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<SplitState> for ReadExistingTasks {
    async fn run(&self, ctx: &mut Context<SplitState>) -> Outcome<Verdict> {
        let number = ctx.state.milestone().number;
        let existing = still_open(self.gh.sub_issues(number).await?);
        if !existing.is_empty() {
            ctx.traces.say(&format!(
                "{} task(s) already open under #{number} — the slice must \
                 not reopen them",
                existing.len()
            ));
        }
        ctx.state.existing = existing;
        Ok(Verdict::Continue)
    }
}

/// The open ones, by number: a task closed as superseded or done is not a
/// slice the plan must avoid — GitHub's sub-issue list carries both.
fn still_open(mut issues: Vec<Issue>) -> Vec<Issue> {
    issues.retain(Issue::is_open);
    issues.sort_by_key(|issue| issue.number);
    issues
}

/// What's already open under the milestone, rendered for the prompt.
fn existing_block(existing: &[Issue]) -> String {
    if existing.is_empty() {
        return "No task exists yet under this milestone.".to_string();
    }
    let lines: Vec<String> = existing
        .iter()
        .map(|issue| format!("  - #{} {}", issue.number, issue.title))
        .collect();
    format!(
        "Tasks already open under this milestone — do not reopen them, only \
         plan what is still missing:\n{}",
        lines.join("\n")
    )
}

/// Sends the prompt that asks for the slice plan, and tolerates one
/// malformed reply by asking again with the parse error attached.
///
/// Not [`harness_core::execution::Tolerance`] — that forgives a whole stage
/// and moves to the next one. This is a second attempt at the *same* stage,
/// because a model asked to emit nothing but JSON sometimes doesn't on the
/// first try. If the second attempt still doesn't parse, this action does
/// not fail: the stage's post-gate (`checks::gates::SliceParses`) does, so
/// Reads, for free, what the checkout already holds of the architecture.
///
/// The plan must name the systems, Concepts, aggregates and `DataCapabilities`
/// that exist — the loop's architecture check refuses, after `code`, a unit
/// of a system nobody has. Read before the paid stage, from the same tree
/// that check reads after it, so the two cannot disagree.
pub struct ReadInventory {
    /// What reads the manifests.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
    /// What tasks of each unit have cost, for the prompt; empty when
    /// nothing was measured.
    pub cost_note: String,
}

#[async_trait(?Send)]
impl Action<SplitState> for ReadInventory {
    async fn run(&self, ctx: &mut Context<SplitState>) -> Outcome<Verdict> {
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        ctx.state.inventory = inventory_block(&tree, &self.cost_note);
        ctx.traces.say(&format!(
            "architecture inventory: {} system(s), {} concept(s), {} capabilit(ies), {} aggregate(s), \
             {} data-capabilit(ies), {} micro-ui(s)",
            tree.systems.len(),
            tree.concepts.len(),
            tree.capabilities.len(),
            tree.aggregates.len(),
            tree.data_capabilities.len(),
            tree.micro_uis.len()
        ));
        Ok(Verdict::Continue)
    }
}

/// Reads, for free, the milestone's slice of the design dossier.
///
/// The micro-frontends, writes, aggregates and rules of its systems,
/// aggregates and rules: what the slices copy their ids, effects and
/// criteria from.
pub struct ReadDossier {
    /// What reads the dossier's files.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl Action<SplitState> for ReadDossier {
    async fn run(&self, ctx: &mut Context<SplitState>) -> Outcome<Verdict> {
        let dossier = Dossier::read(self.disk.as_ref(), &self.root);
        let filter = Filter::of_milestone(&ctx.state.milestone().body);
        if dossier.is_empty() {
            ctx.traces
                .say("no design dossier: the slices are cut from the milestone alone");
        } else {
            ctx.traces.say(&format!(
                "design dossier: systems {}, aggregates {}, rules {}",
                list_or_all(&filter.systems),
                list_or_all(&filter.aggregates),
                list_or_all(&filter.rules)
            ));
        }
        ctx.state.dossier = dossier.block(&filter);
        Ok(Verdict::Continue)
    }
}

/// A list, or `all` when it is empty.
fn list_or_all(items: &[String]) -> String {
    if items.is_empty() {
        "all".to_string()
    } else {
        items.join(", ")
    }
}

/// A manifest's string field, or `?` when it is missing or not JSON.
fn field(json: Option<&serde_json::Value>, key: &str) -> String {
    json.and_then(|j| j.get(key))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?")
        .to_string()
}

/// One line per Capability: id, system, the Concept it implements.
fn capability_lines(tree: &Tree) -> Vec<String> {
    tree.capabilities
        .iter()
        .map(|c| {
            let json = c.manifest.as_ref().and_then(|m| m.json.as_ref());
            let implements = json
                .and_then(|j| j.get("implements"))
                .and_then(serde_json::Value::as_str)
                .map(|i| format!(", implements {i}"))
                .unwrap_or_default();
            format!(
                "{} (system {}{implements})",
                field(json, "capability"),
                c.system
            )
        })
        .collect()
}

/// One line per aggregate: id, its fields, its invariants.
fn aggregate_lines(tree: &Tree) -> Vec<String> {
    tree.aggregates
        .iter()
        .map(|m| {
            let json = m.json.as_ref();
            let invariants: Vec<String> = json
                .and_then(|j| j.get("invariants"))
                .and_then(serde_json::Value::as_array)
                .map(|a| a.iter().map(|i| field(Some(i), "id")).collect())
                .unwrap_or_default();
            let fields: Vec<String> = json
                .and_then(|j| j.get("fields"))
                .and_then(serde_json::Value::as_object)
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            format!(
                "{} [fields: {}; invariants: {}]",
                field(json, "aggregate"),
                listed(&fields),
                listed(&invariants)
            )
        })
        .collect()
}

/// One line per `DataCapability`: id, effect, the aggregate it targets.
fn data_capability_lines(tree: &Tree) -> Vec<String> {
    tree.data_capabilities
        .iter()
        .map(|m| {
            let json = m.json.as_ref();
            let target = json
                .and_then(|j| j.get("target"))
                .and_then(|t| t.get("aggregate"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            format!(
                "{} ({} on {target})",
                field(json, "dataCapability"),
                field(json, "effect")
            )
        })
        .collect()
}

/// Items joined, or `(none)`.
fn listed(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

/// What exists, rendered for the prompt — or the fact that nothing does.
///
/// Three parts after what every slice shares: what the **write side**
/// builds on (the aggregates with their fields and invariants, the
/// `DataCapabilities` with their effect and target), what the **read side**
/// builds on (the Capabilities, the Concepts, the query registry), and what
/// the **UI** builds on (the design system and its kit, the micro-frontends
/// mocked or built, the pages and the frozen screens) — each with the rule
/// its slices keep, so a slice of one kind is told what it writes against.
#[must_use]
pub fn inventory_block(tree: &Tree, cost_note: &str) -> String {
    let inventory = inventory_of(tree);
    if cost_note.trim().is_empty() {
        return inventory;
    }
    format!(
        "{inventory}\n\nWhat tasks of each unit have cost so far, under this harness — a slice \
         is one session, and a unit whose third quartile runs high is one to cut finer:\n{cost_note}"
    )
}

fn inventory_of(tree: &Tree) -> String {
    // A product may hold a design system and a mock-up before its first crate: that counts.
    let empty = tree.systems.is_empty()
        && tree.concepts.is_empty()
        && tree.capabilities.is_empty()
        && tree.aggregates.is_empty()
        && tree.data_capabilities.is_empty()
        && tree.micro_uis.is_empty()
        && tree.design.is_none()
        && tree.kit.is_empty()
        && tree.mocked_components.is_empty()
        && tree.screens.is_empty();
    if empty {
        return "The repository holds no unit of the architecture yet: this milestone's data \
                layer is the first, and every id it declares is new."
            .to_string();
    }
    let micro_uis: Vec<String> = tree
        .micro_uis
        .iter()
        .map(|(system, m)| format!("{} (system {system})", field(m.json.as_ref(), "microUi")))
        .collect();
    let concepts: Vec<String> = tree
        .concepts
        .iter()
        .map(|(name, version)| format!("{name}@{version}"))
        .collect();
    let ui = ui_lines(tree);
    format!(
        "What the repository already holds of the architecture — name these exactly when a \
         slice builds on them, and invent no id that is not here:\n\
         - systems with Rust code: {}\n\
         - Concepts: {}\n\
         - queries/registry.json: {}\n\
         \n\
         The write side (an `aggregate` or a `data-capability` slice builds on these; a \
         DataCapability is a folder named after itself, a dependency of its server, and \
         reaches no other slice):\n\
         - aggregates: {}\n\
         - DataCapabilities: {}\n\
         \n\
         The read side (a `capability` or `persisted-query` slice builds on these, reads \
         through persisted queries only, and reaches no other slice — plumbing of its own \
         system alone):\n\
         - Capabilities: {}\n\
         \n\
         The UI (a `micro-ui` or `composition` slice builds on these: it writes the kit's \
         classes with its own data, names its parts `data-writes`/`data-reads`, and ships \
         its micro-frontend — generated or mocked — before its code):\n\
         - design system: {}\n\
         - kit components generated: {}\n\
         - micro-frontends in the mock-up: {}\n\
         - Micro-UIs with code: {}\n\
         - pages mocked: {}\n\
         - frozen screens: {}",
        listed(&tree.systems),
        listed(&concepts),
        if tree.has_query_registry {
            "present"
        } else {
            "absent"
        },
        listed(&aggregate_lines(tree)),
        listed(&data_capability_lines(tree)),
        listed(&capability_lines(tree)),
        ui.design,
        listed(&ui.kit),
        listed(&ui.mocked),
        listed(&micro_uis),
        listed(&tree.mocked),
        listed(&tree.screens),
    )
}

/// What the UI builds on, rendered: the design system's state, the kit's
/// generated components, the micro-frontends of the mock-up and how each
/// was made.
struct UiLines {
    design: String,
    kit: Vec<String>,
    mocked: Vec<String>,
}

fn ui_lines(tree: &Tree) -> UiLines {
    let design = tree
        .design
        .as_ref()
        .and_then(|m| m.json.as_ref())
        .map_or_else(
            || {
                "none yet: a UI slice styles through the kit's variables with their fallbacks"
                    .to_string()
            },
            |json| {
                let state = field(Some(json), "state");
                format!(
                    "{state} (mock/design/tokens.css, one CSS variable per token){}",
                    if state == "validated" {
                        ""
                    } else {
                        " — not validated: no kit is generated from it yet"
                    }
                )
            },
        );
    let kit: Vec<String> = tree
        .kit
        .iter()
        .map(|(name, title)| {
            if title.is_empty() || title == name {
                name.clone()
            } else {
                format!("{name} ({title})")
            }
        })
        .collect();
    let mocked: Vec<String> = tree
        .mocked_components
        .iter()
        .map(|name| {
            if tree.bundles.iter().any(|b| &b.name == name) {
                format!("{name} (built from its Micro-UI's code)")
            } else {
                format!("{name} (hand-written or generated)")
            }
        })
        .collect();
    UiLines {
        design,
        kit,
        mocked,
    }
}

/// the judgment stays out of the action that sends the prompt.
pub struct AskForSlice {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What the slices are cut for: `strong`, `mid` or `small`.
    pub profile: String,
}

#[async_trait(?Send)]
impl SessionAction<SplitState> for AskForSlice {
    async fn run(&self, open: &mut Open<'_, SplitState>) -> Outcome<Verdict> {
        let milestone = open.state.milestone().clone();
        let existing = existing_block(&open.state.existing);
        let body = {
            let trimmed = milestone.body.trim();
            if trimmed.is_empty() {
                prompts::EMPTY_BODY.to_string()
            } else {
                trimmed.to_string()
            }
        };
        let prompt = splice(
            &self.template,
            &[
                ("num", &milestone.number.to_string()),
                ("title", &milestone.title),
                ("body", &body),
                ("existing", &existing),
                (
                    "profile",
                    &crate::split::orchestration::stages::profile_block(&self.profile),
                ),
                ("inventory", &open.state.inventory),
                ("dossier", &open.state.dossier),
            ],
        );
        let task = format!("#{}", milestone.number);

        let reply =
            ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        // One retry, for a reply that does not parse or whose slices are not
        // whole SPECs: it carries the error alone, the session has the rest.
        let retry_prompt = match plan::parse(&reply.text) {
            Ok(items) => {
                let gaps = plan::spec_gaps(&items);
                if gaps.is_empty() {
                    return Ok(Verdict::Continue);
                }
                format!(
                    "Your previous answer's slices are not whole SPECs: {}. Reply again with \
                     the whole JSON array, each agent slice carrying its `goal`, its \
                     `acceptance` criteria and every rule it claims cited `(R<id>)` in one \
                     of them — ONLY the JSON array, no prose.",
                    gaps.join("; ")
                )
            }
            Err(error) => crate::common::json_reply::retry_prompt(&error),
        };
        ask_and_record(
            open,
            &retry_prompt,
            &self.stage,
            1,
            &task,
            self.spending.as_ref(),
        )
        .await?;
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_checkout_says_the_data_layer_is_the_first() {
        let said = inventory_block(&Tree::default(), "");
        assert!(said.contains("no unit of the architecture yet"));
    }

    #[test]
    fn the_inventory_names_what_exists_exactly() {
        use harness_architecture::tree::{CapabilityCrate, Manifest};
        let manifest = |path: &str, json: &str| Manifest {
            path: path.to_string(),
            json: serde_json::from_str(json).ok(),
        };
        let tree = Tree {
            systems: vec!["campagne".to_string(), "dataguard".to_string()],
            concepts: vec![("NiveauDuGroupe".to_string(), 1)],
            capabilities: vec![CapabilityCrate {
                dir: "crates/campagne/capabilities/niveau".to_string(),
                system: "campagne".to_string(),
                manifest: Some(manifest(
                    "x",
                    r#"{"capability":"campagne.niveau","implements":"NiveauDuGroupe@1"}"#,
                )),
                cargo_toml: None,
            }],
            aggregates: vec![manifest(
                "a",
                r#"{"aggregate":"Campagne","fields":{"nom":{}},"invariants":[{"id":"nom-unique"}]}"#,
            )],
            data_capabilities: vec![manifest(
                "d",
                r#"{"dataCapability":"campagne.creer","effect":"insert","target":{"aggregate":"Campagne"}}"#,
            )],
            ..Tree::default()
        };
        let said = inventory_block(&tree, "");
        for expected in [
            "campagne, dataguard",
            "NiveauDuGroupe@1",
            "campagne.niveau (system campagne, implements NiveauDuGroupe@1)",
            "Campagne [fields: nom; invariants: nom-unique]",
            "campagne.creer (insert on Campagne)",
            "Micro-UIs with code: (none)",
            "design system: none yet",
            "kit components generated: (none)",
        ] {
            assert!(said.contains(expected), "{expected} missing from:\n{said}");
        }
    }

    #[test]
    fn the_ui_inventory_names_the_design_system_the_kit_and_the_mock_up() {
        use harness_architecture::tree::{Bundle, Manifest};
        let tree = Tree {
            design: Some(Manifest {
                path: "mock/design/design.json".to_string(),
                json: serde_json::from_str(r#"{"state":"validated"}"#).ok(),
            }),
            kit: vec![
                ("Button".to_string(), "Button".to_string()),
                ("HeroFrame".to_string(), "Hero frame".to_string()),
            ],
            mocked_components: vec!["Campagnes".to_string(), "Pjs".to_string()],
            bundles: vec![Bundle {
                name: "Pjs".to_string(),
                source: "apps/campagne/pjs".to_string(),
                built_from: None,
                digest: String::new(),
            }],
            mocked: vec!["pages/campagne.html".to_string()],
            screens: vec!["campagne".to_string()],
            ..Tree::default()
        };
        let said = inventory_block(&tree, "");
        for expected in [
            "design system: validated (mock/design/tokens.css",
            "kit components generated: Button, HeroFrame (Hero frame)",
            "micro-frontends in the mock-up: Campagnes (hand-written or generated), Pjs (built from its Micro-UI's code)",
            "pages mocked: pages/campagne.html",
            "frozen screens: campagne",
            "writes the kit's classes",
        ] {
            assert!(said.contains(expected), "{expected} missing from:\n{said}");
        }
        let mut candidate = tree;
        candidate.design = Some(Manifest {
            path: String::new(),
            json: serde_json::from_str(r#"{"state":"candidate"}"#).ok(),
        });
        assert!(inventory_block(&candidate, "").contains("not validated: no kit is generated"));
    }

    #[test]
    fn the_cost_note_follows_the_inventory_when_there_is_one() {
        let said = inventory_block(&Tree::default(), "- capability: median 3.00 $");
        assert!(said.starts_with("The repository holds no unit"), "{said}");
        assert!(
            said.contains("What tasks of each unit have cost so far"),
            "{said}"
        );
        assert!(said.ends_with("- capability: median 3.00 $"), "{said}");
        assert!(!inventory_block(&Tree::default(), "  ").contains("have cost"));
    }

    fn issue(number: u64, state: &str) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            ..Issue::default()
        }
    }

    #[test]
    fn closed_sub_issues_are_not_tasks_already_open() {
        let kept = still_open(vec![issue(9, "open"), issue(8, "closed"), issue(7, "open")]);
        let numbers: Vec<u64> = kept.iter().map(|issue| issue.number).collect();
        assert_eq!(numbers, [7, 9]);
    }

    #[test]
    fn a_milestone_whose_tasks_were_all_closed_is_sliced_from_scratch() {
        assert_eq!(
            existing_block(&still_open(vec![issue(1, "closed")])),
            "No task exists yet under this milestone."
        );
    }
}
