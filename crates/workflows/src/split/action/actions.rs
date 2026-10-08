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

use crate::dev_loop::checks::architecture::Tree;
use crate::split::data::plan;
use crate::split::data::state::SplitState;

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
}

#[async_trait(?Send)]
impl Action<SplitState> for ReadInventory {
    async fn run(&self, ctx: &mut Context<SplitState>) -> Outcome<Verdict> {
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        ctx.state.inventory = inventory_block(&tree);
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
#[must_use]
pub fn inventory_block(tree: &Tree) -> String {
    let empty = tree.systems.is_empty()
        && tree.concepts.is_empty()
        && tree.capabilities.is_empty()
        && tree.aggregates.is_empty()
        && tree.data_capabilities.is_empty()
        && tree.micro_uis.is_empty();
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
    format!(
        "What the repository already holds of the architecture — name these exactly when a \
         slice builds on them, and invent no id that is not here:\n\
         - systems with Rust code: {}\n\
         - Concepts: {}\n\
         - Capabilities: {}\n\
         - aggregates: {}\n\
         - DataCapabilities: {}\n\
         - Micro-UIs: {}\n\
         - queries/registry.json: {}",
        listed(&tree.systems),
        listed(&concepts),
        listed(&capability_lines(tree)),
        listed(&aggregate_lines(tree)),
        listed(&data_capability_lines(tree)),
        listed(&micro_uis),
        if tree.has_query_registry {
            "present"
        } else {
            "absent"
        }
    )
}

/// the judgment stays out of the action that sends the prompt.
pub struct AskForSlice {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: &'static str,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
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
            self.template,
            &[
                ("num", &milestone.number.to_string()),
                ("title", &milestone.title),
                ("body", &body),
                ("existing", &existing),
                ("inventory", &open.state.inventory),
            ],
        );
        let task = format!("#{}", milestone.number);

        let reply =
            ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        if plan::parse(&reply.text).is_ok() {
            return Ok(Verdict::Continue);
        }
        let error = plan::parse(&reply.text).expect_err("just failed above");
        let retry_prompt = format!(
            "{prompt}\n\nYour previous answer's JSON did not parse: {error}. Reply \
             again, with ONLY the JSON array this time — no prose before or after it."
        );
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
        let said = inventory_block(&Tree::default());
        assert!(said.contains("no unit of the architecture yet"));
    }

    #[test]
    fn the_inventory_names_what_exists_exactly() {
        use crate::dev_loop::checks::architecture::{CapabilityCrate, Manifest};
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
        let said = inventory_block(&tree);
        for expected in [
            "campagne, dataguard",
            "NiveauDuGroupe@1",
            "campagne.niveau (system campagne, implements NiveauDuGroupe@1)",
            "Campagne [fields: nom; invariants: nom-unique]",
            "campagne.creer (insert on Campagne)",
            "Micro-UIs: (none)",
        ] {
            assert!(said.contains(expected), "{expected} missing from:\n{said}");
        }
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
