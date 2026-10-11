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
//! A milestone is built in the architecture's layers ([`Layer`]): Concepts,
//! the data layer, the contract, the Capabilities, the UI. A task waits on
//! every task of the nearest earlier layer this split opened, plus whatever
//! it named in `depends_on`; inside a layer nothing is chained, so the loop
//! runs a layer's tasks in parallel. Two exceptions: the data layer is
//! chained on itself (and its first task on the last task that already
//! existed), because the architecture serializes mutations behind one
//! `DataGuard` and so does the board; and a Composition also waits on the
//! Micro-UIs before it, since it composes them.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

use std::collections::BTreeMap;

use crate::common::{branching, labels, sections};
use crate::split::data::plan;
use crate::split::data::state::SplitState;
use harness_architecture::vocabulary::{Declaration, Side};

/// A task's body and labels, from its slice.
///
/// Under their own headings, and `SCOPE` and `ARCHITECTURE` are the two
/// sections no refinement phase rewrites: the boundary this slice states and
/// its place in the architecture survive every later round instead of being
/// replaced by the five sections a refinement knows. A write-side slice is
/// the data layer, and labelled as such.
fn body_and_labels(
    item: &plan::TaskItem,
    declaration: &Declaration,
    kind: &'static str,
    side: Side,
) -> (String, Vec<&'static str>) {
    let body = format!(
        "## {}\n\nbranch: {}\n\n{}\n\n## {}\n\n{}",
        sections::heading_of(sections::SCOPE),
        item.branch,
        item.brief,
        sections::heading_of(sections::ARCHITECTURE),
        declaration.render()
    );
    let labels = if side == Side::Write {
        vec![kind, side.label(), labels::DATA_LAYER]
    } else {
        vec![kind, side.label()]
    };
    (body, labels)
}

/// What a task waits on: the task of this plan that produces each id it
/// needs. An id nobody produces is already there — nothing to wait on. Ids
/// compare case-insensitively: a manifest's `campagne.ajouterPj` and a
/// slice's `campagne.ajouterpj` are one.
fn graph_blockers(needs: &[String], producers: &BTreeMap<String, u64>) -> Vec<u64> {
    let mut blockers: Vec<u64> = needs
        .iter()
        .filter_map(|id| producers.get(&id.to_lowercase()).copied())
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
                producers.entry(id.to_lowercase()).or_insert(number);
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
                    vec![labels::AGENT.to_string(), labels::READ_SIDE.to_string()]
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
        // The data layer's label still marks what a human merges.
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
