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
//! A read-side slice is blocked only by the slices it named in
//! `depends_on`: two Capabilities of one milestone share nothing, so the
//! loop may run them in parallel. A write-side slice is also chained onto
//! the previous write-side slice (or the last task that already existed):
//! the architecture serializes mutations behind one `DataGuard`, and so does
//! the board.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

use crate::common::architecture::Side;
use crate::common::{branching, labels, sections};
use crate::split::data::plan;
use crate::split::data::state::SplitState;

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
        // The write chain starts after whatever already exists: a mutation
        // never runs beside a task that was open before this split.
        let mut last_write = ctx.state.existing.last().map(|issue| issue.number);
        let mut created: Vec<u64> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let kind = if item.needs_human {
                labels::HUMAN
            } else {
                labels::AGENT
            };
            let declaration = item.declaration();
            let side = declaration.side();
            // Under their own headings, and `SCOPE` and `ARCHITECTURE` are the
            // two sections no refinement phase rewrites: the boundary this
            // slice states and its place in the architecture survive every
            // later round instead of being replaced by the five sections a
            // refinement knows.
            let body = format!(
                "## {}\n\nbranch: {}\n\n{}\n\n## {}\n\n{}",
                sections::heading_of(sections::SCOPE),
                item.branch,
                item.brief,
                sections::heading_of(sections::ARCHITECTURE),
                declaration.render()
            );
            let number = self
                .gh
                .create_issue(&item.title, &body, &[kind, side.label()])
                .await?;
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
            if side == Side::Write
                && let Some(blocker) = last_write
                && !blockers.contains(&blocker)
            {
                blockers.push(blocker);
            }
            for blocker in &blockers {
                self.gh.add_blocked_by(number, *blocker).await?;
            }
            if side == Side::Write {
                last_write = Some(number);
            }
            ctx.traces.say(&format!(
                "opened task #{number}: {} ({kind}, {}, {})",
                item.title,
                side.label(),
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
        assert!(gh.writes().is_empty());
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
