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

use crate::common::architecture::{Layer, Side, Unit};
use crate::common::{branching, labels, sections};
use crate::split::data::plan;
use crate::split::data::state::SplitState;

/// A task's body and labels, from its slice.
///
/// Under their own headings, and `SCOPE` and `ARCHITECTURE` are the two
/// sections no refinement phase rewrites: the boundary this slice states and
/// its place in the architecture survive every later round instead of being
/// replaced by the five sections a refinement knows. A write-side slice is
/// the data layer, and labelled as such.
fn body_and_labels(
    item: &plan::TaskItem,
    kind: &'static str,
    side: Side,
) -> (String, Vec<&'static str>) {
    let body = format!(
        "## {}\n\nbranch: {}\n\n{}\n\n## {}\n\n{}",
        sections::heading_of(sections::SCOPE),
        item.branch,
        item.brief,
        sections::heading_of(sections::ARCHITECTURE),
        item.declaration().render()
    );
    let labels = if side == Side::Write {
        vec![kind, side.label(), labels::DATA_LAYER]
    } else {
        vec![kind, side.label()]
    };
    (body, labels)
}

/// What a task waits on by its place in the architecture alone.
///
/// The data layer chains on the last write before it; every other layer
/// waits on the whole nearest earlier layer this split opened, a Composition
/// on the Micro-UIs before it too. A Concept waits on nothing.
fn implied_blockers(
    unit: Unit,
    last_write: Option<u64>,
    by_layer: &BTreeMap<Layer, Vec<u64>>,
) -> Vec<u64> {
    let layer = unit.layer();
    match layer {
        Layer::Concept => Vec::new(),
        Layer::Data => last_write.into_iter().collect(),
        Layer::Contract | Layer::Capability | Layer::Ui => {
            let mut implied = by_layer
                .range(..layer)
                .next_back()
                .map(|(_, tasks)| tasks.clone())
                .unwrap_or_default();
            if unit == Unit::Composition
                && let Some(fragments) = by_layer.get(&Layer::Ui)
            {
                implied.extend(fragments);
            }
            implied
        }
    }
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
        // The data layer's chain starts after whatever already exists: a
        // mutation never runs beside a task that was open before this split.
        let mut last_write = ctx.state.existing.last().map(|issue| issue.number);
        // The tasks this split opened, by layer: what the next layer waits on.
        let mut by_layer: BTreeMap<Layer, Vec<u64>> = BTreeMap::new();
        let mut created: Vec<u64> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let kind = if item.needs_human {
                labels::HUMAN
            } else {
                labels::AGENT
            };
            let declaration = item.declaration();
            let side = declaration.side();
            let layer = declaration.unit.layer();
            let (body, labels) = body_and_labels(item, kind, side);
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
            for blocker in implied_blockers(declaration.unit, last_write, &by_layer) {
                if !blockers.contains(&blocker) {
                    blockers.push(blocker);
                }
            }
            for blocker in &blockers {
                self.gh.add_blocked_by(number, *blocker).await?;
            }
            if layer == Layer::Data {
                last_write = Some(number);
            }
            by_layer.entry(layer).or_default().push(number);
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
                     unit: capability\nsystem: credit\nconcept: Risk@3\nside: harness:read-side"
                        .to_string(),
                    vec![labels::AGENT.to_string(), labels::READ_SIDE.to_string()]
                ),
                Wrote::SubIssueLink(4, 1),
                Wrote::CreatedIssue(
                    "B".to_string(),
                    "## Scope\n\nbranch: feat/b\n\ndo B\n\n## Architecture\n\n\
                     unit: micro-ui\nsystem: credit\nside: harness:read-side"
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

    #[tokio::test]
    async fn read_side_slices_run_in_parallel_and_write_side_slices_are_chained() {
        let gh = Rc::new(gh_with_base());
        // A migration, then two Capabilities that depend on it, then a
        // DataCapability that mutates: the readers share nothing, the two
        // writers are chained, and nobody is chained for being next in line.
        let mut context = ctx(
            false,
            r#"[{"title":"M","brief":"schema","branch":"feat/m","unit":"migration","system":"credit"},
               {"title":"A","brief":"a","branch":"feat/a","unit":"capability","system":"credit","depends_on":[0]},
               {"title":"B","brief":"b","branch":"feat/b","unit":"capability","system":"credit","depends_on":[0]},
               {"title":"W","brief":"w","branch":"feat/w","unit":"data-capability","system":"credit","effect":"update","touches":["Account.creditLimit"]}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        let links: Vec<(u64, u64)> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::BlockedByLink(task, dependency) => Some((task, dependency)),
                _ => None,
            })
            .collect();
        assert_eq!(
            links,
            vec![(2, 1), (3, 1), (4, 1)],
            "A and B wait for M only; W chains onto M"
        );
        let sides: Vec<Vec<String>> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::CreatedIssue(_, _, labels) => Some(labels),
                _ => None,
            })
            .collect();
        assert!(
            sides[0].contains(&labels::WRITE_SIDE.to_string()),
            "a migration"
        );
        assert!(
            sides[0].contains(&labels::DATA_LAYER.to_string()),
            "the data layer is labelled as such"
        );
        assert!(!sides[1].contains(&labels::DATA_LAYER.to_string()));
        assert!(sides[1].contains(&labels::READ_SIDE.to_string()));
        assert!(
            sides[3].contains(&labels::WRITE_SIDE.to_string()),
            "an update"
        );
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
    async fn readers_wait_on_the_data_layer_and_a_concept_before_it_does_not() {
        let gh = Rc::new(gh_with_base());
        // A Concept, the data layer, then two readers that named nothing:
        // the readers still wait on the data layer, the Concept on nothing.
        let mut context = ctx(
            false,
            r#"[{"title":"C","brief":"c","branch":"feat/c","unit":"concept","system":"credit"},
               {"title":"D","brief":"d","branch":"feat/d","unit":"migration","system":"credit"},
               {"title":"A","brief":"a","branch":"feat/a","unit":"capability","system":"credit"},
               {"title":"T","brief":"t","branch":"chore/t","unit":"tooling","system":"credit"}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        let links: Vec<(u64, u64)> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::BlockedByLink(task, dependency) => Some((task, dependency)),
                _ => None,
            })
            .collect();
        assert_eq!(
            links,
            vec![(3, 2), (4, 2)],
            "A and T wait on D; C and D wait on nothing"
        );
    }

    #[tokio::test]
    async fn a_milestone_is_built_in_layers_each_waiting_on_the_one_before() {
        let gh = Rc::new(gh_with_base());
        // A pure insert (read side, still the data layer), the contract, two
        // Capabilities, a Micro-UI, the Composition. Nobody named anything:
        // the layers chain, the Capabilities run in parallel, the UI waits
        // on every action, the Composition on the fragment too.
        let mut context = ctx(
            false,
            r#"[{"title":"D","brief":"d","branch":"feat/d","unit":"data-capability","system":"shop","effect":"insert","touches":[]},
               {"title":"T","brief":"t","branch":"feat/t","unit":"tooling","system":"shop"},
               {"title":"A","brief":"a","branch":"feat/a","unit":"capability","system":"shop"},
               {"title":"B","brief":"b","branch":"feat/b","unit":"persisted-query","system":"shop"},
               {"title":"U","brief":"u","branch":"feat/u","unit":"micro-ui","system":"shop"},
               {"title":"S","brief":"s","branch":"test/s","unit":"composition","system":"shop"}]"#,
        );
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        let links: Vec<(u64, u64)> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::BlockedByLink(task, dependency) => Some((task, dependency)),
                _ => None,
            })
            .collect();
        assert_eq!(
            links,
            vec![
                (2, 1),
                (3, 2),
                (4, 2),
                (5, 3),
                (5, 4),
                (6, 3),
                (6, 4),
                (6, 5)
            ],
            "T waits on D; A and B on T; U on A and B; S on A, B and U"
        );
        let sides: Vec<Vec<String>> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::CreatedIssue(_, _, labels) => Some(labels),
                _ => None,
            })
            .collect();
        assert!(
            sides[0].contains(&labels::READ_SIDE.to_string()),
            "a pure insert keeps the read side: it merges alone"
        );
    }

    #[tokio::test]
    async fn the_first_new_write_side_task_chains_onto_the_last_existing_one() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"A","brief":"do A","branch":"feat/a","unit":"migration"}]"#,
        );
        context.state.existing = vec![Issue {
            number: 9,
            ..Issue::default()
        }];
        Write {
            gh: gh.clone(),
            slice_stage: "slice".to_string(),
            base_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert!(
            gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::BlockedByLink(_, 9)))
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
        }
        .run(&mut context)
        .await
        .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }
}
