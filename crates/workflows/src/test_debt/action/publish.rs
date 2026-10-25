//! The one thing a test-debt attempt writes on GitHub: the label it was
//! asked by.
//!
//! **Removed before the session is paid for, not after it succeeds** — the
//! reasoning of `pr_fix`'s own `ConsumeRequest`: a file whose mutants no
//! session can catch would otherwise buy one session every poll, forever.
//! One label is one attempt. The next rotation that judges the file and
//! still finds mutants poses the label again (`scripts/test-debt.sh`), and a
//! human may pose it by hand.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

use crate::common::labels;
use crate::test_debt::data::state::DebtState;

/// Removes `harness:test-debt` from the issue: the request has been taken.
pub struct ConsumeRequest {
    /// What removes the label.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<DebtState> for ConsumeRequest {
    async fn run(&self, ctx: &mut Context<DebtState>) -> Outcome<Verdict> {
        // A dry run writes nothing; an issue naming no file asks nothing,
        // and keeping its label leaves it visible to whoever fixes the title.
        if ctx.settings.dry_run || ctx.state.file.is_none() {
            return Ok(Verdict::Continue);
        }
        let number = ctx.state.issue().number;
        self.gh.remove_label(number, labels::TEST_DEBT).await?;
        ctx.traces.say(&format!(
            "#{number}: {} consumed — one attempt follows",
            labels::TEST_DEBT
        ));
        Ok(Verdict::Continue)
    }
}

/// Labels the PR the session opened `harness:debt-pr`.
///
/// Found by the
/// attempt's branch, the router lands what carries it
/// (`test_debt::data::landing`). The workflow poses it, never the session —
/// a session touches no `harness:*` label. No PR on the branch (nothing was
/// missed any more, or the session stopped short) is said, not a failure.
pub struct LabelThePr {
    /// What reads the PR and labels it.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<DebtState> for LabelThePr {
    async fn run(&self, ctx: &mut Context<DebtState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || ctx.state.file.is_none() || ctx.state.head.is_empty() {
            return Ok(Verdict::Continue);
        }
        let head = ctx.state.head.clone();
        let pr = match self.gh.pr(&head).await {
            Ok(pr) if pr.is_open() && pr.head == head => pr,
            _ => {
                ctx.traces.say(&format!(
                    "{}: no open PR on {head} — nothing to land",
                    ctx.state.issue().reference()
                ));
                return Ok(Verdict::Continue);
            }
        };
        let Ok(number) = pr.num.parse::<u64>() else {
            return Ok(Verdict::Continue);
        };
        // A board made before the label existed gets it here: `init-repo`
        // installs it on the next upgrade, the PR is labelled now.
        if !self
            .gh
            .labels()
            .await?
            .iter()
            .any(|l| l == labels::TEST_DEBT_PR)
        {
            let label = labels::ALL.iter().find(|l| l.name == labels::TEST_DEBT_PR);
            self.gh
                .create_label(
                    labels::TEST_DEBT_PR,
                    label.map_or("5319e7", |l| l.color),
                    label.map_or("", |l| l.description),
                )
                .await?;
        }
        self.gh.add_label(number, labels::TEST_DEBT_PR).await?;
        ctx.traces.say(&format!(
            "{}: PR {} labelled {} — the harness merges it once green",
            ctx.state.issue().reference(),
            pr.reference(),
            labels::TEST_DEBT_PR
        ));
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool, file: Option<&str>) -> Context<DebtState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            DebtState {
                issue: Some(Issue {
                    number: 7,
                    ..Issue::default()
                }),
                file: file.map(str::to_string),
                findings: String::new(),
                head: String::new(),
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn a_named_file_consumes_the_label_before_anything_is_paid() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, Some("src/lib.rs"));
        ConsumeRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("consumed");
        assert_eq!(
            gh.writes(),
            [Wrote::Unlabelled(7, labels::TEST_DEBT.to_string())]
        );
    }

    #[tokio::test]
    async fn a_dry_run_or_an_issue_naming_nothing_keeps_its_label() {
        for (dry_run, file) in [(true, Some("src/lib.rs")), (false, None)] {
            let gh = Rc::new(FakeGitHub::default());
            let mut context = ctx(dry_run, file);
            ConsumeRequest {
                gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            }
            .run(&mut context)
            .await
            .expect("nothing to do");
            assert!(gh.writes().is_empty(), "dry_run={dry_run} file={file:?}");
        }
    }

    #[tokio::test]
    async fn the_pr_on_the_attempt_s_branch_is_labelled_and_none_is_said() {
        use harness_core::domain::Pr;
        let gh = Rc::new(FakeGitHub {
            prs: vec![(
                "test/debt-7-2".to_string(),
                Pr {
                    num: "40".to_string(),
                    head: "test/debt-7-2".to_string(),
                    state: "OPEN".to_string(),
                    ..Pr::default()
                },
            )],
            ..FakeGitHub::default()
        });
        let mut context = ctx(false, Some("crates/a/src/lib.rs"));
        context.state.head = "test/debt-7-2".to_string();
        LabelThePr {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("labelled");
        let writes = gh.writes();
        assert!(
            matches!(&writes[0], Wrote::CreatedLabel(name, ..) if name == labels::TEST_DEBT_PR),
            "a board without the label gets it: {writes:?}"
        );
        assert_eq!(
            writes.last(),
            Some(&Wrote::Label(40, labels::TEST_DEBT_PR.to_string()))
        );
        let none = Rc::new(FakeGitHub::default());
        LabelThePr {
            gh: Rc::clone(&none) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("said");
        assert_eq!(none.writes(), [] as [Wrote; 0]);
    }
}
