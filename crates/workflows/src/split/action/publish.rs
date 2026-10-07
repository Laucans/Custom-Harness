//! The deterministic write: the milestone's own branch (created the first
//! time it's needed), the tasks opened from the parsed slice plan, and the
//! two labels that mark the milestone as split.
//!
//! The session decided *what* the slices are — this is the only place that
//! decides *how* they land on GitHub: created, linked to the milestone,
//! chained by `blocked_by`, each body carrying a parsable `branch:` line.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

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
        let mut previous = ctx.state.existing.last().map(|issue| issue.number);
        for item in &items {
            let label = if item.needs_human {
                labels::HUMAN
            } else {
                labels::AGENT
            };
            // Under its own heading, and `sections::SCOPE` is the one section
            // no refinement phase rewrites: the boundary this slice states
            // survives every later round instead of being replaced by the five
            // sections a refinement knows.
            let body = format!(
                "## {}\n\nbranch: {}\n\n{}",
                sections::heading_of(sections::SCOPE),
                item.branch,
                item.brief
            );
            let number = self.gh.create_issue(&item.title, &body, &[label]).await?;
            self.gh
                .create_sub_issue_link(milestone_number, number)
                .await?;
            if let Some(blocker) = previous {
                self.gh.add_blocked_by(number, blocker).await?;
            }
            ctx.traces
                .say(&format!("opened task #{number}: {} ({label})", item.title));
            previous = Some(number);
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
    async fn each_task_is_created_linked_chained_and_the_milestone_is_marked() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(
            false,
            r#"[{"title":"A","brief":"do A","branch":"feat/a","needs_human":false},
               {"title":"B","brief":"do B","branch":"feat/b","needs_human":true}]"#,
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
                    "## Scope\n\nbranch: feat/a\n\ndo A".to_string(),
                    vec![labels::AGENT.to_string()]
                ),
                Wrote::SubIssueLink(4, 1),
                Wrote::CreatedIssue(
                    "B".to_string(),
                    "## Scope\n\nbranch: feat/b\n\ndo B".to_string(),
                    vec![labels::HUMAN.to_string()]
                ),
                Wrote::SubIssueLink(4, 2),
                Wrote::BlockedByLink(2, 1),
                Wrote::Unlabelled(4, labels::READY.to_string()),
                Wrote::Label(4, labels::TRIGGERED.to_string()),
            ]
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
        let mut context = ctx(false, r#"[{"title":"A","brief":"do A","branch":"feat/a"}]"#);
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
    async fn the_first_new_task_chains_onto_the_last_existing_one() {
        let gh = Rc::new(gh_with_base());
        let mut context = ctx(false, r#"[{"title":"A","brief":"do A","branch":"feat/a"}]"#);
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
