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
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Event;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::common::{branching, labels, sections};
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
fn body_and_labels(
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
    if let Some(rules) = rules_section(&item.rules) {
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

/// The `## Rules` section of a task: the rules whose tests it makes pass,
/// and what it may not do with them. `None` when the slice names none.
fn rules_section(rules: &[String]) -> Option<String> {
    let ids: Vec<&str> = rules
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    if ids.is_empty() {
        return None;
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
        if !items.is_empty() {
            self.ensure_milestone_branch(ctx.state.milestone()).await?;
        }
        // Who produces what, among the tasks this split opened: what a later
        // task that needs the id waits on.
        let mut producers: BTreeMap<String, u64> = BTreeMap::new();
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
            let number = self.gh.create_issue(&item.title, &body, &labels).await?;
            self.gh
                .create_sub_issue_link(milestone_number, number)
                .await?;
            let mut blockers: Vec<u64> = Vec::new();
            for dependency in &item.depends_on {
                match created.get(*dependency) {
                    Some(blocker) => blockers.push(*blocker),
                    None => ctx.traces.warn(&format!(
                        "task #{number} depends on slice {dependency}, which is not \
                         before it in the plan — dependency ignored"
                    )),
                }
            }
            for blocker in graph_blockers(&declaration.needs, &producers) {
                if !blockers.contains(&blocker) {
                    blockers.push(blocker);
                }
            }
            for blocker in &blockers {
                self.gh.add_blocked_by(number, *blocker).await?;
            }
            for id in &declaration.produces {
                producers.entry(id.clone()).or_insert(number);
            }
            ctx.traces.say(&format!(
                "opened task #{number}: {} ({kind}, {}, {}, {})",
                item.title,
                side.label(),
                layer.name(),
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
    /// Creates the milestone's own branch, off `base_branch`, if it does
    /// not exist yet — the first task's PR needs somewhere to land, and
    /// nothing else in the harness creates this branch.
    async fn ensure_milestone_branch(
        &self,
        milestone: &harness_core::domain::Issue,
    ) -> Outcome<()> {
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
        assert_eq!(rules_section(&[" ".to_string()]), None);
        let bare =
            plan::parse(r#"[{"title": "t", "brief": "b", "branch": "feat/t", "unit": "tooling"}]"#)
                .expect("parse");
        let (body, _) =
            body_and_labels(&bare[0], &bare[0].declaration(), labels::AGENT, Side::Read);
        assert!(!body.contains("## Rules"), "{body}");
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
        assert!(graph_blockers(&["Nothing".to_string()], &producers).is_empty());
    }

    #[tokio::test]
    async fn each_task_is_created_linked_placed_and_the_milestone_is_marked() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"A","brief":"do A","branch":"feat/a","needs_human":false,"unit":"capability","system":"credit","concept":"Risk@3"},
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
                Wrote::CreatedIssue(
                    "A".to_string(),
                    "## Scope\n\nbranch: feat/a\n\ndo A\n\n## Architecture\n\n\
                     unit: capability\nsystem: credit\nconcept: Risk@3\nprofile: mid\nside: harness:read-side"
                        .to_string(),
                    vec![
                        labels::AGENT.to_string(),
                        labels::READ_SIDE.to_string(),
                        labels::READY.to_string()
                    ]
                ),
                Wrote::SubIssueLink(4, 1),
                Wrote::CreatedIssue(
                    "B".to_string(),
                    "## Scope\n\nbranch: feat/b\n\ndo B\n\n## Architecture\n\n\
                     unit: micro-ui\nsystem: credit\nprofile: mid\nside: harness:read-side"
                        .to_string(),
                    vec![labels::HUMAN.to_string(), labels::READ_SIDE.to_string()]
                ),
                Wrote::SubIssueLink(4, 2),
                Wrote::BlockedByLink(2, 1),
                Wrote::Unlabelled(4, labels::READY.to_string()),
                Wrote::Label(4, labels::TRIGGERED.to_string()),
            ]
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
        assert!(bodies[1].contains("unit: aggregate\nsystem: campagne\naggregate: PJ\nproduces: PJ\nprofile: small\nside: harness:write-side"), "{}", bodies[1]);
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
