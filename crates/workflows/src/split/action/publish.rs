//! The deterministic write: the milestone's own branch (created the first
//! time it's needed), the tasks opened from the parsed slice plan, and the
//! two labels that mark the milestone as split.
//!
//! The session decided *what* the slices are — this is the only place that
//! decides *how* they land on GitHub: created, linked to the milestone,
//! each body carrying a parsable `branch:` line and its `## Architecture`
//! section, labelled with its side, and linked by `blocked_by` to what it
//! declared it builds on.
//!
//! # What is chained, and what is not
//!
//! A task is chained on the task of this plan that `produces` an id it
//! `needs` (`graph_blockers`), plus whatever it named in `depends_on`; an
//! id no task of the plan produces is read as already in the repository.
//! Nothing else is chained: two tasks that share no id run in parallel, and
//! two that change the structure of one aggregate are kept apart at start
//! time by the loop (`tasks::startable_tasks`), not by a link. The layers
//! ([`Layer`]) order the plan's reading and the journal's wording, never a
//! link. Tasks that existed before this split are never chained.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Issue, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};

use harness_architecture::vocabulary::Unit;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::common::{branching, labels, native_milestone, sections};
use crate::split::data::plan;
use crate::split::data::state::SplitState;
use harness_architecture::vocabulary::{Declaration, Side};

/// A task's body and labels, from its slice.
///
/// Under their own headings, and `SCOPE`, `ARCHITECTURE` and `RULES` are the
/// sections no refinement phase rewrites: the boundary this slice states, its
/// place in the architecture and the rules it makes pass survive any later
/// round. Then the SPEC the slice carries — goal, acceptance criteria,
/// business rules, a technical sketch — which a refinement asked for by a
/// human may rework. A write-side slice is the data layer, and labelled as
/// such; an agent task is `harness:ready` at once, the harness opening the
/// tap a human closes.
pub(crate) fn body_and_labels(
    item: &plan::TaskItem,
    declaration: &Declaration,
    kind: &'static str,
    side: Side,
) -> (String, Vec<&'static str>) {
    let mut body = format!(
        "## {}\n\nbranch: {}\n\n{}\n\n## {}\n\n{}",
        sections::heading_of(sections::SCOPE),
        item.branch,
        item.brief,
        sections::heading_of(sections::ARCHITECTURE),
        declaration.render()
    );
    if let Some(rules) = rules_section(&item.rules, declaration.unit) {
        let _ = write!(
            body,
            "\n\n## {}\n\n{rules}",
            sections::heading_of(sections::RULES)
        );
    }
    // The SPEC the session builds from, written here: no refinement runs
    // after a split unless a human asks for one.
    let bullets = |items: &[String]| -> String {
        items
            .iter()
            .map(|i| i.trim())
            .filter(|i| !i.is_empty())
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    for (key, text) in [
        ("business-goal", item.goal.trim().to_string()),
        ("acceptance-criteria", bullets(&item.acceptance)),
        ("business-rules", bullets(&item.business_rules)),
        ("technical", item.technical.trim().to_string()),
    ] {
        if !text.is_empty() {
            let _ = write!(body, "\n\n## {}\n\n{text}", sections::heading_of(key));
        }
    }
    let mut labels = if side == Side::Write {
        vec![kind, side.label(), labels::DATA_LAYER]
    } else {
        vec![kind, side.label()]
    };
    // The tap is opened here: a human closes it by taking the label off.
    if kind == labels::AGENT {
        labels.push(labels::READY);
    }
    (body, labels)
}

/// `body` with the task's `id:` under its `unit:` line — the first id it
/// produces — for a unit an id names (`Unit::has_an_id`), as the contract
/// manager and the missing-task opener write it: every task the harness
/// opens names the unit it builds or evolves (`ForgeDesMondes`' #574).
fn with_unit_id(body: &str, declaration: &Declaration) -> String {
    let Some(id) = declaration
        .produces
        .first()
        .filter(|_| declaration.unit.has_an_id())
    else {
        return body.to_string();
    };
    let rendered = declaration.render();
    body.replacen(
        &rendered,
        &crate::dev_loop::data::missing::with_id(&rendered, id),
        1,
    )
}

/// The `## Rules` section of a task: the rules whose tests it makes pass,
/// and what it may not do with them. `None` when the slice names none.
///
/// A screen's says it is frozen now: the router no longer holds a screen
/// that names rules until every producer lands (`ForgeDesMondes`' #143), so
/// the session must not wait on a rule pending on a unit another milestone
/// builds — `rules-hold` proves it on every run once that unit is built.
fn rules_section(rules: &[String], unit: Unit) -> Option<String> {
    let ids: Vec<&str> = rules
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    if ids.is_empty() {
        return None;
    }
    if unit == Unit::Composition {
        return Some(format!(
            "{}\n\nTheir tests, under `tests/rules/` (contract P, `rules.json`), are written \
             already. Freeze the screen against the mock-up now: these rules' tests stay pending \
             while a unit they need is not built, and rules-hold proves them on every run once \
             it is. A pending rule is not this task's to wait for; one that runs red here is: \
             make it pass, or contest it with `CONTEST.md`. Never change them.",
            ids.join(", ")
        ));
    }
    Some(format!(
        "{}\n\nTheir tests, under `tests/rules/` (contract P, `rules.json`), are written \
         already: this task makes them pass — or leaves them pending on a unit another task \
         builds. Never change them; one you cannot meet is contested with `CONTEST.md`.",
        ids.join(", ")
    ))
}

/// What a task waits on: the task of this plan that produces each id it
/// needs. An id nobody produces is already there — nothing to wait on. Ids
/// compare case-insensitively: a manifest's `campagne.ajouterPj` and a
/// slice's `campagne.ajouterpj` are one.
/// `unblocked`, or `blocked by #a, #b`.
fn blocked_phrase(blockers: &[u64]) -> String {
    if blockers.is_empty() {
        "unblocked".to_string()
    } else {
        format!(
            "blocked by {}",
            blockers
                .iter()
                .map(|b| format!("#{b}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// The tasks a split has opened so far, as a slice's blockers are read off
/// them.
struct Chain<'a> {
    /// The tasks opened, in plan order: what `depends_on` indexes.
    created: &'a [u64],
    /// Who produces which id among them.
    producers: &'a BTreeMap<String, u64>,
    /// The aggregate tasks among them.
    aggregates: &'a BTreeSet<u64>,
    /// The schema task, when the plan has one.
    schema: Option<u64>,
}

/// What task `number` waits on: the slices it `depends_on`, then the
/// producers of what it `needs`, each once.
fn blockers_of(
    number: u64,
    item: &plan::TaskItem,
    declaration: &harness_architecture::vocabulary::Declaration,
    chain: &Chain<'_>,
    traces: &harness_core::traces::Logbook,
) -> Vec<u64> {
    let mut blockers: Vec<u64> = Vec::new();
    for dependency in &item.depends_on {
        match chain.created.get(*dependency) {
            Some(blocker) => blockers.push(*blocker),
            None => traces.warn(&format!(
                "task #{number} depends on slice {dependency}, which is not \
                 before it in the plan — dependency ignored"
            )),
        }
    }
    for blocker in chained(
        declaration.unit,
        &declaration.needs,
        chain.producers,
        chain.aggregates,
        chain.schema,
    ) {
        if !blockers.contains(&blocker) {
            blockers.push(blocker);
        }
    }
    blockers
}

/// What a slice waits on among the tasks before it: the producers of its
/// `needs`
/// — except that an aggregate waits on the schema slice, never on another
/// aggregate: the table it references is laid there, with every other
/// (`ForgeDesMondes`' `Session` waited on `Media`'s migration, `SoundPlayback`
/// on both; with the schema laid first they build side by side).
fn chained(
    unit: Unit,
    needs: &[String],
    producers: &BTreeMap<String, u64>,
    aggregates: &BTreeSet<u64>,
    schema: Option<u64>,
) -> Vec<u64> {
    let mut blockers = graph_blockers(needs, producers);
    if unit == Unit::Aggregate
        && let Some(schema) = schema
    {
        for blocker in &mut blockers {
            if aggregates.contains(blocker) {
                *blocker = schema;
            }
        }
        blockers.sort_unstable();
        blockers.dedup();
    }
    blockers
}

fn graph_blockers(needs: &[String], producers: &BTreeMap<String, u64>) -> Vec<u64> {
    // The id as written first — `Media` the aggregate is not `media` the
    // system's wiring, and the one written last would have hidden the other
    // (ForgeDesMondes' #125 waited on the wiring, not on its aggregate) —
    // then any spelling, as the milestones' chaining reads an aggregate.
    let mut blockers: Vec<u64> = needs
        .iter()
        .filter_map(|id| {
            producers.get(id).copied().or_else(|| {
                producers
                    .iter()
                    .find(|(known, _)| known.eq_ignore_ascii_case(id))
                    .map(|(_, number)| *number)
            })
        })
        .collect();
    blockers.sort_unstable();
    blockers.dedup();
    blockers
}

/// Opens one task per item of the parsed plan, in order, then marks the
/// milestone split.
pub struct Write {
    /// What creates the tasks, links them, and labels the milestone.
    pub gh: Rc<dyn GitHub>,
    /// The stage whose reply carries the slice plan.
    pub slice_stage: String,
    /// Where the milestone's own branch is created from, if it does not
    /// exist yet.
    pub base_branch: String,
    /// What the slices were cut for — written in every task, so the SPEC
    /// is calibrated the same way.
    pub profile: String,
}

#[async_trait(?Send)]
impl Action<SplitState> for Write {
    async fn run(&self, ctx: &mut Context<SplitState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get(&self.slice_stage) else {
            return Err(Halt::Failed(
                "no slice reply to publish from — the slice stage must run first".to_string(),
            ));
        };
        // The post-gate on the slice stage (`checks::gates::SliceParses`)
        // already demanded this parses; a failure here would be that gate
        // not having run, not a reason to improvise.
        let items = plan::parse(&reply.text).map_err(|e| {
            Halt::Failed(format!(
                "the slice plan still did not parse ({e}) — SliceParses \
                 should have stopped the round before this stage ever ran"
            ))
        })?;

        let milestone_number = ctx.state.milestone().number;
        let native = if items.is_empty() {
            None
        } else {
            self.ensure_milestone_branch(ctx.state.milestone()).await?;
            self.native(ctx.state.milestone(), &ctx.traces).await
        };
        // Who produces what, among the tasks this split opened: what a later
        // task that needs the id waits on.
        let mut producers: BTreeMap<String, u64> = BTreeMap::new();
        let mut aggregates: BTreeSet<u64> = BTreeSet::new();
        let mut schema: Option<u64> = None;
        let mut created: Vec<u64> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let kind = if item.needs_human {
                labels::HUMAN
            } else {
                labels::AGENT
            };
            let mut declaration = item.declaration();
            declaration.profile = Some(self.profile.clone());
            let side = declaration.side();
            let layer = declaration.unit.layer();
            let (body, labels) = body_and_labels(item, &declaration, kind, side);
            let body = with_unit_id(&body, &declaration);
            let number = self.gh.create_issue(&item.title, &body, &labels).await?;
            self.link(milestone_number, native, number, &ctx.traces)
                .await?;
            let blockers = blockers_of(
                number,
                item,
                &declaration,
                &Chain {
                    created: &created,
                    producers: &producers,
                    aggregates: &aggregates,
                    schema,
                },
                &ctx.traces,
            );
            for blocker in &blockers {
                self.gh.add_blocked_by(number, *blocker).await?;
            }
            for id in &declaration.produces {
                producers.entry(id.clone()).or_insert(number);
            }
            if declaration.unit == Unit::Aggregate {
                aggregates.insert(number);
            }
            if schema.is_none() && plan::is_schema(item) {
                schema = Some(number);
            }
            ctx.traces.say(&format!(
                "opened task #{number}: {} ({kind}, {}, {}, {})",
                item.title,
                side.label(),
                layer.name(),
                blocked_phrase(&blockers)
            ));
            created.push(number);
            debug_assert_eq!(created.len(), index + 1);
        }
        if items.is_empty() {
            ctx.traces.say("the slice named no task for this milestone");
        }

        // Posed last, in this order: the milestone must never read as
        // "still needing a split" once this ran, whether or not it created
        // anything new — a re-poll must not re-split it.
        self.gh
            .remove_label(milestone_number, labels::READY)
            .await?;
        self.gh
            .add_label(milestone_number, labels::TRIGGERED)
            .await?;
        let human_tasks: Vec<u64> = items
            .iter()
            .zip(&created)
            .filter(|(item, _)| item.needs_human)
            .map(|(_, number)| *number)
            .collect();
        ctx.traces.event(&Event::MilestoneSplit {
            milestone: milestone_number,
            tasks: created,
            human_tasks,
        });
        Ok(Verdict::Continue)
    }
}

impl Write {
    /// GitHub's native milestone mirroring `milestone`, made once for every
    /// task of the split: `None`, the refusal told, when GitHub will not —
    /// the tasks open without the field.
    async fn native(&self, milestone: &Issue, log: &Logbook) -> Option<u64> {
        match native_milestone::ensure(self.gh.as_ref(), milestone).await {
            Ok(number) => Some(number),
            Err(why) => {
                log.warn(&why);
                None
            }
        }
    }

    /// Links `task` under `milestone`, then puts it on the milestone's
    /// native milestone when there is one — a refusal of the latter told,
    /// never a failed split.
    async fn link(
        &self,
        milestone: u64,
        native: Option<u64>,
        task: u64,
        log: &Logbook,
    ) -> Outcome<()> {
        self.gh.create_sub_issue_link(milestone, task).await?;
        if let Some(native) = native
            && let Err(why) =
                native_milestone::assign(self.gh.as_ref(), native, milestone, task).await
        {
            log.warn(&why);
        }
        Ok(())
    }

    /// Creates the milestone's own branch, off `base_branch`, if it does
    /// not exist yet — the first task's PR needs somewhere to land, and
    /// nothing else in the harness creates this branch.
    async fn ensure_milestone_branch(&self, milestone: &Issue) -> Outcome<()> {
        let branch = branching::milestone_branch(milestone.number, &milestone.title);
        if self.gh.branch_sha(&branch).await?.is_some() {
            return Ok(());
        }
        let base_sha = self
            .gh
            .branch_sha(&self.base_branch)
            .await?
            .ok_or_else(|| {
                Halt::Failed(format!(
                    "base branch {} has no sha — cannot create {branch} from it",
                    self.base_branch
                ))
            })?;
        self.gh.create_branch(&branch, &base_sha).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;
    use std::collections::HashMap;

    #[test]
    fn a_task_carries_its_whole_spec_and_an_agent_one_is_ready_at_once() {
        let items = plan::parse(
            r#"[{"title": "List", "brief": "the list", "branch": "feat/list", "unit": "capability", "system": "campagne",
                 "goal": " The DM sees every PJ. ", "acceptance": ["given PJs, when opened, then all listed (R3)", " "],
                 "business_rules": ["names are unique"], "technical": "crates/campagne/capabilities/lister-pjs/"}]"#,
        )
        .expect("parse");
        let item = &items[0];
        let (body, labels) = body_and_labels(item, &item.declaration(), labels::AGENT, Side::Read);
        let found = sections::parse(&body);
        assert_eq!(
            found.get("business-goal").map(String::as_str),
            Some("The DM sees every PJ.")
        );
        assert_eq!(
            found.get("acceptance-criteria").map(String::as_str),
            Some("- given PJs, when opened, then all listed (R3)")
        );
        assert_eq!(
            found.get("business-rules").map(String::as_str),
            Some("- names are unique")
        );
        assert_eq!(
            found.get("technical").map(String::as_str),
            Some("crates/campagne/capabilities/lister-pjs/")
        );
        assert!(labels.contains(&labels::READY));
        let (_, human) = body_and_labels(item, &item.declaration(), labels::HUMAN, Side::Read);
        assert!(
            !human.contains(&labels::READY),
            "a human's task waits on the human"
        );
        let bare =
            plan::parse(r#"[{"title": "t", "brief": "b", "branch": "feat/t", "unit": "tooling"}]"#)
                .expect("parse");
        let (body, _) =
            body_and_labels(&bare[0], &bare[0].declaration(), labels::AGENT, Side::Read);
        assert!(
            !body.contains("## Business Goal") && !body.contains("## Technical"),
            "{body}"
        );
    }

    #[test]
    fn a_task_names_the_rules_it_makes_pass_in_a_section_no_refinement_rewrites() {
        let items = plan::parse(
            r#"[{"title": "List", "brief": "the list", "branch": "feat/list", "unit": "capability", "system": "campagne", "rules": ["R3", "", " R5 "]}]"#,
        )
        .expect("parse");
        let item = &items[0];
        let (body, _) = body_and_labels(item, &item.declaration(), labels::AGENT, Side::Read);
        let found = sections::parse(&body);
        let rules = found.get(sections::RULES).cloned().unwrap_or_default();
        assert!(rules.starts_with("R3, R5\n\n"), "{rules}");
        assert!(rules.contains("Never change them") && rules.contains("CONTEST.md"));
        assert!(found.contains_key(sections::ARCHITECTURE), "{body}");
        assert_eq!(rules_section(&[" ".to_string()], Unit::Capability), None);
        let bare =
            plan::parse(r#"[{"title": "t", "brief": "b", "branch": "feat/t", "unit": "tooling"}]"#)
                .expect("parse");
        let (body, _) =
            body_and_labels(&bare[0], &bare[0].declaration(), labels::AGENT, Side::Read);
        assert!(!body.contains("## Rules"), "{body}");
    }

    #[test]
    fn a_screen_s_rules_say_they_are_proven_where_their_units_are_built() {
        let items = plan::parse(
            r#"[{"title": "Soundbox", "brief": "freeze it", "branch": "feat/soundbox", "unit": "composition", "system": "soundbox", "rules": ["R111"]}]"#,
        )
        .expect("parse");
        let item = &items[0];
        let (body, _) = body_and_labels(item, &item.declaration(), labels::AGENT, Side::Read);
        let rules = sections::parse(&body)
            .get(sections::RULES)
            .cloned()
            .unwrap_or_default();
        assert!(rules.starts_with("R111\n\n"), "{rules}");
        for said in [
            "Freeze the screen against the mock-up now",
            "rules-hold proves them",
            "not this task's to wait for",
            "CONTEST.md",
            "Never change them",
        ] {
            assert!(rules.contains(said), "{said}: {rules}");
        }
        let unit = rules_section(&["R3".to_string()], Unit::MicroUi).unwrap_or_default();
        assert!(
            !unit.contains("Freeze the screen"),
            "only a screen is told: {unit}"
        );
    }

    fn ctx(dry_run: bool, text: &str) -> Context<SplitState> {
        let mut ctx = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            SplitState {
                milestone: Some(Issue {
                    number: 4,
                    title: "Territory tooling".to_string(),
                    ..Issue::default()
                }),
                ..SplitState::default()
            },
            Logbook::null(),
        );
        ctx.results.insert(
            "slice".to_string(),
            Reply {
                text: text.to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        ctx
    }

    /// The journal's lines, kept.
    #[derive(Default)]
    struct Capture(std::cell::RefCell<Vec<String>>);

    impl harness_core::traces::Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    /// A fake with the milestone branch absent and `main_agent` at a known
    /// sha — the common setup for a test that creates tasks.
    fn gh_with_base() -> FakeGitHub {
        FakeGitHub {
            branch_shas: HashMap::from([
                ("milestone/4-territory-tooling".to_string(), None),
                ("main_agent".to_string(), Some("basesha".to_string())),
            ]),
            ..FakeGitHub::default()
        }
    }

    #[test]
    fn a_need_is_chained_on_its_exact_producer_before_any_spelling() {
        let mut producers = BTreeMap::new();
        producers.insert("media".to_string(), 114);
        producers.insert("Media".to_string(), 117);
        producers.insert("PJ".to_string(), 3);
        assert_eq!(graph_blockers(&["Media".to_string()], &producers), [117]);
        assert_eq!(graph_blockers(&["media".to_string()], &producers), [114]);
        assert_eq!(
            graph_blockers(
                &["pj".to_string(), "Media".to_string(), "media".to_string()],
                &producers
            ),
            [3, 114, 117],
            "any spelling when no exact id; sorted, deduplicated"
        );
        assert!(
            graph_blockers(&["Nothing".to_string()], &producers).is_empty(),
            "graph_blockers(&['Nothing'.to_string()], &producers) should be empty"
        );
    }

    #[test]
    fn the_trace_says_what_a_task_is_blocked_by() {
        assert_eq!(blocked_phrase(&[]), "unblocked");
        assert_eq!(blocked_phrase(&[7, 12]), "blocked by #7, #12");
    }

    #[tokio::test]
    async fn the_split_chains_an_aggregate_on_the_schema_task_and_a_write_on_its_aggregate() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"Shell","brief":"b","branch":"feat/shell","unit":"tooling","system":"-","technical":"apps/shell/"},
               {"title":"Schema","brief":"b","branch":"feat/schema","unit":"infrastructure","system":"-","technical":"crates/data/migrations/0005_media.sql"},
               {"title":"Media","brief":"b","branch":"feat/media","unit":"aggregate","system":"media","aggregate":"Media","produces":["Media"],"needs":["Campaign"],"depends_on":[1]},
               {"title":"Session","brief":"b","branch":"feat/session","unit":"aggregate","system":"session","aggregate":"Session","produces":["Session"],"needs":["Media"]},
               {"title":"Save","brief":"b","branch":"feat/save","unit":"data-capability","system":"session","aggregate":"Session","produces":["session.save"],"needs":["Session","Media"]}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert_eq!(
            links(&gh),
            vec![(3, 2), (4, 2), (5, 3), (5, 4)],
            "Media and Session wait on the schema (#2, not the tooling before it); the write \
             waits on both aggregates"
        );

        // No schema slice: the first task is not taken for one, and the
        // aggregates chain on each other as before.
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"Shell","brief":"b","branch":"feat/shell","unit":"tooling","system":"-","technical":"apps/shell/"},
               {"title":"Media","brief":"b","branch":"feat/media","unit":"aggregate","system":"media","aggregate":"Media","produces":["Media"]},
               {"title":"Session","brief":"b","branch":"feat/session","unit":"aggregate","system":"session","aggregate":"Session","produces":["Session"],"needs":["Media"]}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert_eq!(links(&gh), vec![(3, 2)]);
    }

    #[test]
    fn an_aggregate_waits_on_the_schema_slice_not_on_another_aggregate() {
        let producers = BTreeMap::from([
            ("Media".to_string(), 117),
            ("Campaign".to_string(), 105),
            ("media".to_string(), 90),
        ]);
        let aggregates = BTreeSet::from([117, 105]);
        let needs = [
            "Media".to_string(),
            "Campaign".to_string(),
            "media".to_string(),
        ];
        assert_eq!(
            chained(Unit::Aggregate, &needs, &producers, &aggregates, Some(200)),
            [90, 200]
        );
        assert_eq!(
            chained(Unit::Aggregate, &needs, &producers, &aggregates, None),
            [90, 105, 117],
            "no schema slice: the aggregates chain as before"
        );
        assert_eq!(
            chained(
                Unit::DataCapability,
                &needs,
                &producers,
                &aggregates,
                Some(200)
            ),
            [90, 105, 117],
            "a write waits on its aggregate's own task"
        );
    }

    #[test]
    fn a_unit_an_id_names_carries_its_id_and_the_plumbing_does_not() {
        let body_of = |json: &str| {
            let item = plan::parse(&format!("[{json}]")).expect("parse").remove(0);
            let declaration = item.declaration();
            let (body, _) = body_and_labels(&item, &declaration, labels::AGENT, declaration.side());
            with_unit_id(&body, &declaration)
        };
        let evolving = body_of(
            r#"{"title":"Export","brief":"b","branch":"feat/export","unit":"data-capability","system":"campaign","produces":["campaign.exportCampaign","campaign.other"]}"#,
        );
        assert!(
            evolving
                .contains("unit: data-capability\nid: campaign.exportCampaign\nsystem: campaign"),
            "{evolving}"
        );
        let wiring = body_of(
            r#"{"title":"Wiring","brief":"b","branch":"chore/wiring-campaign","unit":"tooling","system":"campaign","produces":["campaign"]}"#,
        );
        assert!(!wiring.contains("id: "), "{wiring}");
        let bare = body_of(
            r#"{"title":"UI","brief":"b","branch":"feat/ui","unit":"micro-ui","system":"campaign"}"#,
        );
        assert!(!bare.contains("id: "), "{bare}");
    }

    #[tokio::test]
    async fn each_task_is_created_linked_placed_and_the_milestone_is_marked() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"A","brief":"do A","branch":"feat/a","needs_human":false,"unit":"capability","system":"credit","concept":"Risk@3","produces":["credit.calculateRisk"]},
               {"title":"B","brief":"do B","branch":"feat/b","needs_human":true,"unit":"micro-ui","system":"credit","depends_on":[0]}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        let writes = gh.writes();
        assert_eq!(
            writes,
            vec![
                Wrote::CreatedBranch(
                    "milestone/4-territory-tooling".to_string(),
                    "basesha".to_string()
                ),
                Wrote::NativeMilestone(4, "Territory tooling".to_string(), true),
                Wrote::CreatedIssue(
                    "A".to_string(),
                    "## Scope\n\nbranch: feat/a\n\ndo A\n\n## Architecture\n\n\
                     unit: capability\nid: credit.calculateRisk\nsystem: credit\nconcept: Risk@3\n\
                     produces: credit.calculateRisk\nprofile: mid\nside: harness:read-side"
                        .to_string(),
                    vec![
                        labels::AGENT.to_string(),
                        labels::READ_SIDE.to_string(),
                        labels::READY.to_string()
                    ]
                ),
                Wrote::SubIssueLink(4, 1),
                Wrote::IssueMilestone(1, 1),
                Wrote::CreatedIssue(
                    "B".to_string(),
                    "## Scope\n\nbranch: feat/b\n\ndo B\n\n## Architecture\n\n\
                     unit: micro-ui\nsystem: credit\nprofile: mid\nside: harness:read-side"
                        .to_string(),
                    vec![labels::HUMAN.to_string(), labels::READ_SIDE.to_string()]
                ),
                Wrote::SubIssueLink(4, 2),
                Wrote::IssueMilestone(2, 1),
                Wrote::BlockedByLink(2, 1),
                Wrote::Unlabelled(4, labels::READY.to_string()),
                Wrote::Label(4, labels::TRIGGERED.to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn a_native_milestone_github_refuses_is_told_once_and_the_tasks_open_all_the_same() {
        let gh = Rc::new(FakeGitHub {
            natives_broken: Some(Halt::Halted("HTTP 403".to_string())),
            ..gh_with_base()
        });
        let mut context = ctx(
            false,
            r#"[{"title":"A","brief":"do A","branch":"feat/a","needs_human":false,"unit":"capability","system":"credit","concept":"Risk@3"},
               {"title":"B","brief":"do B","branch":"feat/b","needs_human":false,"unit":"capability","system":"credit","concept":"Loan@3"}]"#,
        );
        let capture = Rc::new(Capture::default());
        context.traces = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published all the same");
        assert_eq!(links(&gh).len(), 0, "no blocker between A and B");
        let linked: Vec<Wrote> = gh
            .writes()
            .into_iter()
            .filter(|w| matches!(w, Wrote::SubIssueLink(..) | Wrote::IssueMilestone(..)))
            .collect();
        assert_eq!(
            linked,
            vec![Wrote::SubIssueLink(4, 1), Wrote::SubIssueLink(4, 2)]
        );
        let warned: Vec<String> = capture
            .0
            .borrow()
            .iter()
            .filter(|line| line.starts_with("warning:"))
            .cloned()
            .collect();
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert!(
            warned[0].contains("#4's native milestone could not be kept: HTTP 403"),
            "{warned:?}"
        );
    }

    #[tokio::test]
    async fn a_task_whose_native_milestone_github_refuses_is_still_linked() {
        let gh = FakeGitHub {
            natives_broken: Some(Halt::Halted("HTTP 403".to_string())),
            ..FakeGitHub::default()
        };
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        let write = Write {
            gh: Rc::new(gh),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        };
        write.link(4, Some(1), 7, &log).await.expect("linked");
        write.link(4, None, 8, &log).await.expect("linked");
        let said = capture.0.borrow().clone();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("#7 is a sub-issue of #4 but not on its native"),
            "{said:?}"
        );
    }

    /// The links `blocked_by` drew, as `(task, dependency)`.
    fn links(gh: &FakeGitHub) -> Vec<(u64, u64)> {
        gh.writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::BlockedByLink(task, dependency) => Some((task, dependency)),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_task_waits_on_what_it_builds_on_and_on_nothing_else() {
        let gh = Rc::new(gh_with_base());
        // Two aggregates (#1 PJ, #2 Campagne), their commands (#3 on PJ, #4
        // on Campagne), three readers (#5 lists PJ, #6 lists Campagne, #7
        // the party level reads both), two Micro-UIs (#8 reads #5 and
        // triggers #3; #9 reads #6), and the screen (#10) composing both.
        // A Concept (#0 → task #11 last, to show order does not matter)
        // that #7 implements — but listed first here, as #1 of the plan.
        let mut context = ctx(
            false,
            r#"[{"title":"Niveau","brief":"c","branch":"feat/c","unit":"concept","system":"campagne","produces":["NiveauDuGroupe@1"]},
               {"title":"PJ","brief":"a","branch":"feat/pj","unit":"aggregate","system":"campagne","aggregate":"PJ","produces":["PJ"]},
               {"title":"Campagne","brief":"a","branch":"feat/campagne","unit":"aggregate","system":"campagne","aggregate":"Campagne","produces":["Campagne"]},
               {"title":"ajouterPj","brief":"d","branch":"feat/ajouter","unit":"data-capability","system":"campagne","effect":"insert","touches":[],"aggregate":"PJ","produces":["campagne.ajouterPj"],"needs":["PJ"]},
               {"title":"creerCampagne","brief":"d","branch":"feat/creer","unit":"data-capability","system":"campagne","effect":"insert","touches":[],"aggregate":"Campagne","produces":["campagne.creerCampagne"],"needs":["Campagne"]},
               {"title":"listerPjs","brief":"r","branch":"feat/lister-pjs","unit":"capability","system":"campagne","produces":["campagne.listerPjs"],"needs":["PJ"]},
               {"title":"listerCampagnes","brief":"r","branch":"feat/lister-c","unit":"capability","system":"campagne","produces":["campagne.listerCampagnes"],"needs":["Campagne"]},
               {"title":"niveauDuGroupe","brief":"r","branch":"feat/niveau","unit":"capability","system":"campagne","concept":"NiveauDuGroupe@1","produces":["campagne.niveauDuGroupe"],"needs":["PJ","Campagne","NiveauDuGroupe@1"]},
               {"title":"Pjs","brief":"u","branch":"feat/ui-pjs","unit":"micro-ui","system":"campagne","produces":["Pjs"],"needs":["campagne.listerPjs","campagne.ajouterPj"]},
               {"title":"Campagnes","brief":"u","branch":"feat/ui-c","unit":"micro-ui","system":"campagne","produces":["Campagnes"],"needs":["campagne.listerCampagnes","campagne.creerCampagne","Account"]},
               {"title":"campagne","brief":"s","branch":"test/screen","unit":"composition","system":"campagne","produces":["campagne"],"needs":["Pjs","Campagnes"]}]"#,
        );
        context.state.existing = vec![Issue {
            number: 99,
            ..Issue::default()
        }];
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "small".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert_eq!(
            links(&gh),
            vec![
                (4, 2),
                (5, 3),
                (6, 2),
                (7, 3),
                (8, 1),
                (8, 2),
                (8, 3),
                (9, 4),
                (9, 6),
                (10, 5),
                (10, 7),
                (11, 9),
                (11, 10),
            ],
            "each task waits on the producers of what it needs: the two aggregates and the Concept \
             wait on nothing, the two commands and the three readers on their aggregate(s), each \
             Micro-UI on its own reads and actions, the screen on its two fragments — `Account`, \
             which nobody produces, is already there; the task that existed before (#99) \
             chains nothing"
        );
        // Written in every task: what it produces and needs, and the profile it was cut for.
        let bodies: Vec<String> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::CreatedIssue(_, body, _) => Some(body),
                _ => None,
            })
            .collect();
        assert!(
            bodies[8].contains(
                "produces: Pjs\nneeds: campagne.listerPjs, campagne.ajouterPj\nprofile: small"
            ),
            "{}",
            bodies[8]
        );
        assert!(bodies[1].contains("unit: aggregate\nid: PJ\nsystem: campagne\naggregate: PJ\nproduces: PJ\nprofile: small\nside: harness:write-side"), "{}", bodies[1]);
        // The data layer's label still marks what merges after its review.
        let labelled: Vec<Vec<String>> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::CreatedIssue(_, _, labels) => Some(labels),
                _ => None,
            })
            .collect();
        assert!(
            labelled[1].contains(&labels::DATA_LAYER.to_string()),
            "an aggregate"
        );
        assert!(
            !labelled[3].contains(&labels::DATA_LAYER.to_string()),
            "a pure insert reads"
        );
        assert!(labelled[3].contains(&labels::READ_SIDE.to_string()));
    }

    #[tokio::test]
    async fn an_id_needed_in_another_case_still_finds_its_producer() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"D","brief":"d","branch":"feat/d","unit":"data-capability","system":"c","effect":"insert","produces":["campagne.ajouterPj"]},
               {"title":"U","brief":"u","branch":"feat/u","unit":"micro-ui","system":"c","needs":["campagne.ajouterpj"]}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert_eq!(links(&gh), vec![(2, 1)]);
    }

    #[tokio::test]
    async fn a_dependency_on_a_later_slice_is_ignored_with_a_warning() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"A","brief":"a","branch":"feat/a","unit":"capability","depends_on":[7]}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert!(
            !gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::BlockedByLink(..)))
        );
    }

    #[tokio::test]
    async fn an_already_existing_milestone_branch_is_not_recreated() {
        let gh = Rc::new(FakeGitHub {
            branch_shas: HashMap::from([(
                "milestone/4-territory-tooling".to_string(),
                Some("alreadythere".to_string()),
            )]),
            ..FakeGitHub::default()
        });
        let mut context = ctx(
            false,
            r#"[{"title":"A","brief":"do A","branch":"feat/a","unit":"capability"}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert!(
            !gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::CreatedBranch(..)))
        );
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(true, "[]");
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("nothing to do");
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn an_empty_plan_still_marks_the_milestone_split() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, "[]");
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("nothing to create, still marked");
        assert_eq!(
            gh.writes(),
            vec![
                Wrote::Unlabelled(4, labels::READY.to_string()),
                Wrote::Label(4, labels::TRIGGERED.to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn a_read_side_task_is_not_chained_onto_what_already_exists() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"A","brief":"do A","branch":"feat/a","unit":"capability"}]"#,
        );
        context.state.existing = vec![Issue {
            number: 9,
            ..Issue::default()
        }];
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert!(
            !gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::BlockedByLink(..))),
            "a Capability shares nothing with its neighbours"
        );
    }

    #[tokio::test]
    async fn an_unparseable_reply_is_a_failure_not_an_improvisation() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, "not json");
        let err = Write {
            gh,
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
        }
        .run(&mut context)
        .await
        .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }
}
