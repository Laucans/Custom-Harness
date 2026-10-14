//! The whole rules' task: what it establishes before paying, and the lock it
//! holds.
//!
//! Precheck reads the issue. One closed since the router saw it, one no
//! longer labelled `harness:rules`, or one whose `harness:ready` another
//! attempt already took, is **not** a refusal: the clean "nothing to do"
//! stop (`Ok(Some(message))`), the lock never touched. Only an issue that
//! cannot be read at all fails.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::common::labels;
use crate::rules::data::request::Ask;
use crate::rules::data::state::RulesState;

/// A whole rules' task: precheck, lock, the sequence, a summary.
pub struct RulesRun {
    /// The tooling this workflow demands.
    pub pre: Gate<RulesState>,
    /// How many rounds are left. One run is one attempt.
    pub remaining: Cell<u32>,
    /// What reads the issue.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The issues' lock directory.
    pub rules_dir: PathBuf,
    /// The rules' issue number.
    pub issue: u64,
    /// The issue's number as the lock names it.
    pub lock_name: String,
    /// The round, already mounted: the table's steps.
    pub round: Round<RulesState>,
}

#[async_trait(?Send)]
impl Workflow<RulesState> for RulesRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<RulesState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.issue).await?;
        let why = if !issue.is_open() {
            Some("is closed — the rules are settled")
        } else if !issue.has(labels::RULES) {
            Some("is no rules' task")
        } else if !issue.has(labels::READY) {
            Some("is not ready — nobody asks for an attempt now")
        } else {
            None
        };
        if let Some(why) = why {
            return Ok(Some(format!("{} {why}", issue.reference())));
        }
        ctx.traces
            .say(&format!("rules {}  {}", issue.reference(), issue.title));
        ctx.state.issue = Some(issue);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.rules_dir,
            name: &self.lock_name,
        })
    }

    fn held(&self, _ctx: &Context<RulesState>) -> String {
        format!(
            "skip — an attempt at the rules of issue #{} is already running",
            self.issue
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<RulesState> + '_> {
        // Borrowed, not rebuilt: there is only ever one round per run.
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<RulesState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing pushed".to_string());
        }
        // Deliberately not "merged": the judge merges or hands the issue to
        // a human, and the board says which.
        Some(match &ctx.state.ask {
            Some(Ask::Write(rules)) => format!(
                "one attempt at the tests of {} rule(s) (issue #{})",
                rules.ids.len(),
                self.issue
            ),
            Some(Ask::Contest { .. }) => {
                format!(
                    "one attempt at settling the contest (issue #{})",
                    self.issue
                )
            }
            None => format!(
                "issue #{} could not be read — handed to a human",
                self.issue
            ),
        })
    }
}

#[async_trait(?Send)]
impl Executable<RulesState> for RulesRun {
    fn pre(&self) -> Option<&Gate<RulesState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<RulesState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    //! The precheck outcomes, against a fake GitHub.

    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::rules::config::fake as config_fake;
    use crate::rules::data::request;
    use crate::rules::ports::fake as ports_fake;
    use crate::rules::run;
    use harness_core::domain::{Halt, Issue};
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("harness-rules-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn built(gh: &Rc<FakeGitHub>, rules_dir: PathBuf) -> RulesRun {
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config_fake::in_dir(rules_dir),
            &run::Request { issue: 7 },
            Gate::empty("tooling"),
        )
    }

    fn ctx(dry_run: bool) -> Context<RulesState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            RulesState::default(),
            Logbook::null(),
        )
    }

    fn issue(state: &str, labels: &[&str]) -> Issue {
        let rules = serde_json::json!({ "version": 1, "rules": [{
            "id": "R1", "title": "t", "family": "business",
            "criterion": { "given": "g", "when": "w", "then": "t" }
        }] });
        Issue {
            number: 7,
            title: request::FIRST_TITLE.to_string(),
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: request::body("First.", &rules).expect("body"),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_closed_unlabelled_or_unready_issue_is_a_clean_skip() {
        for (state, labelled) in [
            ("closed", &[labels::RULES, labels::READY][..]),
            ("open", &[labels::READY][..]),
            ("open", &[labels::RULES][..]),
        ] {
            let gh = Rc::new(FakeGitHub {
                issues: vec![issue(state, labelled)],
                ..FakeGitHub::default()
            });
            let run = built(&gh, dir(&format!("skip-{state}-{}", labelled.len())));
            let mut context = ctx(false);
            let verdict = Guarded::execute(&run, &mut context)
                .await
                .expect("a clean skip, not an error");
            assert_eq!(verdict, Verdict::Continue, "{state} {labelled:?}");
            assert!(context.state.issue.is_none(), "nothing picked");
            assert!(gh.writes().is_empty(), "no label touched");
        }
    }

    #[tokio::test]
    async fn an_unreadable_issue_fails() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Failed("no credentials".to_string())),
            ..FakeGitHub::default()
        });
        let run = built(&gh, dir("unreadable"));
        let err = Workflow::execute(&run, &mut ctx(false))
            .await
            .expect_err("must fail");
        assert!(err.reason().contains("no credentials"));
    }

    #[tokio::test]
    async fn a_dry_run_reads_the_rules_and_writes_nothing() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue("open", &[labels::RULES, labels::READY])],
            ..FakeGitHub::default()
        });
        let run = built(&gh, dir("dry"));
        let mut context = ctx(true);
        Workflow::execute(&run, &mut context)
            .await
            .expect("a rehearsal");
        assert!(matches!(&context.state.ask, Some(Ask::Write(r)) if r.ids == ["R1"]));
        assert!(gh.writes().is_empty(), "a dry run keeps the request");
        assert_eq!(
            Workflow::summary(&run, &context).as_deref(),
            Some("dry run — nothing pushed")
        );
    }

    #[test]
    fn one_attempt_per_issue_holds_its_lock_and_its_tooling_gate() {
        let gh = Rc::new(FakeGitHub::default());
        let rules_dir = dir("lock");
        let run = built(&gh, rules_dir.clone());
        let lock = Workflow::lock(&run).expect("an attempt is locked per issue");
        assert_eq!(lock.dir, rules_dir.as_path());
        assert_eq!(lock.name, "7");
        let held = Workflow::held(&run, &ctx(false));
        assert!(held.starts_with("skip") && held.contains("#7"), "{held}");
        assert_eq!(Executable::pre(&run).expect("mounted").name, "tooling");
        assert!(std::ptr::eq(
            Workflow::remaining(&run),
            std::ptr::from_ref(&run.remaining)
        ));
        assert_eq!(run.remaining.get(), 1);
    }

    #[test]
    fn the_summary_never_claims_the_rules_landed() {
        let gh = Rc::new(FakeGitHub::default());
        let run = built(&gh, dir("summary"));
        let mut context = ctx(false);
        let said =
            |context: &Context<RulesState>| Workflow::summary(&run, context).unwrap_or_default();
        assert!(said(&context).contains("handed to a human"));
        context.state.ask = Some(Ask::Contest {
            said: String::new(),
        });
        assert!(said(&context).contains("contest"));
        let Ok(Ask::Write(rules)) = request::read(request::FIRST_TITLE, &issue("open", &[]).body)
        else {
            panic!("rules");
        };
        context.state.ask = Some(Ask::Write(rules));
        let summary = said(&context);
        assert!(
            summary.contains("1 rule(s)") && !summary.contains("merged"),
            "{summary}"
        );
    }
}
