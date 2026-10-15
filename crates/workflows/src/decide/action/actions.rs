//! What a decision reads for free, and what it asks of its one session.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::common::decision::Question;
use crate::common::{labels, sections};
use crate::decide::data::decision::{self, kind_of};
use crate::decide::data::state::DecideState;
use crate::dev_loop::data::tasks;

/// How much of a thread a prompt reads, its latest end: past it, the oldest
/// words go first.
pub const THREAD_BUDGET: usize = 6_000;

/// The latest `budget` characters of a text, cut on a character.
fn latest(text: &str, budget: usize) -> String {
    let count = text.chars().count();
    if count <= budget {
        return text.to_string();
    }
    let kept: String = text.chars().skip(count - budget).collect();
    format!("(earlier words left out)\n{kept}")
}

/// Reads, for free, what the session decides from: the question, the
/// comments, and the task's PR with its review when the task waits on it.
pub struct ReadCase {
    /// What reads the issue's comments and its PR.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<DecideState> for ReadCase {
    async fn run(&self, ctx: &mut Context<DecideState>) -> Outcome<Verdict> {
        let issue = ctx.state.issue().clone();
        let question = sections::parse(&issue.body)
            .get(sections::DECISION)
            .and_then(|text| Question::parse(text));
        ctx.state.owner_decided = question.as_ref().is_some_and(Question::decided_by_owner);
        let review_pending = tasks::review_pending(&issue);
        let rules = issue.has(labels::RULES);
        ctx.state.kind = kind_of(question.as_ref(), review_pending, rules);
        ctx.state.question = question;
        ctx.state.comments = latest(
            &self.gh.issue_comments(issue.number).await?.join("\n---\n"),
            THREAD_BUDGET,
        );
        // The PR the decision may merge or repair: a task's on its declared
        // branch, the rules' task's on `rules/<n>`.
        let branch = if rules {
            Some(format!("rules/{}", issue.number))
        } else if review_pending {
            tasks::declared_branch(&issue.body)
        } else {
            None
        };
        if let Some(branch) = branch
            && let Ok(pr) = self.gh.pr(&branch).await
        {
            ctx.state.pr_comments = latest(&self.gh.pr_comments(&pr.num).await?, THREAD_BUDGET);
            ctx.state.pr = Some(pr);
        }
        ctx.traces.say(&format!(
            "{}: a decision is asked ({}){}{}",
            issue.reference(),
            ctx.state.kind,
            ctx.state
                .pr
                .as_ref()
                .map(|pr| format!(", on PR {}", pr.reference()))
                .unwrap_or_default(),
            if ctx.state.owner_decided {
                " — the owner already answered"
            } else {
                ""
            }
        ));
        Ok(Verdict::Continue)
    }
}

/// Sends the prompt that decides, and asks once more when the answer does
/// not read as a decision — the gate after it judges the second.
pub struct AskDecider {
    /// The stage name, for the journal and the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

/// The values the template is spliced with.
fn values(state: &DecideState) -> Vec<(&'static str, String)> {
    let issue = state.issue();
    let decision = state
        .question
        .as_ref()
        .map_or_else(|| "(none)".to_string(), Question::render);
    let (pr, review, actions) = state.pr.as_ref().map_or_else(
        || {
            (
                "(none)".to_string(),
                "(none)".to_string(),
                "`answer` or `cut`".to_string(),
            )
        },
        |pr| {
            (
                format!("{} ({} -> {})", pr.reference(), pr.head, pr.base),
                state.pr_comments.clone(),
                "`answer`, `merge`, `repair` or `cut`".to_string(),
            )
        },
    );
    vec![
        ("num", issue.number.to_string()),
        ("title", issue.title.clone()),
        ("kind", state.kind.clone()),
        ("body", issue.body.clone()),
        ("decision", decision),
        (
            "comments",
            if state.comments.trim().is_empty() {
                "(none)".to_string()
            } else {
                state.comments.clone()
            },
        ),
        ("pr", pr),
        ("review", review),
        ("actions", actions),
    ]
}

#[async_trait(?Send)]
impl SessionAction<DecideState> for AskDecider {
    async fn run(&self, open: &mut Open<'_, DecideState>) -> Outcome<Verdict> {
        let owned = values(&open.state);
        let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let prompt = splice(&self.template, &pairs);
        let task = open.state.issue().reference();
        let has_pr = open.state.pr.is_some();
        let reply =
            ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        if let Err(why) = decision::parse(&reply.text, has_pr) {
            let retry = format!(
                "Your answer does not read as a decision: {why}. Answer again with ONLY the JSON \
                 object — action, summary, answer, confidence — no prose."
            );
            ask_and_record(open, &retry, &self.stage, 1, &task, self.spending.as_ref()).await?;
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::{Issue, Pr};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(issue: Issue) -> Context<DecideState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            DecideState {
                issue: Some(issue),
                ..DecideState::default()
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn a_task_waiting_on_its_review_is_read_with_its_pr() {
        let gh = Rc::new(FakeGitHub {
            issue_comments: vec!["PR #40 still does not merge".to_string()],
            prs: vec![(
                "feat/pjs".to_string(),
                Pr {
                    num: "40".to_string(),
                    head: "feat/pjs".to_string(),
                    base: "milestone/3-x".to_string(),
                    ..Pr::default()
                },
            )],
            pr_comment_bodies: vec![("40".to_string(), "VERDICT: blocking".to_string())],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Issue {
            number: 15,
            body: "## Scope\n\nbranch: feat/pjs\n\nthe list".to_string(),
            labels: vec![labels::REVIEW_PENDING.to_string()],
            ..Issue::default()
        });
        ReadCase {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("read");
        assert_eq!(context.state.kind, "review");
        assert_eq!(
            context.state.pr.as_ref().map(|p| p.num.as_str()),
            Some("40")
        );
        assert!(context.state.pr_comments.contains("blocking"));
        let spliced = values(&context.state);
        let get = |k: &str| {
            spliced
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
        };
        assert!(get("actions").unwrap_or_default().contains("`merge`"));
        assert_eq!(
            get("pr").as_deref(),
            Some("#40 (feat/pjs -> milestone/3-x)")
        );
    }

    #[tokio::test]
    async fn an_owner_s_answer_is_read_and_a_stop_keeps_its_kind() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(Issue {
            number: 9,
            body: "## Decision\n\nkind: stop\nblocks: which provider\nround: 1\ndecided-by: owner\nverdict: Stripe"
                .to_string(),
            ..Issue::default()
        });
        ReadCase {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("read");
        assert!(context.state.owner_decided);
        assert_eq!(context.state.kind, "stop");
        assert!(context.state.pr.is_none());
        let spliced = values(&context.state);
        let get = |k: &str| {
            spliced
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("actions").as_deref(), Some("`answer` or `cut`"));
        assert!(
            get("decision")
                .unwrap_or_default()
                .contains("which provider")
        );
    }

    #[tokio::test]
    async fn a_rules_task_is_read_with_its_pull_request() {
        let gh = Rc::new(FakeGitHub {
            prs: vec![(
                "rules/6".to_string(),
                Pr {
                    num: "50".to_string(),
                    head: "rules/6".to_string(),
                    base: "main_agent".to_string(),
                    ..Pr::default()
                },
            )],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Issue {
            number: 6,
            labels: vec![labels::RULES.to_string()],
            ..Issue::default()
        });
        ReadCase {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("read");
        assert_eq!(context.state.kind, "rules");
        assert_eq!(
            context.state.pr.as_ref().map(|p| p.num.as_str()),
            Some("50")
        );
    }

    #[test]
    fn a_long_thread_keeps_its_latest_words() {
        let long = "a".repeat(10) + &"b".repeat(THREAD_BUDGET);
        let kept = latest(&long, THREAD_BUDGET);
        assert!(kept.starts_with("(earlier words left out)\n"));
        assert!(!kept.contains("aaaaaaaaaa"), "the oldest words go first");
        assert_eq!(latest("short", THREAD_BUDGET), "short");
    }
}
