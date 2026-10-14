//! What the rules' task reads for free, and what it asks of its two paid
//! sessions.
//!
//! The templates are received, never fetched: they live with the table that
//! sends them (`orchestration::stages`).

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::rules::data::request::{self, Ask};
use crate::rules::data::state::RulesState;

/// What stands in for the rules when an issue carries none: a contest.
const NO_RULES: &str = "(a contest carries no rules: judge against the pull request's own rules.json and its decision)";

/// Reads, for free, what the paid stages need: what the issue asks, and
/// what its comments say.
pub struct ReadRequest {
    /// What reads the issue's comments.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<RulesState> for ReadRequest {
    async fn run(&self, ctx: &mut Context<RulesState>) -> Outcome<Verdict> {
        let issue = ctx.state.issue().clone();
        match request::read(&issue.title, &issue.body) {
            Ok(ask) => {
                let what = match &ask {
                    Ask::Write(rules) => format!(
                        "{} rule(s), {} to conform",
                        rules.ids.len(),
                        rules.conform.len()
                    ),
                    Ask::Contest { .. } => "a contest".to_string(),
                };
                ctx.traces
                    .say(&format!("{}: the rules' task — {what}", issue.reference()));
                ctx.state.ask = Some(ask);
            }
            Err(why) => {
                ctx.traces
                    .say(&format!("{}: cannot be read — {why}", issue.reference()));
                ctx.state.unreadable = Some(why);
                return Ok(Verdict::Continue);
            }
        }
        // A review a former attempt left, the human's answer: the session
        // starts from them.
        ctx.state.comments = self.gh.issue_comments(issue.number).await?.join("\n---\n");
        Ok(Verdict::Continue)
    }
}

/// The values every template of the task is spliced with.
fn values(state: &RulesState, branch: &str) -> Vec<(&'static str, String)> {
    let (rules, conform, said) = match &state.ask {
        Some(Ask::Write(rules)) => (
            rules.json.clone(),
            if rules.conform.is_empty() {
                "none".to_string()
            } else {
                rules.conform.join(", ")
            },
            String::new(),
        ),
        Some(Ask::Contest { said }) => (NO_RULES.to_string(), "none".to_string(), said.clone()),
        None => (NO_RULES.to_string(), "none".to_string(), String::new()),
    };
    let comments = if state.comments.trim().is_empty() {
        "(none)".to_string()
    } else {
        state.comments.clone()
    };
    vec![
        ("num", state.issue().number.to_string()),
        ("branch", branch.to_string()),
        ("rules", rules),
        ("conform", conform),
        ("said", said),
        ("comments", comments),
    ]
}

/// Sends the prompt that writes the rules and their tests — or settles a
/// contest — and opens their pull request.
pub struct AskToWrite {
    /// The stage name, for the journal and the registry.
    pub stage: String,
    /// The template for the rules to write.
    pub write: String,
    /// The template for a contest to settle.
    pub contest: String,
    /// The integration branch the pull request targets.
    pub branch: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<RulesState> for AskToWrite {
    async fn run(&self, open: &mut Open<'_, RulesState>) -> Outcome<Verdict> {
        let template = match open.state.ask {
            Some(Ask::Contest { .. }) => &self.contest,
            _ => &self.write,
        };
        let owned = values(&open.state, &self.branch);
        let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let prompt = splice(template, &pairs);
        let task = open.state.issue().reference();
        ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

/// Sends the prompt that judges each test against its criterion, in a
/// session of its own, and merges when every test is faithful.
pub struct AskToJudge {
    /// The stage name, for the journal and the registry.
    pub stage: String,
    /// The template, received from the table.
    pub template: String,
    /// The integration branch the pull request targets.
    pub branch: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<RulesState> for AskToJudge {
    async fn run(&self, open: &mut Open<'_, RulesState>) -> Outcome<Verdict> {
        let owned = values(&open.state, &self.branch);
        let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let prompt = splice(&self.template, &pairs);
        let task = open.state.issue().reference();
        ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use serde_json::json;

    fn ctx(title: &str, body: &str) -> Context<RulesState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            RulesState {
                issue: Some(Issue {
                    number: 4,
                    title: title.to_string(),
                    body: body.to_string(),
                    state: "open".to_string(),
                    ..Issue::default()
                }),
                ..RulesState::default()
            },
            Logbook::null(),
        )
    }

    fn rules_body() -> String {
        let rules = json!({ "version": 1, "rules": [{
            "id": "R1", "title": "t", "family": "journey", "state": "conform",
            "criterion": { "given": "g", "when": "w", "then": "t" }
        }] });
        request::body("First.", &rules).expect("body")
    }

    #[tokio::test]
    async fn the_rules_and_the_comments_land_in_the_state() {
        let gh = Rc::new(FakeGitHub {
            issue_comments: vec!["R1's test asserts nothing".to_string()],
            ..FakeGitHub::default()
        });
        let mut context = ctx(request::FIRST_TITLE, &rules_body());
        ReadRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("read");
        assert!(matches!(&context.state.ask, Some(Ask::Write(r)) if r.conform == ["R1"]));
        assert!(context.state.comments.contains("asserts nothing"));
        let spliced = values(&context.state, "main_agent");
        let get = |k: &str| {
            spliced
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("conform").as_deref(), Some("R1"));
        assert_eq!(get("num").as_deref(), Some("4"));
        assert!(get("rules").unwrap_or_default().contains("\"R1\""));
    }

    #[tokio::test]
    async fn an_unreadable_request_reads_nothing_further() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(request::FIRST_TITLE, "no block");
        ReadRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("read");
        assert!(context.state.ask.is_none());
        assert!(context.state.unreadable.is_some());
    }

    #[tokio::test]
    async fn a_contest_carries_what_the_development_said_and_no_rules() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx("Rules contested: PR #9", "R1 cannot hold");
        ReadRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("read");
        let spliced = values(&context.state, "main_agent");
        let get = |k: &str| {
            spliced
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("said").as_deref(), Some("R1 cannot hold"));
        assert_eq!(get("rules").as_deref(), Some(NO_RULES));
        assert_eq!(get("comments").as_deref(), Some("(none)"));
    }
}
